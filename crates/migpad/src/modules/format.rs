//! The format of the file of a document: reading it again in another encoding, saving it in one,
//! and its line breaks. The encodings are listed as Notepad++ lists them: Unicode first, then a
//! submenu for each script. The status bar has the same menus.

use gpui::App;
use migpad_core::encoding::Encoding;
use migpad_core::line_ending::LineEnding;

use crate::commands::{Command, MenuId, Module, Registry, SubItem};
use crate::strings::{Key, fill};
use crate::workspace::Workspace;

/// Reads the file of the document again in this encoding.
#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = format, no_json)]
pub struct ReopenWithEncoding(pub Encoding);

/// Saves the document in this encoding, with a byte order mark or without.
#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = format, no_json)]
pub struct SaveWithEncoding(pub Encoding, pub bool);

/// Makes every line break of the document this one.
#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = format, no_json)]
pub struct ConvertLineEndings(pub LineEnding);

/// The encodings of each script, as the menus list them after Unicode.
pub const GROUPS: [(Key, &[&str]); 14] = [
    (Key::FormatCyrillic, &["windows-1251", "KOI8-R", "KOI8-U", "IBM866", "ISO-8859-5", "x-mac-cyrillic"]),
    (Key::FormatWestern, &["windows-1252", "ISO-8859-15", "macintosh"]),
    (Key::FormatCentral, &["windows-1250", "ISO-8859-2", "ISO-8859-16"]),
    (Key::FormatBaltic, &["windows-1257", "ISO-8859-4", "ISO-8859-13"]),
    (Key::FormatOtherEuropean, &["ISO-8859-3", "ISO-8859-10", "ISO-8859-14"]),
    (Key::FormatGreek, &["windows-1253", "ISO-8859-7"]),
    (Key::FormatTurkish, &["windows-1254"]),
    (Key::FormatHebrew, &["windows-1255", "ISO-8859-8", "ISO-8859-8-I"]),
    (Key::FormatArabic, &["windows-1256", "ISO-8859-6"]),
    (Key::FormatVietnamese, &["windows-1258"]),
    (Key::FormatThai, &["windows-874"]),
    (Key::FormatChinese, &["GBK", "gb18030", "Big5"]),
    (Key::FormatJapanese, &["Shift_JIS", "EUC-JP", "ISO-2022-JP"]),
    (Key::FormatKorean, &["EUC-KR"]),
];

/// The commands of the line breaks, in the order of the menus.
pub const LINE_ENDINGS: [&str; 3] = ["format.lf", "format.crlf", "format.cr"];

pub struct FormatModule;

impl Module for FormatModule {
    fn id(&self) -> &'static str {
        "format"
    }

    fn register(&self, registry: &mut Registry) {
        registry.add(Command::new("format.reopen", Key::FormatReopen, ReopenWithEncoding(Encoding::UTF_8)), None);
        registry.add_submenu(Key::FormatReopen, reopen_items, MenuId::File, 1);
        registry.add(Command::new("format.save", Key::FormatSave, SaveWithEncoding(Encoding::UTF_8, false)), None);
        registry.add_submenu(Key::FormatSave, save_items, MenuId::File, 2);
        // A large file is not converted: that would take a walk over all of it, and its undo step
        // would keep two copies of it.
        let line_endings = vec![
            Command::new("format.lf", Key::FormatLf, ConvertLineEndings(LineEnding::Lf))
                .checked(|workspace, cx| line_ending(workspace, cx) == LineEnding::Lf)
                .enabled(convertible),
            Command::new("format.crlf", Key::FormatCrlf, ConvertLineEndings(LineEnding::CrLf))
                .checked(|workspace, cx| line_ending(workspace, cx) == LineEnding::CrLf)
                .enabled(convertible),
            Command::new("format.cr", Key::FormatCr, ConvertLineEndings(LineEnding::Cr))
                .checked(|workspace, cx| line_ending(workspace, cx) == LineEnding::Cr)
                .enabled(convertible),
        ];
        registry.add_group(Key::FormatLineEndings, line_endings, MenuId::Edit, 3);

        registry.on_window_action(|workspace, reopen: &ReopenWithEncoding, window, cx| {
            workspace.reopen_with_encoding(reopen.0, window, cx)
        });
        registry.on_window_action(|workspace, save: &SaveWithEncoding, window, cx| {
            workspace.save_with_encoding(save.0, save.1, window, cx)
        });
        registry.on_window_action(|workspace, convert: &ConvertLineEndings, window, cx| {
            workspace.convert_line_endings(convert.0, window, cx)
        });
    }
}

fn line_ending(workspace: &Workspace, cx: &App) -> LineEnding {
    workspace.document().read(cx).format.line_ending
}

/// Whether the line breaks of the document of the window can be converted: not those of a large
/// file, nor of one still loading.
pub fn convertible(workspace: &Workspace, cx: &App) -> bool {
    let doc = workspace.document().read(cx);
    !doc.is_large() && !doc.is_preview()
}

/// The groups of the encodings of the scripts, each item made by `item`.
fn groups(item: impl Fn(Encoding) -> SubItem) -> impl Iterator<Item = SubItem> {
    GROUPS.into_iter().map(move |(label, names)| SubItem::Group {
        label,
        items: names.iter().filter_map(|name| Encoding::for_name(name)).map(&item).collect(),
    })
}

/// File ▸ Reopen with Encoding: Unicode, then the scripts; the encoding of the document checked.
/// An untitled document has no file to read again.
pub fn reopen_items(workspace: Option<&Workspace>, cx: &App) -> Vec<SubItem> {
    let doc = workspace.map(|workspace| workspace.document().read(cx));
    let current = doc.map(|doc| doc.format.encoding);
    let enabled = doc.is_some_and(|doc| doc.path.is_some());
    let item = |encoding: Encoding| SubItem::Choice {
        label: encoding.name().to_owned(),
        checked: current == Some(encoding),
        enabled,
        action: Box::new(ReopenWithEncoding(encoding)),
    };
    let mut items: Vec<SubItem> = [Encoding::UTF_8, Encoding::UTF_16LE, Encoding::UTF_16BE].map(item).into();
    items.push(SubItem::Separator);
    items.extend(groups(item));
    items
}

/// File ▸ Save with Encoding: UTF-8 without and with a byte order mark, UTF-16, then the scripts;
/// the format of the document checked. UTF-16 keeps the byte order mark of a document already in
/// it, and has one otherwise.
pub fn save_items(workspace: Option<&Workspace>, cx: &App) -> Vec<SubItem> {
    let doc = workspace.map(|workspace| workspace.document().read(cx));
    let format = doc.map(|doc| doc.format);
    let enabled = doc.is_some_and(|doc| !doc.is_preview());
    let is = |encoding: Encoding| format.is_some_and(|format| format.encoding == encoding);
    let choice = |label: String, checked: bool, encoding: Encoding, bom: bool| SubItem::Choice {
        label,
        checked,
        enabled,
        action: Box::new(SaveWithEncoding(encoding, bom)),
    };
    let utf8 = Encoding::UTF_8;
    let bom = format.is_some_and(|format| format.bom);
    let mut items = vec![
        choice(utf8.name().to_owned(), is(utf8) && !bom, utf8, false),
        choice(fill(Key::StatusWithBom, &[("encoding", utf8.name())]), is(utf8) && bom, utf8, true),
    ];
    for utf16 in [Encoding::UTF_16LE, Encoding::UTF_16BE] {
        let bom = if is(utf16) { bom } else { true };
        items.push(choice(utf16.name().to_owned(), is(utf16), utf16, bom));
    }
    items.push(SubItem::Separator);
    items.extend(groups(|encoding| choice(encoding.name().to_owned(), is(encoding), encoding, false)));
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strings::{LANGUAGE_LOCK, Language, mnemonic, set_language, tr};

    #[test]
    fn every_encoding_is_listed_once() {
        let mut listed: Vec<&str> = vec!["UTF-8", "UTF-16LE", "UTF-16BE"];
        listed.extend(GROUPS.iter().flat_map(|(_, names)| names.iter().copied()));
        assert!(listed.iter().all(|name| Encoding::for_name(name).is_some()), "{listed:?}");
        let mut all: Vec<&str> = Encoding::ALL.iter().map(|encoding| encoding.name()).collect();
        all.sort_unstable();
        listed.sort_unstable();
        assert_eq!(listed, all);
    }

    #[test]
    fn the_groups_have_mnemonics_of_their_own() {
        let _lock = LANGUAGE_LOCK.lock();
        for language in [Language::English, Language::Russian] {
            set_language(language);
            let mut letters: Vec<String> = GROUPS
                .iter()
                .map(|(key, _)| {
                    let at = mnemonic(*key).unwrap_or_else(|| panic!("{language:?}: {key:?} has no mnemonic"));
                    tr(*key)[at..].chars().next().unwrap().to_lowercase().to_string()
                })
                .collect();
            letters.sort();
            letters.dedup();
            assert_eq!(letters.len(), GROUPS.len(), "{language:?}: mnemonics of the groups repeat");
        }
        set_language(Language::English);
    }
}
