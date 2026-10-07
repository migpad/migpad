//! Documents: the text with its line index, the format of its file and the state of the file
//! on disk.

pub mod journal;
mod journaling;
mod load;
mod save;

use std::fmt;
use std::fs::File;
use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

pub use journaling::{RecoverError, Recovered};
pub use load::{LARGE_FILE, Loader, OpenAs, OpenError, Opened, PREVIEW_LEN, open};
pub use save::SaveError;

use crate::encoding::{Encoding, Losses};
use crate::history::{Edit, EditKind, History, Selection, Transaction};
use crate::line_ending::{self, LineEnding};
use crate::text::{GapBuffer, LineIndex, MAX_LEN, TextStore};

/// A random identifier of a document; it names the journal file and stays the same when the
/// document is renamed or saved under another name.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct DocumentId([u8; 16]);

impl DocumentId {
    /// A new identifier: random, and unique within the process even when made at the same time.
    pub fn random() -> Self {
        use std::hash::{BuildHasher, Hasher};
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        // `RandomState` is seeded by the system's random number generator.
        let state = std::collections::hash_map::RandomState::new();
        let time = SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        let count = COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut bytes = [0u8; 16];
        for (half, out) in bytes.as_chunks_mut::<8>().0.iter_mut().enumerate() {
            let mut hasher = state.build_hasher();
            hasher.write_u128(time.as_nanos());
            hasher.write_u32(std::process::id());
            hasher.write_u64(count);
            hasher.write_usize(half);
            *out = hasher.finish().to_le_bytes();
        }
        DocumentId(bytes)
    }
}

impl fmt::Display for DocumentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.iter().try_for_each(|byte| write!(f, "{byte:02x}"))
    }
}

impl fmt::Debug for DocumentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl std::str::FromStr for DocumentId {
    type Err = ();

    /// Parses the 32 hex digits that [`Display`](fmt::Display) writes.
    fn from_str(s: &str) -> Result<Self, ()> {
        if s.len() != 32 || !s.is_ascii() {
            return Err(());
        }
        let mut bytes = [0u8; 16];
        for (byte, pair) in bytes.iter_mut().zip(s.as_bytes().as_chunks::<2>().0) {
            *byte = u8::from_str_radix(std::str::from_utf8(pair).map_err(|_| ())?, 16).map_err(|_| ())?;
        }
        Ok(DocumentId(bytes))
    }
}

/// The text store of documents; a store that reads large files in parts may replace it later.
pub type Text = GapBuffer;

/// A text being edited, with or without a file.
#[derive(Debug)]
pub struct Document {
    text: Text,
    lines: LineIndex,
    pub format: Format,
    pub path: Option<PathBuf>,
    /// The file as it was when it was loaded or last saved.
    pub disk: Option<Fingerprint>,
    large: bool,
    /// The beginning of a larger file, shown while the rest loads.
    preview: bool,
    decode_losses: Losses,
    history: History,
    id: DocumentId,
    /// The state of the history the file on disk has, if it can be reached; see [`History::state`].
    saved_at: Option<u64>,
    journal: journaling::JournalState,
    /// See [`Document::version`].
    version: u64,
}

/// A new number for a text, see [`Document::version`].
fn next_version() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

impl Document {
    /// An empty untitled document.
    pub fn new() -> Self {
        Document {
            text: Text::new(),
            lines: LineIndex::new(),
            format: Format::default(),
            path: None,
            disk: None,
            large: false,
            preview: false,
            decode_losses: Losses::default(),
            history: History::new(),
            id: DocumentId::random(),
            saved_at: Some(0),
            journal: Default::default(),
            version: next_version(),
        }
    }

    /// A number for the text as it is now: every edit, undo and redo gives a new one, and no two
    /// documents of the program share one — a document read again from its file has a new number
    /// too. What was found in a text holds while its number does.
    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn text(&self) -> &Text {
        &self.text
    }

    pub fn lines(&self) -> &LineIndex {
        &self.lines
    }

    /// The whole text as one slice, for searching. Moves the gap of the buffer to the end, which
    /// takes time proportional to the text after it; the text does not change.
    pub fn contiguous_text(&mut self) -> &[u8] {
        self.text.make_contiguous()
    }

    /// Whether the file is large enough to turn off the features that would slow it down.
    pub fn is_large(&self) -> bool {
        self.large
    }

    /// Whether this is the beginning of a larger file, shown while the rest loads, see
    /// [`Opened::Partial`]. A preview can be neither edited nor saved: the loaded text replaces it,
    /// and saving it would cut the file short.
    pub fn is_preview(&self) -> bool {
        self.preview
    }

    /// Bytes of the file that could not be decoded when it was loaded: they became U+FFFD,
    /// so saving would not restore them.
    pub fn decode_losses(&self) -> Losses {
        self.decode_losses
    }

    /// Applies `edits` as one undo step: each replaces a range of the text that the previous ones
    /// left. `before` and `after` are the selections that undo and redo restore. Nothing is applied
    /// to a preview, or if the text would grow beyond [`MAX_LEN`] on the way.
    ///
    /// # Panics
    ///
    /// If a range is reversed or out of bounds.
    pub fn edit(
        &mut self,
        edits: &[(Range<usize>, &[u8])],
        before: Selection,
        after: Selection,
        kind: EditKind,
        now: Instant,
    ) -> Result<(), EditError> {
        if self.preview {
            return Err(EditError::Preview);
        }
        let mut len = self.text.len();
        for (range, text) in edits {
            len = len.saturating_sub(range.len()) + text.len();
            if len > MAX_LEN {
                return Err(EditError::TooLong);
            }
        }
        if edits.is_empty() {
            return Ok(());
        }
        self.journal_before_edit();
        let edits = edits
            .iter()
            .map(|(range, text)| {
                let deleted = self.text.to_vec(range.clone());
                apply(&mut self.text, &mut self.lines, range.clone(), text);
                Edit { pos: range.start, deleted, inserted: text.to_vec() }
            })
            .collect();
        let transaction = Transaction { edits, before, after };
        let merged = self.history.can_merge(&transaction, kind, now);
        self.journal_write(|journal| journal.write_transaction(&transaction, kind, merged));
        let recorded = self.history.record(transaction, kind, now);
        debug_assert_eq!(merged, recorded);
        self.version = next_version();
        self.journal_after_change();
        Ok(())
    }

    /// Reverts the last undo step; returns the selection it started with.
    pub fn undo(&mut self) -> Option<Selection> {
        if !self.history.can_undo() {
            return None;
        }
        self.journal_before_edit();
        let transaction = self.history.undo()?;
        for edit in transaction.edits.iter().rev() {
            apply(&mut self.text, &mut self.lines, edit.pos..edit.pos + edit.inserted.len(), &edit.deleted);
        }
        let (before, breaks) = (transaction.before, transaction.edits.iter().any(Edit::breaks_line));
        self.version = next_version();
        if breaks {
            self.follow_line_endings();
        }
        self.journal_write(journal::Journal::write_undo);
        self.journal_after_change();
        Some(before)
    }

    /// Applies the last undone step again; returns the selection it ended with.
    pub fn redo(&mut self) -> Option<Selection> {
        if !self.history.can_redo() {
            return None;
        }
        self.journal_before_edit();
        let transaction = self.history.redo()?;
        for edit in &transaction.edits {
            apply(&mut self.text, &mut self.lines, edit.pos..edit.pos + edit.deleted.len(), &edit.inserted);
        }
        let (after, breaks) = (transaction.after, transaction.edits.iter().any(Edit::breaks_line));
        self.version = next_version();
        if breaks {
            self.follow_line_endings();
        }
        self.journal_write(journal::Journal::write_redo);
        self.journal_after_change();
        Some(after)
    }

    /// After undo or redo of line breaks — of a conversion, say — Enter inserts the line break the
    /// text uses most again, as for a file just read. Not in a large document: that would take a
    /// walk over all of it, and it cannot be converted anyway. The journal needs no record: undo and
    /// redo do the same when it is read.
    fn follow_line_endings(&mut self) {
        if self.large {
            return;
        }
        if let Some(line_ending) = LineEnding::dominant(line_ending::count(&self.text)) {
            self.format.line_ending = line_ending;
        }
    }

    /// Makes every line break of the text `to`, as one undo step from `selection`, and Enter insert
    /// it from now on. Returns where the selection is in the new text — `None` if no line break had
    /// to change, and only Enter changes.
    pub fn convert_line_endings(
        &mut self,
        to: LineEnding,
        selection: Selection,
        now: Instant,
    ) -> Result<Option<Selection>, EditError> {
        if self.preview {
            return Err(EditError::Preview);
        }
        let target = to.as_bytes();
        let points = [selection.anchor, selection.head];
        let mut mapped = points;
        // The text from the first line break to change to the end of the last one, converted.
        let mut span: Option<(usize, usize)> = None;
        let mut converted = Vec::new();
        {
            let text = self.text.make_contiguous();
            let mut at = 0;
            while let Some(found) = memchr::memchr2(b'\r', b'\n', &text[at..]) {
                let pos = at + found;
                let len = if text[pos] == b'\r' && text.get(pos + 1) == Some(&b'\n') { 2 } else { 1 };
                at = pos + len;
                if &text[pos..at] == target {
                    continue;
                }
                let copied = span.map_or(pos, |(_, end)| end);
                if span.is_none() {
                    span = Some((pos, pos));
                }
                converted.extend_from_slice(&text[copied..pos]);
                converted.extend_from_slice(target);
                span = span.map(|(start, _)| (start, at));
                for (point, mapped) in points.iter().zip(&mut mapped) {
                    if *point >= at {
                        *mapped = *mapped + target.len() - len;
                    } else if *point > pos {
                        // Inside a CRLF: before the line break that takes its place.
                        *mapped -= *point - pos;
                    }
                }
            }
        }
        let format = Format { line_ending: to, ..self.format };
        let Some((start, end)) = span else {
            self.set_format(format);
            return Ok(None);
        };
        let after = Selection { anchor: mapped[0], head: mapped[1] };
        self.edit(&[(start..end, &converted)], selection, after, EditKind::Other, now)?;
        self.set_format(format);
        Ok(Some(after))
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    /// Ends the current undo step, so that the next edit does not join it: the caret moved
    /// by itself, or the document lost focus.
    pub fn seal_undo_step(&mut self) {
        self.history.seal();
    }
}

/// Replaces `range` of `text` with `bytes` and updates the line index.
fn apply(text: &mut Text, lines: &mut LineIndex, range: Range<usize>, bytes: &[u8]) {
    text.replace(range.clone(), bytes);
    lines.on_edit(text, range, bytes.len());
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

/// How a document is stored in its file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    pub encoding: Encoding,
    pub bom: bool,
    /// The line break that Enter inserts: the one the file uses most, LF for new files.
    pub line_ending: LineEnding,
}

impl Default for Format {
    fn default() -> Self {
        Format { encoding: Encoding::UTF_8, bom: false, line_ending: LineEnding::Lf }
    }
}

/// The state of a file on disk, to notice when another program changes or replaces it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fingerprint {
    pub len: u64,
    pub modified: Option<SystemTime>,
    /// Device and inode on Unix, volume and file index on Windows.
    pub id: Option<(u64, u64)>,
}

impl Fingerprint {
    pub fn of_file(file: &File) -> io::Result<Self> {
        let metadata = file.metadata()?;
        Ok(Fingerprint { len: metadata.len(), modified: metadata.modified().ok(), id: crate::platform::file_id(file) })
    }

    pub fn of_path(path: &Path) -> io::Result<Self> {
        Self::of_file(&File::open(path)?)
    }
}

/// Why an edit was not applied; the text is as it was.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditError {
    /// The text would be longer than [`MAX_LEN`].
    TooLong,
    /// The document is a preview, see [`Document::is_preview`].
    Preview,
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EditError::TooLong => write!(f, "the text would be longer than {MAX_LEN} bytes"),
            EditError::Preview => f.write_str("the file is still loading"),
        }
    }
}

impl std::error::Error for EditError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(doc: &Document) -> Vec<u8> {
        doc.text().to_vec(0..doc.text().len())
    }

    #[test]
    fn edits_undo_and_redo() {
        let mut doc = Document::new();
        let now = Instant::now();
        doc.edit(&[(0..0, b"one\ntwo\r\nthree")], Selection::caret(0), Selection::caret(14), EditKind::Other, now)
            .unwrap();
        assert_eq!(doc.lines().count(), 3);
        // Two edits in one step: the second one sees the text the first one left.
        let selected = Selection { anchor: 3, head: 9 };
        doc.edit(&[(3..9, b" "), (0..0, b"[")], selected, Selection::caret(5), EditKind::Other, now).unwrap();
        assert_eq!(text(&doc), b"[one three");
        assert_eq!(doc.lines().count(), 1);

        assert_eq!(doc.undo(), Some(selected));
        assert_eq!(text(&doc), b"one\ntwo\r\nthree");
        assert_eq!(doc.lines().count(), 3);
        assert_eq!(doc.undo(), Some(Selection::caret(0)));
        assert_eq!((text(&doc), doc.lines().count()), (Vec::new(), 1));
        assert_eq!(doc.undo(), None);

        assert_eq!(doc.redo(), Some(Selection::caret(14)));
        assert_eq!(doc.redo(), Some(Selection::caret(5)));
        assert_eq!(text(&doc), b"[one three");
        assert_eq!(doc.lines().count(), 1);
        assert!(!doc.can_redo() && doc.can_undo());
    }

    #[test]
    fn typing_is_undone_at_once() {
        let mut doc = Document::new();
        let now = Instant::now();
        for (i, c) in ["a", "b", "c"].iter().enumerate() {
            let before = Selection::caret(i);
            doc.edit(&[(i..i, c.as_bytes())], before, Selection::caret(i + 1), EditKind::Typing, now).unwrap();
        }
        assert_eq!(doc.undo(), Some(Selection::caret(0)));
        assert_eq!(text(&doc), b"");
        assert!(!doc.can_undo());
    }

    #[test]
    fn every_change_of_the_text_has_a_new_version() {
        let mut doc = Document::new();
        let other = Document::new();
        assert_ne!(doc.version(), other.version());
        let now = Instant::now();
        let mut seen = vec![doc.version()];
        doc.edit(&[(0..0, b"ab")], Selection::caret(0), Selection::caret(2), EditKind::Other, now).unwrap();
        seen.push(doc.version());
        doc.undo();
        seen.push(doc.version());
        doc.redo();
        seen.push(doc.version());
        let count = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), count, "{seen:?}");
        // Nothing changed: the same version.
        let version = doc.version();
        doc.seal_undo_step();
        assert_eq!(doc.redo(), None);
        assert_eq!(doc.version(), version);
    }

    fn document(text: &[u8], line_ending: LineEnding) -> Document {
        let mut doc = Document::new();
        doc.edit(&[(0..0, text)], Selection::caret(0), Selection::caret(0), EditKind::Other, Instant::now()).unwrap();
        doc.format.line_ending = line_ending;
        doc
    }

    #[test]
    fn line_breaks_convert_as_one_step_and_undo_brings_them_back() {
        let mut doc = document(b"a\r\nb\r\nc\nd", LineEnding::CrLf);
        let now = Instant::now();
        // The caret before d, and a selection from inside the first CRLF to after b.
        let after = doc.convert_line_endings(LineEnding::Lf, Selection::caret(8), now).unwrap();
        assert_eq!(text(&doc), b"a\nb\nc\nd");
        assert_eq!(after, Some(Selection::caret(6)));
        assert_eq!(doc.format.line_ending, LineEnding::Lf);
        assert_eq!(doc.undo(), Some(Selection::caret(8)));
        assert_eq!(text(&doc), b"a\r\nb\r\nc\nd");
        assert_eq!(doc.format.line_ending, LineEnding::CrLf, "Enter follows the text again");
        assert_eq!(doc.redo(), Some(Selection::caret(6)));
        assert_eq!(doc.format.line_ending, LineEnding::Lf);
        assert!(!doc.can_redo());

        let mut doc = document(b"a\nb\rc", LineEnding::Lf);
        let selection = Selection { anchor: 1, head: 5 };
        let after = doc.convert_line_endings(LineEnding::CrLf, selection, now).unwrap();
        assert_eq!(text(&doc), b"a\r\nb\r\nc");
        assert_eq!(after, Some(Selection { anchor: 1, head: 7 }));
        let inside =
            document(b"x\r\ny", LineEnding::CrLf).convert_line_endings(LineEnding::Cr, Selection::caret(2), now);
        assert_eq!(inside.unwrap(), Some(Selection::caret(1)), "inside a CRLF: before its line break");
    }

    #[test]
    fn converting_to_what_the_text_has_changes_only_enter() {
        let mut doc = document(b"a\nb\n", LineEnding::CrLf);
        let version = doc.version();
        assert_eq!(doc.convert_line_endings(LineEnding::Lf, Selection::caret(1), Instant::now()), Ok(None));
        assert_eq!((text(&doc), doc.format.line_ending), (b"a\nb\n".to_vec(), LineEnding::Lf));
        assert_eq!(doc.version(), version, "the text did not change");
        assert_eq!(
            document(b"", LineEnding::Lf).convert_line_endings(LineEnding::Cr, Selection::caret(0), Instant::now()),
            Ok(None)
        );
    }

    #[test]
    fn new_documents_are_utf8_with_lf() {
        let doc = Document::new();
        assert_eq!(doc.format, Format { encoding: Encoding::UTF_8, bom: false, line_ending: LineEnding::Lf });
        assert_eq!((doc.path.as_ref(), doc.disk, doc.is_large()), (None, None, false));
    }
}
