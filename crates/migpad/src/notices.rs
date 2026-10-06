//! Notifications over the text of a tab: what went wrong opening a file, bytes that could not be
//! read in its encoding.

use std::io;
use std::path::Path;

use migpad_core::document::{Document, OpenError};
use migpad_core::text::TextStore;
use migpad_ui::notification::Severity;

use crate::strings::{Key, fill, number, tr};

/// A notification of a tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub severity: Severity,
    pub message: String,
}

/// The file at `path` could not be opened.
pub fn open_failed(path: &Path, error: &OpenError) -> Notice {
    let file = path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned());
    let reason = match error {
        OpenError::Io(error) => match error.kind() {
            io::ErrorKind::PermissionDenied => tr(Key::NoticeNoPermission).to_owned(),
            io::ErrorKind::IsADirectory => tr(Key::NoticeIsFolder).to_owned(),
            _ => error.to_string(),
        },
        OpenError::TooLarge => tr(Key::NoticeTooLarge).to_owned(),
        OpenError::Cancelled => error.to_string(),
    };
    Notice { severity: Severity::Error, message: fill(Key::NoticeOpenFailed, &[("file", &file), ("reason", &reason)]) }
}

/// Bytes of the file of `doc` that were not valid in its encoding and became U+FFFD, if any.
pub fn decode_losses(doc: &Document) -> Option<Notice> {
    let losses = doc.decode_losses();
    let first = losses.first.filter(|_| !losses.is_empty())?;
    let line = doc.lines().line_of(first.min(doc.text().len())) as u64 + 1;
    let message = fill(Key::NoticeDecodeLosses, &[("encoding", doc.format.encoding.name()), ("line", &number(line))]);
    Some(Notice { severity: Severity::Warning, message })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strings::{LANGUAGE_LOCK, Language, set_language};

    #[test]
    fn errors_of_opening_are_told_by_their_kind() {
        let _lock = LANGUAGE_LOCK.lock();
        let path = Path::new("/notes/план.txt");
        let denied = OpenError::Io(io::Error::from(io::ErrorKind::PermissionDenied));
        let folder = OpenError::Io(io::Error::from(io::ErrorKind::IsADirectory));
        set_language(Language::Russian);
        assert_eq!(open_failed(path, &denied).message, "Не удалось открыть «план.txt»: нет прав на чтение.");
        assert_eq!(open_failed(path, &folder).message, "Не удалось открыть «план.txt»: это папка.");
        assert_eq!(open_failed(path, &OpenError::TooLarge).message, "Не удалось открыть «план.txt»: файл больше 4 ГиБ.");
        set_language(Language::English);
        assert_eq!(open_failed(path, &denied).message, "Could not open “план.txt”: no permission to read it.");
        assert_eq!(open_failed(path, &OpenError::TooLarge).severity, Severity::Error);
    }

    #[test]
    fn a_document_read_without_losses_has_no_notice() {
        assert_eq!(decode_losses(&Document::new()), None);
    }
}
