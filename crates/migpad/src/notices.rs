//! Notifications over the text of a tab: what went wrong opening or saving a file, bytes that
//! could not be read in its encoding.

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
}

impl NoticeAction {
    pub fn label(self) -> &'static str {
        tr(match self {
            NoticeAction::SaveInUtf8 => Key::NoticeSaveInUtf8,
            NoticeAction::ShowFirst(_) => Key::NoticeShowFirst,
            NoticeAction::SaveReplacing => Key::NoticeSaveReplacing,
            NoticeAction::SaveAs => Key::NoticeSaveAs,
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
            message: fill(Key::NoticeSaveFailed, &[("file", &file), ("reason", &error.to_string())]),
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
    fn a_document_read_without_losses_has_no_notice() {
        assert_eq!(decode_losses(&Document::new()), None);
    }
}
