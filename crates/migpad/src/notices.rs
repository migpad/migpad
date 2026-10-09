//! Notifications over the text of a tab: what went wrong opening or saving a file, bytes that
//! could not be read in its encoding, a journal that stopped, a file changed by another program.

use std::io;
use std::path::{Path, PathBuf};

use migpad_core::document::{Document, Format, OpenError, SaveError};
use migpad_core::encoding::Losses;
use migpad_core::settings::{Expected, Problem};
use migpad_core::text::TextStore;
use migpad_ui::notification::Severity;

use crate::strings::{Key, Unfit, fill, number, tr};

/// A notification of a tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub severity: Severity,
    pub message: String,
    /// What it is about: a new notification about the same takes the place of the old one.
    pub topic: Option<Topic>,
    /// The buttons for what can be done about it.
    pub actions: Vec<NoticeAction>,
    /// What a failed save aimed at: its buttons save there and so, not to the file of the document
    /// in its format.
    pub target: Option<SaveTarget>,
}

/// The file and the format a save aimed at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveTarget {
    pub path: PathBuf,
    pub format: Format,
}

impl Notice {
    fn new(severity: Severity, message: String) -> Self {
        Notice { severity, message, topic: None, actions: Vec::new(), target: None }
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
    /// The file of the settings.
    Settings,
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
    /// Opens the file of the settings in a tab.
    OpenSettings,
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
            NoticeAction::OpenSettings => Key::SettingsOpenFile,
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

/// `doc` was not saved to `path` in `format`: what was in the way, and what can be done. Nothing
/// to tell for a preview, which cannot be saved at all.
pub fn save_failed(doc: &Document, path: &Path, format: Format, error: &SaveError) -> Option<Notice> {
    let file = file_name(path);
    let encoding = format.encoding.name();
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
                target: None,
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
                target: None,
            }
        }
        SaveError::PermissionDenied(_) => Notice {
            severity: Severity::Error,
            message: fill(Key::NoticeNoWritePermission, &[("file", &file)]),
            topic: Some(Topic::Save),
            actions: vec![NoticeAction::SaveAs],
            target: None,
        },
        SaveError::Io(error) => Notice {
            severity: Severity::Error,
            message: fill(Key::NoticeSaveFailed, &[("file", &file), ("reason", &write_reason(error))]),
            topic: Some(Topic::Save),
            actions: vec![NoticeAction::SaveAs],
            target: None,
        },
        SaveError::Preview => return None,
    };
    Some(Notice { target: Some(SaveTarget { path: path.to_owned(), format }), ..notice })
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
        target: None,
    }
}

/// The file `file` was changed by another program, and the document has changes of its own: they
/// are shown, and the file on disk is a choice away.
pub fn changed_on_disk(file: &str) -> Notice {
    Notice {
        severity: Severity::Warning,
        message: fill(Key::NoticeChangedOnDisk, &[("file", file)]),
        topic: Some(Topic::Disk),
        actions: vec![NoticeAction::LoadFromDisk, NoticeAction::KeepMine],
        target: None,
    }
}

/// The large file `file` was changed while MigPad was closed, and its journal has no copy of it
/// for the changes to go on: it opens as it is, and the changes are set aside.
pub fn changes_set_aside(file: &str) -> Notice {
    let message = fill(Key::NoticeChangesSetAside, &[("file", file)]);
    Notice { topic: Some(Topic::Disk), ..Notice::new(Severity::Warning, message) }
}

/// The file `file` was read again in another encoding, and the changes of its text went among the
/// closed tabs.
pub fn changes_kept(file: &str) -> Notice {
    Notice::new(Severity::Info, fill(Key::NoticeChangesKept, &[("file", file)]))
}

/// The file `file` is not on disk any more: there is nothing to read again.
pub fn gone_from_disk(file: &str) -> Notice {
    Notice::new(Severity::Warning, fill(Key::NoticeGoneFromDisk, &[("file", file)]))
}

/// The line breaks of the large file `file` are not converted: that would take a walk over all of
/// it, and its undo step would keep two copies of it.
pub fn too_large_to_convert(file: &str) -> Notice {
    Notice::new(Severity::Info, fill(Key::NoticeTooLargeToConvert, &[("file", file)]))
}

/// Another running copy of MigPad holds the folder of data: this one keeps nothing there.
pub fn another_copy() -> Notice {
    Notice::new(Severity::Warning, tr(Key::NoticeAnotherCopy).to_owned())
}

/// The command `migpad` is a link at `path` to MigPad.
pub fn command_installed(path: &str) -> Notice {
    Notice::new(Severity::Info, fill(Key::NoticeCommandInstalled, &[("path", path)]))
}

/// MigPad runs from a disk image or a temporary copy: a link to it would lead nowhere.
pub fn command_temporary() -> Notice {
    Notice::new(Severity::Warning, tr(Key::NoticeCommandTemporary).to_owned())
}

/// The command `migpad` could not be made.
pub fn command_failed(reason: &str) -> Notice {
    Notice::new(Severity::Error, fill(Key::NoticeCommandFailed, &[("reason", reason)]))
}

/// The recent file `file` is not there any more: it leaves the list.
pub fn recent_gone(file: &str) -> Notice {
    Notice::new(Severity::Warning, fill(Key::NoticeRecentGone, &[("file", file)]))
}

/// The changes of `file` could not be recovered from its journal, which is set aside.
pub fn recover_failed(file: &str, reason: &str) -> Notice {
    Notice::new(Severity::Error, fill(Key::NoticeRecoverFailed, &[("file", file), ("reason", reason)]))
}

/// What is wrong with the file of the settings, if anything: the settings it could not give have
/// their defaults.
pub fn settings_problems(problems: &[Problem]) -> Option<Notice> {
    let invalid: Vec<String> = problems
        .iter()
        .filter_map(|problem| match problem {
            Problem::Invalid { key, line, expected } => {
                let line = line.map_or_else(|| "?".to_owned(), |line| number(line as u64));
                let expected = expected_value(expected);
                let values = [("key", *key), ("line", line.as_str()), ("expected", expected.as_str())];
                Some(fill(Key::SettingsInvalidItem, &values))
            }
            _ => None,
        })
        .collect();
    let (severity, message) = match problems.first()? {
        Problem::Unreadable(reason) => (Severity::Error, fill(Key::SettingsUnreadable, &[("reason", reason)])),
        Problem::Syntax { line, message } => {
            let line = number(*line as u64);
            (Severity::Error, fill(Key::SettingsSyntax, &[("line", line.as_str()), ("reason", message.as_str())]))
        }
        Problem::Invalid { .. } => (Severity::Warning, fill(Key::SettingsInvalid, &[("list", &invalid.join("; "))])),
    };
    Some(Notice {
        severity,
        message,
        topic: Some(Topic::Settings),
        actions: vec![NoticeAction::OpenSettings],
        target: None,
    })
}

/// What a setting takes, in words.
fn expected_value(expected: &Expected) -> String {
    match expected {
        Expected::Bool => tr(Key::SettingsExpectedBool).to_owned(),
        Expected::Number(range) => {
            let (min, max) = (range.start().to_string(), range.end().to_string());
            fill(Key::SettingsExpectedNumber, &[("min", min.as_str()), ("max", max.as_str())])
        }
        Expected::OneOf(values) => {
            let values: Vec<String> = values.iter().map(|value| format!("\"{value}\"")).collect();
            fill(Key::SettingsExpectedOneOf, &[("values", values.join(", ").as_str())])
        }
        Expected::Text => tr(Key::SettingsExpectedText).to_owned(),
    }
}

/// The settings could not be written to the file at `path`.
pub fn settings_write_failed(path: &Path, error: &io::Error) -> Notice {
    let (file, reason) = (path.display().to_string(), write_reason(error));
    let message = fill(Key::SettingsWriteFailed, &[("file", file.as_str()), ("reason", reason.as_str())]);
    Notice {
        severity: Severity::Error,
        message,
        topic: Some(Topic::Settings),
        actions: vec![NoticeAction::OpenSettings],
        target: None,
    }
}

/// The font of the settings, `font`, is not in the system: documents show in `fallback`.
pub fn font_missing(font: &str, fallback: &str) -> Notice {
    let message = fill(Key::SettingsFontMissing, &[("font", font), ("fallback", fallback)]);
    Notice {
        severity: Severity::Warning,
        message,
        topic: Some(Topic::Settings),
        actions: vec![NoticeAction::OpenSettings],
        target: None,
    }
}

/// How many strings of a translation that MigPad cannot take are named.
const UNFIT_NAMED: usize = 10;

/// The translation `file` could not be read: MigPad shows its own strings.
pub fn translation_unreadable(file: &str, reason: &str) -> Notice {
    Notice::new(Severity::Warning, fill(Key::TranslationUnreadable, &[("file", file), ("reason", reason)]))
}

/// The translation `file` has strings MigPad cannot take, `unfit`: it shows its own for them.
pub fn translation_unfit(file: &str, unfit: &[(String, Unfit)]) -> Notice {
    let mut items: Vec<String> = unfit
        .iter()
        .take(UNFIT_NAMED)
        .map(|(key, why)| {
            let why = tr(match why {
                Unfit::UnknownKey => Key::TranslationUnknown,
                Unfit::Placeholders => Key::TranslationPlaceholders,
                Unfit::Forms => Key::TranslationForms,
                Unfit::StrayAmpersand => Key::TranslationAmpersand,
            });
            fill(Key::TranslationItem, &[("key", key.as_str()), ("why", why)])
        })
        .collect();
    if unfit.len() > UNFIT_NAMED {
        items.push(fill(Key::TranslationMore, &[("count", &number((unfit.len() - UNFIT_NAMED) as u64))]));
    }
    let message = fill(Key::TranslationUnfit, &[("file", file), ("list", &items.join("; "))]);
    Notice::new(Severity::Warning, message)
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

    fn format(encoding: &str) -> Format {
        Format { encoding: migpad_core::encoding::Encoding::for_name(encoding).unwrap(), ..Format::default() }
    }

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
        let notice = save_failed(&doc, path, format("windows-1251"), &error).unwrap();
        assert_eq!(
            notice.message,
            "Characters windows-1251 cannot hold: 1, the first on line 3. “plan.txt” is not saved."
        );
        assert_eq!(notice.actions, [NoticeAction::SaveInUtf8, NoticeAction::ShowFirst(6), NoticeAction::SaveReplacing]);
        let denied = SaveError::PermissionDenied(io::Error::from(io::ErrorKind::PermissionDenied));
        let notice = save_failed(&doc, path, format("UTF-8"), &denied).unwrap();
        assert_eq!((notice.severity, notice.actions), (Severity::Error, vec![NoticeAction::SaveAs]));
        assert_eq!(save_failed(&doc, path, format("UTF-8"), &SaveError::Preview), None);
        set_language(Language::Russian);
        let error = SaveError::Losses { encoding: Losses::default(), decoding: lost };
        let notice = save_failed(&doc, path, format("UTF-16LE"), &error).unwrap();
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
        let notice = save_failed(&Document::new(), Path::new("/notes/plan.txt"), format("UTF-8"), &error).unwrap();
        assert_eq!(notice.message, "Could not save “plan.txt”: the disk is full.");
    }

    #[test]
    fn problems_of_the_settings_are_told() {
        let _lock = LANGUAGE_LOCK.lock();
        set_language(Language::English);
        assert_eq!(settings_problems(&[]), None);
        let syntax = Problem::Syntax { line: 3, message: "invalid table header".into() };
        let notice = settings_problems(&[syntax]).unwrap();
        assert_eq!(
            notice.message,
            "The settings file has an error on line 3: invalid table header. Until it is fixed, MigPad does not take settings from it or write to it."
        );
        assert_eq!((notice.severity, notice.topic), (Severity::Error, Some(Topic::Settings)));
        assert_eq!(notice.actions, [NoticeAction::OpenSettings]);
        let problems = [
            Problem::Invalid { key: "editor.tab_width", line: Some(5), expected: Expected::Number(1..=16) },
            Problem::Invalid { key: "theme", line: Some(2), expected: Expected::OneOf(&["system", "light", "dark"]) },
        ];
        set_language(Language::Russian);
        let notice = settings_problems(&problems).unwrap();
        assert_eq!(
            notice.message,
            "У некоторых настроек значения, которые MigPad не принимает, — они не применены: editor.tab_width в строке 5 — целое число от 1 до 16; theme в строке 2 — одно из значений \"system\", \"light\", \"dark\"."
        );
        assert_eq!(notice.severity, Severity::Warning);
        set_language(Language::English);
    }

    #[test]
    fn a_document_read_without_losses_has_no_notice() {
        assert_eq!(decode_losses(&Document::new()), None);
    }
}
