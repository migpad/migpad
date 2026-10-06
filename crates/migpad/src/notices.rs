//! Notifications over the text of a tab: what went wrong opening or saving a file, bytes that
//! could not be read in its encoding, a journal that stopped, a file changed by another program.

use std::io;
use std::path::Path;

use migpad_core::document::{Document, OpenError, SaveError};
use migpad_core::encoding::Losses;
use migpad_core::text::TextStore;
use migpad_ui::notification::Severity;

use crate::strings::{Key, fill, number, tr};

/// A notification of a tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub severity: Severity,
    pub message: String,
    /// What it is about: a new notification about the same takes the place of the old one.
    pub topic: Option<Topic>,
    /// The buttons for what can be done about it.
    pub actions: Vec<NoticeAction>,
}

impl Notice {
    fn new(severity: Severity, message: String) -> Self {
        Notice { severity, message, topic: None, actions: Vec::new() }
    }
}

/// What a notification is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Topic {
    /// The last attempt to save the document.
    Save,
    /// The journal of the document.
    Journal,
    /// The file on disk, changed by another program.
    Disk,
}

/// What a button of a notification does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeAction {
    /// Saves the document in UTF-8, which holds every character.
    SaveInUtf8,
    /// Selects the first place where something would be lost: a byte offset in the text.
    ShowFirst(usize),
    /// Saves with what cannot be written replaced.
    SaveReplacing,
    /// Saves under another name.
    SaveAs,
    /// Opens the file as it is on disk; the text of the document goes among the closed tabs.
    LoadFromDisk,
    /// Keeps the text of the document over the file changed on disk.
    KeepMine,
}

impl NoticeAction {
    pub fn label(self) -> &'static str {
        tr(match self {
            NoticeAction::SaveInUtf8 => Key::NoticeSaveInUtf8,
            NoticeAction::ShowFirst(_) => Key::NoticeShowFirst,
            NoticeAction::SaveReplacing => Key::NoticeSaveReplacing,
            NoticeAction::SaveAs => Key::NoticeSaveAs,
            NoticeAction::LoadFromDisk => Key::NoticeLoadFromDisk,
            NoticeAction::KeepMine => Key::NoticeKeepMine,
        })
    }
}

/// The file at `path` could not be opened.
pub fn open_failed(path: &Path, error: &OpenError) -> Notice {
    let message = fill(Key::NoticeOpenFailed, &[("file", &file_name(path)), ("reason", &reason(path, error))]);
    Notice::new(Severity::Error, message)
}

/// The file at `path` stopped loading short: its beginning shows, and cannot be edited.
pub fn load_failed(path: &Path, error: &OpenError) -> Notice {
    let message = fill(Key::NoticeLoadFailed, &[("file", &file_name(path)), ("reason", &reason(path, error))]);
    Notice::new(Severity::Error, message)
}

/// `doc` was not saved to `path` in the encoding `encoding`: what was in the way, and what can be
/// done. Nothing to tell for a preview, which cannot be saved at all.
pub fn save_failed(doc: &Document, path: &Path, encoding: &str, error: &SaveError) -> Option<Notice> {
    let file = file_name(path);
    let line_of = |losses: &Losses| {
        let first = losses.first.unwrap_or(0).min(doc.text().len());
        (first, number(doc.lines().line_of(first) as u64 + 1))
    };
    let notice = match error {
        SaveError::Losses { encoding: lost, decoding } if !lost.is_empty() => {
            let (first, line) = line_of(lost);
            let count = number(lost.count as u64);
            let values = [("encoding", encoding), ("count", count.as_str()), ("line", line.as_str()), ("file", &file)];
            Notice {
                severity: Severity::Warning,
                message: fill(Key::NoticeEncodeLosses, &values),
                topic: Some(Topic::Save),
                actions: vec![NoticeAction::SaveInUtf8, NoticeAction::ShowFirst(first), NoticeAction::SaveReplacing],
            }
            .with_decoding(decoding)
        }
        SaveError::Losses { decoding, .. } => {
            let (first, line) = line_of(decoding);
            let count = number(decoding.count as u64);
            let values = [("encoding", encoding), ("count", count.as_str()), ("line", line.as_str()), ("file", &file)];
            Notice {
                severity: Severity::Warning,
                message: fill(Key::NoticeDecodedLosses, &values),
                topic: Some(Topic::Save),
                actions: vec![NoticeAction::ShowFirst(first), NoticeAction::SaveReplacing, NoticeAction::SaveAs],
            }
        }
        SaveError::PermissionDenied(_) => Notice {
            severity: Severity::Error,
            message: fill(Key::NoticeNoWritePermission, &[("file", &file)]),
            topic: Some(Topic::Save),
            actions: vec![NoticeAction::SaveAs],
        },
        SaveError::Io(error) => Notice {
            severity: Severity::Error,
            message: fill(Key::NoticeSaveFailed, &[("file", &file), ("reason", &write_reason(error))]),
            topic: Some(Topic::Save),
            actions: vec![NoticeAction::SaveAs],
        },
        SaveError::Preview => return None,
    };
    Some(notice)
}

impl Notice {
    /// Bytes of the file that were broken when it was read are lost too, whatever is chosen: then
    /// saving in UTF-8 does not keep everything, and is not offered.
    fn with_decoding(mut self, decoding: &Losses) -> Self {
        if !decoding.is_empty() {
            self.actions.retain(|action| *action != NoticeAction::SaveInUtf8);
        }
        self
    }
}

/// The journal of the document `file` stopped after `error`: its changes are no longer kept safe
/// from a crash, though nothing is lost now.
pub fn journal_failed(file: &str, error: &io::Error) -> Notice {
    let message = fill(Key::NoticeJournalFailed, &[("file", file), ("reason", &write_reason(error))]);
    Notice { topic: Some(Topic::Journal), ..Notice::new(Severity::Warning, message) }
}

/// The file `file` was changed by another program while MigPad was closed, and the document has
/// changes of its own: they are shown, and the file on disk is a choice away.
pub fn changed_while_closed(file: &str) -> Notice {
    Notice {
        severity: Severity::Warning,
        message: fill(Key::NoticeChangedWhileClosed, &[("file", file)]),
        topic: Some(Topic::Disk),
        actions: vec![NoticeAction::LoadFromDisk, NoticeAction::KeepMine],
    }
}

/// The large file `file` was changed while MigPad was closed, and its journal has no copy of it
/// for the changes to go on: it opens as it is, and the changes are set aside.
pub fn changes_set_aside(file: &str) -> Notice {
    let message = fill(Key::NoticeChangesSetAside, &[("file", file)]);
    Notice { topic: Some(Topic::Disk), ..Notice::new(Severity::Warning, message) }
}

/// The changes of `file` could not be recovered from its journal, which is set aside.
pub fn recover_failed(file: &str, reason: &str) -> Notice {
    Notice::new(Severity::Error, fill(Key::NoticeRecoverFailed, &[("file", file), ("reason", reason)]))
}

/// Why a file could not be written, told by the kind of the error: the system's own words only
/// for what has no words here.
fn write_reason(error: &io::Error) -> String {
    match error.kind() {
        io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem => tr(Key::NoticeWriteDenied).to_owned(),
        io::ErrorKind::StorageFull => tr(Key::NoticeDiskFull).to_owned(),
        _ => error.to_string(),
    }
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned())
}

/// Why the file at `path` could not be read, told by the kind of the error: the system's own
/// words only for what has no words here. Windows refuses to open a folder as a file without
/// saying it is one.
fn reason(path: &Path, error: &OpenError) -> String {
    match error {
        _ if path.is_dir() => tr(Key::NoticeIsFolder).to_owned(),
        OpenError::Io(error) => match error.kind() {
            io::ErrorKind::PermissionDenied => tr(Key::NoticeNoPermission).to_owned(),
            io::ErrorKind::IsADirectory => tr(Key::NoticeIsFolder).to_owned(),
            _ => error.to_string(),
        },
        OpenError::TooLarge => tr(Key::NoticeTooLarge).to_owned(),
        OpenError::Cancelled => error.to_string(),
    }
}

/// Bytes of the file of `doc` that were not valid in its encoding and became U+FFFD, if any.
pub fn decode_losses(doc: &Document) -> Option<Notice> {
    let losses = doc.decode_losses();
    let first = losses.first.filter(|_| !losses.is_empty())?;
    let line = doc.lines().line_of(first.min(doc.text().len())) as u64 + 1;
    let count = number(losses.count as u64);
    let values = [("encoding", doc.format.encoding.name()), ("count", count.as_str()), ("line", &number(line))];
    let message = fill(Key::NoticeDecodeLosses, &values);
    Some(Notice::new(Severity::Warning, message))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strings::{LANGUAGE_LOCK, Language, set_language};

    #[test]
    fn errors_of_opening_are_told_by_their_kind() {
        let _lock = LANGUAGE_LOCK.lock();
        let path = Path::new("/no such folder/план.txt");
        let denied = OpenError::Io(io::Error::from(io::ErrorKind::PermissionDenied));
        let folder = OpenError::Io(io::Error::from(io::ErrorKind::IsADirectory));
        set_language(Language::Russian);
        assert_eq!(open_failed(path, &denied).message, "Не удалось открыть «план.txt»: нет прав на чтение.");
        assert_eq!(open_failed(path, &folder).message, "Не удалось открыть «план.txt»: это папка.");
        assert_eq!(
            open_failed(path, &OpenError::TooLarge).message,
            "Не удалось открыть «план.txt»: файл больше 4 ГиБ."
        );
        set_language(Language::English);
        assert_eq!(open_failed(path, &denied).message, "Could not open “план.txt”: no permission to read it.");
        assert_eq!(open_failed(path, &OpenError::TooLarge).severity, Severity::Error);
        // A folder, whatever the system said.
        let folder = std::env::temp_dir();
        assert!(open_failed(&folder, &denied).message.contains("it is a folder"));
    }

    #[test]
    fn save_failures_offer_what_can_be_done() {
        let _lock = LANGUAGE_LOCK.lock();
        set_language(Language::English);
        let mut doc = Document::new();
        let now = std::time::Instant::now();
        let (before, after) = (migpad_core::history::Selection::caret(0), migpad_core::history::Selection::caret(9));
        doc.edit(&[(0..0, "a\nb\nc 😀 d".as_bytes())], before, after, migpad_core::history::EditKind::Other, now)
            .unwrap();
        let path = Path::new("/notes/plan.txt");
        let lost = Losses { count: 1, first: Some(6) };
        let error = SaveError::Losses { encoding: lost, decoding: Losses::default() };
        let notice = save_failed(&doc, path, "windows-1251", &error).unwrap();
        assert_eq!(
            notice.message,
            "Characters windows-1251 cannot hold: 1, the first on line 3. “plan.txt” is not saved."
        );
        assert_eq!(notice.actions, [NoticeAction::SaveInUtf8, NoticeAction::ShowFirst(6), NoticeAction::SaveReplacing]);
        let denied = SaveError::PermissionDenied(io::Error::from(io::ErrorKind::PermissionDenied));
        let notice = save_failed(&doc, path, "UTF-8", &denied).unwrap();
        assert_eq!((notice.severity, notice.actions), (Severity::Error, vec![NoticeAction::SaveAs]));
        assert_eq!(save_failed(&doc, path, "UTF-8", &SaveError::Preview), None);
        set_language(Language::Russian);
        let error = SaveError::Losses { encoding: Losses::default(), decoding: lost };
        let notice = save_failed(&doc, path, "UTF-16LE", &error).unwrap();
        assert!(
            notice.message.starts_with("Мест «plan.txt», нечитаемых в кодировке UTF-16LE при открытии: 1"),
            "{}",
            notice.message
        );
        assert!(!notice.actions.contains(&NoticeAction::SaveInUtf8));
        set_language(Language::English);
    }

    #[test]
    fn a_stopped_journal_is_told_with_its_reason() {
        let _lock = LANGUAGE_LOCK.lock();
        let denied = io::Error::from(io::ErrorKind::PermissionDenied);
        set_language(Language::Russian);
        let notice = journal_failed("план.txt", &denied);
        assert_eq!(
            notice.message,
            "Правки «план.txt» больше не защищены от сбоя: не удалось записать журнал (нет прав на запись)."
        );
        assert_eq!((notice.severity, notice.topic), (Severity::Warning, Some(Topic::Journal)));
        set_language(Language::English);
        let full = io::Error::from(io::ErrorKind::StorageFull);
        assert_eq!(
            journal_failed("plan.txt", &full).message,
            "Changes to “plan.txt” are no longer kept safe from a crash: the journal could not be written (the disk is full)."
        );
        let error = SaveError::Io(io::Error::from(io::ErrorKind::StorageFull));
        let notice = save_failed(&Document::new(), Path::new("/notes/plan.txt"), "UTF-8", &error).unwrap();
        assert_eq!(notice.message, "Could not save “plan.txt”: the disk is full.");
    }

    #[test]
    fn a_document_read_without_losses_has_no_notice() {
        assert_eq!(decode_losses(&Document::new()), None);
    }
}
