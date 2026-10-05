//! Documents: the text with its line index, the format of its file and the state of the file
//! on disk.

mod load;

use std::fmt;
use std::fs::Metadata;
use std::ops::Range;
use std::path::PathBuf;
use std::time::{Instant, SystemTime};

pub use load::{LARGE_FILE, Loader, OpenAs, OpenError, Opened, PREVIEW_LEN, open};

use crate::encoding::{Encoding, Losses};
use crate::history::{Edit, EditKind, History, Selection, Transaction};
use crate::line_ending::LineEnding;
use crate::text::{GapBuffer, LineIndex, MAX_LEN, TextStore};

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
    decode_losses: Losses,
    history: History,
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
            decode_losses: Losses::default(),
            history: History::new(),
        }
    }

    pub fn text(&self) -> &Text {
        &self.text
    }

    pub fn lines(&self) -> &LineIndex {
        &self.lines
    }

    /// Whether the file is large enough to turn off the features that would slow it down.
    pub fn is_large(&self) -> bool {
        self.large
    }

    /// Bytes of the file that could not be decoded when it was loaded: they became U+FFFD,
    /// so saving would not restore them.
    pub fn decode_losses(&self) -> Losses {
        self.decode_losses
    }

    /// Applies `edits` as one undo step: each replaces a range of the text that the previous ones
    /// left. `before` and `after` are the selections that undo and redo restore. Nothing is applied
    /// if the text would grow beyond [`MAX_LEN`] on the way.
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
    ) -> Result<(), TooLong> {
        let mut len = self.text.len();
        for (range, text) in edits {
            len = len.saturating_sub(range.len()) + text.len();
            if len > MAX_LEN {
                return Err(TooLong);
            }
        }
        if edits.is_empty() {
            return Ok(());
        }
        let edits = edits
            .iter()
            .map(|(range, text)| {
                let deleted = self.text.to_vec(range.clone());
                apply(&mut self.text, &mut self.lines, range.clone(), text);
                Edit { pos: range.start, deleted, inserted: text.to_vec() }
            })
            .collect();
        self.history.record(Transaction { edits, before, after }, kind, now);
        Ok(())
    }

    /// Reverts the last undo step; returns the selection it started with.
    pub fn undo(&mut self) -> Option<Selection> {
        let transaction = self.history.undo()?;
        for edit in transaction.edits.iter().rev() {
            apply(&mut self.text, &mut self.lines, edit.pos..edit.pos + edit.inserted.len(), &edit.deleted);
        }
        Some(transaction.before)
    }

    /// Applies the last undone step again; returns the selection it ended with.
    pub fn redo(&mut self) -> Option<Selection> {
        let transaction = self.history.redo()?;
        for edit in &transaction.edits {
            apply(&mut self.text, &mut self.lines, edit.pos..edit.pos + edit.deleted.len(), &edit.inserted);
        }
        Some(transaction.after)
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
    /// Device and inode on Unix. None on Windows for now: its file index is not in stable Rust.
    pub id: Option<(u64, u64)>,
}

impl Fingerprint {
    pub fn of(metadata: &Metadata) -> Self {
        #[cfg(unix)]
        let id = {
            use std::os::unix::fs::MetadataExt;
            Some((metadata.dev(), metadata.ino()))
        };
        #[cfg(not(unix))]
        let id = None;
        Fingerprint { len: metadata.len(), modified: metadata.modified().ok(), id }
    }
}

/// An edit would make the text longer than [`MAX_LEN`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TooLong;

impl fmt::Display for TooLong {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the text would be longer than {MAX_LEN} bytes")
    }
}

impl std::error::Error for TooLong {}

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
    fn new_documents_are_utf8_with_lf() {
        let doc = Document::new();
        assert_eq!(doc.format, Format { encoding: Encoding::UTF_8, bom: false, line_ending: LineEnding::Lf });
        assert_eq!((doc.path.as_ref(), doc.disk, doc.is_large()), (None, None, false));
    }
}
