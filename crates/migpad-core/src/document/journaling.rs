//! The journal of a document as it is edited: when it is created, what it records, how it is
//! rewritten, and how a document is recovered from it.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::journal::{BaseRef, Journal, ReadError, Record, ReplayError, read};
use super::{Document, DocumentId, Fingerprint, Format, OpenError};
use crate::encoding::{Encoding, Losses};
use crate::text::TextStore;

/// A journal is rewritten when it has grown to twice its size after the last rewrite plus this:
/// rewriting then costs a constant share of what was written.
const REWRITE_SLACK: u64 = 4 << 20;

/// The journal of a document while it is edited.
#[derive(Debug)]
pub(super) struct JournalState {
    /// Where the journal is kept; `None` turns journaling off.
    dir: Option<PathBuf>,
    /// Where the journal was kept before an error stopped it: it starts there again once the text
    /// is saved, or to keep the changes when the program quits.
    stopped_dir: Option<PathBuf>,
    file: Option<Journal>,
    /// Records were written since the last `sync`.
    unsynced: bool,
    /// The text was just saved: the next edit starts with a snapshot of it.
    snapshot_due: bool,
    /// The size of the journal after it was last written anew.
    rewritten_size: u64,
    rewrite_slack: u64,
    error: Option<io::Error>,
}

impl Default for JournalState {
    fn default() -> Self {
        JournalState {
            dir: None,
            stopped_dir: None,
            file: None,
            unsynced: false,
            snapshot_due: false,
            rewritten_size: 0,
            rewrite_slack: REWRITE_SLACK,
            error: None,
        }
    }
}

impl Document {
    pub fn id(&self) -> DocumentId {
        self.id
    }

    /// Keeps the journal of the document in `dir`, from the next edit on.
    pub fn journal_in(&mut self, dir: PathBuf) {
        self.journal.dir = Some(dir);
    }

    /// The journal file, once an edit has created it.
    pub fn journal_path(&self) -> Option<PathBuf> {
        self.journal.file.as_ref()?;
        Some(journal_file(self.journal.dir.as_deref()?, self.id))
    }

    /// Whether the text differs from its file: there is something to save. An untitled document
    /// has something to save once it has text.
    pub fn is_modified(&self) -> bool {
        match self.path {
            None => !self.text.is_empty(),
            Some(_) => self.saved_at != Some(self.history.state()),
        }
    }

    /// Flushes the journal to the disk if anything was written since the last time: by a timer
    /// a second or two after the last edit, when the app goes to the background, and on quitting.
    pub fn sync_journal(&mut self) {
        let result = match &self.journal.file {
            Some(journal) if self.journal.unsynced => journal.sync(),
            _ => return,
        };
        self.journal.unsynced = false;
        if let Err(error) = result {
            self.journal_failed(error);
        }
    }

    /// The journal file to flush to the disk in the background, if anything was written since the
    /// last time: what [`Document::sync_journal`] does, off the main thread. If the flush fails,
    /// tell [`Document::journal_sync_failed`].
    pub fn journal_to_sync(&mut self) -> Option<fs::File> {
        let journal = self.journal.file.as_ref().filter(|_| self.journal.unsynced)?;
        match journal.handle() {
            Ok(file) => {
                self.journal.unsynced = false;
                Some(file)
            }
            Err(error) => {
                self.journal_failed(error);
                None
            }
        }
    }

    /// The journal could not be flushed to the disk: it stops, as after any error of writing it.
    pub fn journal_sync_failed(&mut self, error: io::Error) {
        if self.journal.file.is_some() {
            self.journal_failed(error);
        }
    }

    /// The error that stopped the journal, if any: edits are no longer protected from a crash.
    pub fn take_journal_error(&mut self) -> Option<io::Error> {
        self.journal.error.take()
    }

    /// Stops the journal and removes its file: the document is closed, and has nothing that its
    /// file has not ([ADR 0020]).
    pub fn remove_journal(&mut self) {
        // Windows does not remove a file that is open.
        self.journal.file = None;
        self.journal.stopped_dir = None;
        if let Some(dir) = self.journal.dir.take() {
            let _ = fs::remove_file(journal_file(&dir, self.id));
        }
    }

    /// Makes sure the changes of the document are in a journal, so that the next start brings
    /// them back: after an error stopped the journal, it is written anew with the text, if it can
    /// be now. Tells whether the changes are kept — a document without changes has nothing to keep.
    pub fn keep_changes(&mut self) -> bool {
        if !self.is_modified() || self.journal.file.is_some() {
            return true;
        }
        if self.journal.dir.is_none() {
            self.journal.dir = self.journal.stopped_dir.take();
        }
        if self.journal.dir.is_none() {
            return false;
        }
        self.rewrite_journal(true);
        self.journal.file.is_some()
    }

    /// Notes that the text was just saved to `path` in `format`, and the file is `disk` now.
    /// The journal is rewritten with the file as its base; the next edit snapshots the text.
    pub fn saved(&mut self, path: PathBuf, format: Format, disk: Fingerprint) {
        self.path = Some(path);
        self.format = format;
        self.disk = Some(disk);
        self.decode_losses = Losses::default();
        // Typing must not join the saved step: the saved state would change under it.
        self.history.seal();
        self.saved_at = Some(self.history.state());
        if self.journal.file.is_some() {
            self.rewrite_journal(false);
        }
        // A journal stopped by an error starts again from the file just saved.
        if self.journal.dir.is_none() {
            self.journal.dir = self.journal.stopped_dir.take();
        }
        self.journal.snapshot_due = !self.large;
    }

    /// Keeps the text over a file that another program has changed: the file is `disk` now, and
    /// the text has changes to save, whatever it had before. The journal starts anew from the
    /// text, so that a recovery does not take the file for changed again — but that of a large
    /// file, which would have to copy it.
    pub fn keep_over_file(&mut self, disk: Option<Fingerprint>) {
        self.disk = disk;
        self.saved_at = None;
        if self.journal.file.is_some() && !self.large {
            self.rewrite_journal(true);
        }
    }

    /// Changes the format without saving, like converting the line endings.
    pub fn set_format(&mut self, format: Format) {
        self.format = format;
        self.journal_write(|journal| journal.write_format(format));
        self.journal_after_change();
    }

    /// Recovers a document from the journal at `journal_path`, and continues that journal.
    /// `open_file` loads the file that a base without a snapshot refers to, in the given encoding.
    pub fn recover(
        journal_path: &Path,
        open_file: impl FnOnce(&Path, Encoding) -> Result<Document, OpenError>,
    ) -> Result<Recovered, RecoverError> {
        let contents = read(journal_path)?;
        let last_base = contents.records.iter().rposition(|r| matches!(r, Record::Base { .. }));
        let Some(Record::Base { path, format, disk, snapshot, .. }) = last_base.map(|i| &contents.records[i]) else {
            return Err(ReplayError::NoBase.into());
        };
        let unsaved_edits = last_base.is_some_and(|i| i + 1 < contents.records.len());
        let dir = journal_path.parent().map(Path::to_path_buf);
        let (mut doc, file_changed) = if snapshot.is_some() {
            let doc = Document::replay(&contents, None)?;
            let file_changed = path.as_deref().is_some_and(|path| fingerprint(path) != *disk);
            (doc, file_changed)
        } else {
            let path = path.as_deref().ok_or(ReplayError::NoFileText)?;
            let mut file = open_file(path, format.encoding)?;
            if file.disk == *disk {
                (Document::replay(&contents, Some(file))?, false)
            } else if unsaved_edits {
                return Ok(Recovered::Conflict { file });
            } else {
                // Nothing was left unsaved: the file as it is now, without the old history.
                let _ = fs::remove_file(journal_path);
                file.id = contents.id;
                file.journal.dir = dir;
                return Ok(Recovered::Restored { doc: file, file_changed: true });
            }
        };
        doc.journal.dir = dir;
        doc.journal.file = Some(Journal::resume(journal_path, contents.valid_len)?);
        // The text is the file, as saved: the next edit starts with a copy of it, as after saving.
        doc.journal.snapshot_due = snapshot.is_none() && !doc.large;
        Ok(Recovered::Restored { doc, file_changed })
    }

    /// Before an edit, undo or redo: creates the journal, or snapshots the text just saved.
    pub(super) fn journal_before_edit(&mut self) {
        if self.journal.dir.is_none() {
            return;
        }
        if self.journal.file.is_none() {
            self.rewrite_journal(!self.large);
            self.journal.snapshot_due = false;
        } else if std::mem::take(&mut self.journal.snapshot_due) {
            let base = BaseRef {
                path: self.path.as_deref(),
                format: self.format,
                disk: self.disk,
                decode_losses: self.decode_losses,
                saved: self.saved_position(),
                snapshot: Some(&self.text),
            };
            let result = self.journal.file.as_mut().expect("checked above").write_base(base);
            self.after_write(result);
        }
    }

    /// Appends a record to the journal, if there is one.
    pub(super) fn journal_write(&mut self, write: impl FnOnce(&mut Journal) -> io::Result<()>) {
        let Some(journal) = &mut self.journal.file else { return };
        let result = write(journal);
        self.after_write(result);
    }

    /// After an edit, undo or redo, once the text and the history agree again: rewrites the
    /// journal if it has grown too large.
    pub(super) fn journal_after_change(&mut self) {
        let Some(journal) = &self.journal.file else { return };
        // Without a snapshot, the edits after a base that is the file cannot be dropped.
        if journal.size() > 2 * self.journal.rewritten_size + self.journal.rewrite_slack
            && !(self.large && self.is_modified())
        {
            self.rewrite_journal(!self.large);
        }
    }

    fn after_write(&mut self, result: io::Result<()>) {
        match result {
            Ok(()) => self.journal.unsynced = true,
            Err(error) => self.journal_failed(error),
        }
    }

    /// Writes the journal anew: the undo history, then a base with the text as it is — a snapshot,
    /// or the file if `snapshot` is false and the text is saved. Replaces the old journal through
    /// a temporary file, so a crash leaves one or the other.
    fn rewrite_journal(&mut self, snapshot: bool) {
        let Some(dir) = self.journal.dir.clone() else { return };
        let snapshot = snapshot || self.is_modified() || self.disk.is_none();
        let path = journal_file(&dir, self.id);
        let temp = path.with_extension("mpj.tmp");
        let write = || -> io::Result<Journal> {
            fs::create_dir_all(&dir)?;
            let mut journal = Journal::create(&temp, self.id, self.path.as_deref())?;
            for (transaction, kind) in self.history.done_steps() {
                journal.write_transaction(transaction, kind, false)?;
            }
            let mut undone = 0;
            for (transaction, kind) in self.history.undone_steps() {
                journal.write_transaction(transaction, kind, false)?;
                undone += 1;
            }
            for _ in 0..undone {
                journal.write_undo()?;
            }
            journal.write_base(BaseRef {
                path: self.path.as_deref(),
                format: self.format,
                disk: self.disk,
                decode_losses: self.decode_losses,
                saved: self.saved_position(),
                snapshot: snapshot.then_some(&self.text),
            })?;
            journal.sync()?;
            Ok(journal)
        };
        let result = write();
        // Windows does not rename over a file that is open.
        self.journal.file = None;
        match result.and_then(|journal| fs::rename(&temp, &path).map(|()| journal)) {
            Ok(journal) => {
                self.journal.rewritten_size = journal.size();
                self.journal.file = Some(journal);
                self.journal.unsynced = false;
            }
            Err(error) => {
                let _ = fs::remove_file(&temp);
                self.journal_failed(error);
            }
        }
    }

    /// The position of the saved state in the undo history, see [`History::position`].
    fn saved_position(&self) -> Option<u64> {
        self.path.as_ref()?;
        self.history.position_of(self.saved_at?)
    }

    /// Stops the journal after an error. A journal that misses records would restore an older
    /// text, so it is removed; the edits stay in memory.
    fn journal_failed(&mut self, error: io::Error) {
        self.journal.file = None;
        if let Some(dir) = self.journal.dir.take() {
            let _ = fs::remove_file(journal_file(&dir, self.id));
            self.journal.stopped_dir = Some(dir);
        }
        self.journal.error = Some(error);
    }
}

/// What [`Document::recover`] found in a journal.
#[derive(Debug)]
pub enum Recovered {
    /// The document as it was before quitting or a crash. `file_changed` tells that its file on
    /// disk is no longer the one the journal knew: show what changed.
    Restored { doc: Document, file_changed: bool },
    /// The journal continues a file that has changed on disk and has no snapshot of it, so its
    /// edits cannot be applied: `file` is the file as it is now. The journal is left for the
    /// caller to set aside.
    Conflict { file: Document },
}

/// Why a document cannot be recovered from its journal.
#[derive(Debug)]
pub enum RecoverError {
    Read(ReadError),
    Replay(ReplayError),
    Open(OpenError),
    Io(io::Error),
}

impl From<ReadError> for RecoverError {
    fn from(error: ReadError) -> Self {
        RecoverError::Read(error)
    }
}

impl From<ReplayError> for RecoverError {
    fn from(error: ReplayError) -> Self {
        RecoverError::Replay(error)
    }
}

impl From<OpenError> for RecoverError {
    fn from(error: OpenError) -> Self {
        RecoverError::Open(error)
    }
}

impl From<io::Error> for RecoverError {
    fn from(error: io::Error) -> Self {
        RecoverError::Io(error)
    }
}

impl fmt::Display for RecoverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RecoverError::Read(error) => error.fmt(f),
            RecoverError::Replay(error) => error.fmt(f),
            RecoverError::Open(error) => error.fmt(f),
            RecoverError::Io(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for RecoverError {}

fn journal_file(dir: &Path, id: DocumentId) -> PathBuf {
    dir.join(format!("{id}.mpj"))
}

fn fingerprint(path: &Path) -> Option<Fingerprint> {
    Fingerprint::of_path(path).ok()
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::document::journal::Record;
    use crate::document::{OpenAs, Opened, open};
    use crate::history::{EditKind, Selection};
    use crate::testing::{TempDir, TempFile};

    fn text(doc: &Document) -> Vec<u8> {
        doc.text().to_vec(0..doc.text().len())
    }

    fn open_file(path: &Path, encoding: Encoding) -> Result<Document, OpenError> {
        match open(path, OpenAs::Encoding(encoding))? {
            Opened::Complete(doc) => Ok(doc),
            Opened::Partial { loader, .. } => loader.load(),
        }
    }

    /// Opens `file` with the journal kept in `dir`.
    fn journaled(file: &TempFile, dir: &TempDir) -> Document {
        let mut doc = open_file(&file.0, Encoding::UTF_8).unwrap();
        doc.journal_in(dir.0.clone());
        doc
    }

    fn insert(doc: &mut Document, pos: usize, text: &str, kind: EditKind) {
        let (before, after) = (Selection::caret(pos), Selection::caret(pos + text.len()));
        doc.edit(&[(pos..pos, text.as_bytes())], before, after, kind, Instant::now()).unwrap();
    }

    /// Types `text` a character at a time, as one undo step.
    fn type_text(doc: &mut Document, pos: usize, text: &str) {
        for (i, c) in text.char_indices() {
            insert(doc, pos + i, c.encode_utf8(&mut [0; 4]), EditKind::Typing);
        }
    }

    fn recovered(doc: &Document) -> Recovered {
        Document::recover(&doc.journal_path().unwrap(), open_file).unwrap()
    }

    fn restored(recovered: Recovered) -> (Document, bool) {
        match recovered {
            Recovered::Restored { doc, file_changed } => (doc, file_changed),
            Recovered::Conflict { .. } => panic!("unexpected conflict"),
        }
    }

    /// Saves the text over its file, as saving will.
    fn save(doc: &mut Document) {
        let path = doc.path.clone().unwrap();
        std::fs::write(&path, text(doc)).unwrap();
        let disk = Fingerprint::of_path(&path).unwrap();
        doc.saved(path, doc.format, disk);
    }

    fn records(doc: &Document) -> Vec<Record> {
        read(&doc.journal_path().unwrap()).unwrap().records
    }

    #[test]
    fn the_journal_appears_on_the_first_edit() {
        let (file, dir) = (TempFile::new("journaling-first", b"hello"), TempDir::new("journaling-first"));
        let mut doc = journaled(&file, &dir);
        assert_eq!((doc.journal_path(), dir.files().len()), (None, 0));
        insert(&mut doc, 5, "!", EditKind::Other);
        assert_eq!(dir.files(), [doc.journal_path().unwrap()]);
        let Record::Base { snapshot, disk, saved, .. } = &records(&doc)[0] else { panic!("a base first") };
        assert_eq!((snapshot.as_deref(), *disk, *saved), (Some(&b"hello"[..]), doc.disk, Some(0)));
    }

    #[test]
    fn large_files_start_from_the_file() {
        let (file, dir) = (TempFile::new("journaling-large", b"big"), TempDir::new("journaling-large"));
        let mut doc = journaled(&file, &dir);
        doc.large = true;
        insert(&mut doc, 3, "!", EditKind::Other);
        assert!(matches!(&records(&doc)[0], Record::Base { snapshot: None, .. }));
    }

    #[test]
    fn recovery_restores_the_text_the_history_and_unsaved_changes() {
        let (file, dir) = (TempFile::new("journaling-recover", b"one\n"), TempDir::new("journaling-recover"));
        let mut doc = journaled(&file, &dir);
        type_text(&mut doc, 4, "ab");
        insert(&mut doc, 6, "\n", EditKind::Other);
        type_text(&mut doc, 7, "c");
        doc.undo();
        assert!(doc.is_modified());
        // A crash: nothing but the writes already done.
        let (mut back, file_changed) = restored(recovered(&doc));
        drop(doc);
        assert!(!file_changed);
        assert_eq!(text(&back), b"one\nab\n");
        assert!(back.is_modified());
        assert_eq!(back.redo(), Some(Selection::caret(8)));
        back.undo();
        back.undo();
        assert_eq!(text(&back), b"one\nab");
        back.undo();
        assert_eq!(text(&back), b"one\n", "typed \"ab\" is one step");
        assert!(!back.is_modified(), "back to the saved text");
        // The recovered journal goes on.
        insert(&mut back, 0, "0", EditKind::Other);
        assert_eq!(text(&restored(recovered(&back)).0), b"0one\n");
    }

    #[test]
    fn saving_rewrites_the_journal_and_keeps_the_history() {
        let (file, dir) = (TempFile::new("journaling-save", b"start"), TempDir::new("journaling-save"));
        let mut doc = journaled(&file, &dir);
        insert(&mut doc, 5, " one", EditKind::Other);
        doc.undo();
        insert(&mut doc, 5, " two", EditKind::Other);
        save(&mut doc);
        assert!(!doc.is_modified());
        let rewritten = records(&doc);
        assert!(matches!(rewritten.last(), Some(Record::Base { snapshot: None, saved: Some(1), .. })));
        assert_eq!(rewritten.len(), 2, "the undo history, then the base");
        // The next edit snapshots the saved text first.
        insert(&mut doc, 0, ">", EditKind::Other);
        let after = records(&doc);
        assert!(matches!(&after[2], Record::Base { snapshot: Some(text), .. } if text == b"start two"));
        let (mut back, _) = restored(recovered(&doc));
        assert_eq!(text(&back), b">start two");
        back.undo();
        assert!(!back.is_modified());
        back.undo();
        assert_eq!(text(&back), b"start");
        assert!(back.is_modified());
    }

    #[test]
    fn a_changed_file_without_a_snapshot_is_a_conflict() {
        let (file, dir) = (TempFile::new("journaling-conflict", b"text"), TempDir::new("journaling-conflict"));
        let mut doc = journaled(&file, &dir);
        doc.large = true;
        insert(&mut doc, 4, "!", EditKind::Other);
        std::fs::write(&file.0, b"changed elsewhere").unwrap();
        match recovered(&doc) {
            Recovered::Conflict { file } => assert_eq!(text(&file), b"changed elsewhere"),
            Recovered::Restored { .. } => panic!("expected a conflict"),
        }
        assert!(doc.journal_path().unwrap().exists(), "the journal is kept to be set aside");
    }

    #[test]
    fn a_snapshot_survives_a_changed_file() {
        let (file, dir) = (TempFile::new("journaling-snapshot", b"text"), TempDir::new("journaling-snapshot"));
        let mut doc = journaled(&file, &dir);
        insert(&mut doc, 4, "!", EditKind::Other);
        std::fs::write(&file.0, b"changed elsewhere").unwrap();
        let (back, file_changed) = restored(recovered(&doc));
        assert!(file_changed);
        assert_eq!(text(&back), b"text!");
    }

    #[test]
    fn a_saved_file_changed_elsewhere_opens_as_it_is() {
        let (file, dir) = (TempFile::new("journaling-reload", b"text"), TempDir::new("journaling-reload"));
        let mut doc = journaled(&file, &dir);
        insert(&mut doc, 4, "!", EditKind::Other);
        save(&mut doc);
        let journal = doc.journal_path().unwrap();
        std::fs::write(&file.0, b"changed elsewhere").unwrap();
        let (back, file_changed) = restored(recovered(&doc));
        assert!(file_changed && !back.can_undo() && !back.is_modified());
        assert_eq!((text(&back), back.id()), (b"changed elsewhere".to_vec(), doc.id()));
        assert!(!journal.exists(), "the old journal no longer applies");
    }

    #[test]
    fn a_text_kept_over_a_changed_file_is_not_taken_for_changed_again() {
        let (file, dir) = (TempFile::new("journaling-keep", b"text"), TempDir::new("journaling-keep"));
        let mut doc = journaled(&file, &dir);
        insert(&mut doc, 4, "!", EditKind::Other);
        std::fs::write(&file.0, b"changed elsewhere").unwrap();
        doc.keep_over_file(Fingerprint::of_path(&file.0).ok());
        assert!(doc.is_modified());
        let (mut back, file_changed) = restored(recovered(&doc));
        assert!(!file_changed);
        assert_eq!(text(&back), b"text!");
        // No state of the history is the file any more.
        back.undo();
        assert!(back.is_modified());
    }

    #[test]
    fn a_saved_text_is_copied_again_after_a_restart() {
        let (file, dir) = (TempFile::new("journaling-restart", b"text"), TempDir::new("journaling-restart"));
        let mut doc = journaled(&file, &dir);
        insert(&mut doc, 4, "!", EditKind::Other);
        save(&mut doc);
        // The next start, then an edit, then a change of the file while MigPad is closed again.
        let (mut back, _) = restored(recovered(&doc));
        drop(doc);
        insert(&mut back, 0, ">", EditKind::Other);
        std::fs::write(&file.0, b"changed elsewhere").unwrap();
        let (again, file_changed) = restored(recovered(&back));
        assert!(file_changed, "the copy of the text keeps the edit, whatever the file is now");
        assert_eq!(text(&again), b">text!");
    }

    #[test]
    fn a_journal_stopped_by_an_error_starts_again() {
        let (file, dir) = (TempFile::new("journaling-again", b"text"), TempDir::new("journaling-again"));
        let mut doc = journaled(&file, &dir);
        insert(&mut doc, 4, "!", EditKind::Other);
        doc.journal_sync_failed(io::Error::other("the disk is full"));
        assert!(doc.take_journal_error().is_some() && dir.files().is_empty());
        // On quitting, the changes are kept in a journal anew.
        assert!(doc.keep_changes());
        assert_eq!(text(&restored(recovered(&doc)).0), b"text!");
        // Once saved, the journal goes on.
        doc.journal_sync_failed(io::Error::other("the disk is full"));
        save(&mut doc);
        insert(&mut doc, 5, "?", EditKind::Other);
        assert_eq!(text(&restored(recovered(&doc)).0), b"text!?");
        // Nowhere to keep them.
        let mut untitled = Document::new();
        insert(&mut untitled, 0, "draft", EditKind::Other);
        assert!(!untitled.keep_changes());
        assert!(Document::new().keep_changes(), "nothing to keep");
    }

    #[test]
    fn journal_errors_keep_the_edits() {
        let file = TempFile::new("journaling-error", b"text");
        let not_a_dir = TempFile::new("journaling-error-dir", b"");
        let mut doc = open_file(&file.0, Encoding::UTF_8).unwrap();
        doc.journal_in(not_a_dir.0.clone());
        insert(&mut doc, 4, "!", EditKind::Other);
        assert_eq!(text(&doc), b"text!");
        assert!(doc.take_journal_error().is_some());
        insert(&mut doc, 5, "?", EditKind::Other);
        assert_eq!((text(&doc), doc.journal_path()), (b"text!?".to_vec(), None));
        assert!(doc.take_journal_error().is_none(), "the journal stopped");
    }

    #[test]
    fn the_journal_is_flushed_in_the_background_once_per_writes() {
        let (file, dir) = (TempFile::new("journaling-flush", b"text"), TempDir::new("journaling-flush"));
        let mut doc = journaled(&file, &dir);
        assert!(doc.journal_to_sync().is_none(), "no journal yet");
        insert(&mut doc, 4, "!", EditKind::Other);
        let handle = doc.journal_to_sync().expect("written since the last flush");
        handle.sync_data().unwrap();
        assert!(doc.journal_to_sync().is_none(), "nothing written since");
        insert(&mut doc, 5, "?", EditKind::Other);
        assert!(doc.journal_to_sync().is_some());
        // A flush that fails stops the journal, as a failed write does.
        doc.journal_sync_failed(io::Error::other("the disk is gone"));
        assert!(doc.take_journal_error().is_some());
        assert!(dir.files().is_empty(), "a journal that may miss records is removed");
        assert_eq!(text(&doc), b"text!?");
    }

    #[test]
    fn a_closed_document_removes_its_journal() {
        let (file, dir) = (TempFile::new("journaling-remove", b"text"), TempDir::new("journaling-remove"));
        let mut doc = journaled(&file, &dir);
        insert(&mut doc, 4, "!", EditKind::Other);
        assert_eq!(dir.files().len(), 1);
        doc.remove_journal();
        assert!(dir.files().is_empty());
        insert(&mut doc, 5, "?", EditKind::Other);
        assert!(dir.files().is_empty(), "no journal after it");
        assert!(doc.take_journal_error().is_none());
    }

    #[test]
    fn a_growing_journal_is_rewritten() {
        let (file, dir) = (TempFile::new("journaling-rewrite", b"x"), TempDir::new("journaling-rewrite"));
        let mut doc = journaled(&file, &dir);
        doc.journal.rewrite_slack = 500;
        // Typing writes a record per character, while the history keeps one step.
        let mut sizes = Vec::new();
        for i in 0..300 {
            type_text(&mut doc, i + 1, "y");
            sizes.push(doc.journal.file.as_ref().unwrap().size());
        }
        assert!(sizes.windows(2).any(|w| w[1] < w[0]), "rewritten at least once");
        // Unrewritten, 300 records would take some 24 000 bytes.
        assert!(*sizes.iter().max().unwrap() < 3000, "kept small: {sizes:?}");
        assert_eq!(dir.files().len(), 1, "no temporary files left");
        doc.sync_journal();
        let (mut back, _) = restored(recovered(&doc));
        assert_eq!(text(&back), text(&doc));
        back.undo();
        assert_eq!(text(&back), b"x", "the typing is still one step");
    }
}
