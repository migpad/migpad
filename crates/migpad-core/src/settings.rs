//! The settings of MigPad: `settings.toml` in the folder of the data, which the settings window
//! writes and one may edit by hand. A setting the file does not have has its default; what is
//! wrong with the file changes nothing — a value MigPad cannot take, or the whole file if it is not
//! TOML, leaves the settings as they were, the defaults as MigPad starts. Changes go into the text
//! of the file, so that its comments, its order and the keys MigPad does not know stay as they
//! were; a file that is not TOML is not written over.

use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};
use std::{fs, io};

use toml_edit::{Document, DocumentMut, Item, Table, TableLike, Value};

use crate::state::write_atomically;

/// The sizes the font of documents can take.
pub const FONT_SIZES: RangeInclusive<u32> = 6..=72;
/// The widths of a tab, in columns.
pub const TAB_WIDTHS: RangeInclusive<u32> = 1..=16;

const LANGUAGES: &[&str] = &["system", "en", "ru"];
const THEMES: &[&str] = &["system", "light", "dark"];

/// The language of the interface.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Language {
    /// Russian if the system is in Russian, English otherwise.
    #[default]
    System,
    English,
    Russian,
}

impl Language {
    fn name(self) -> &'static str {
        LANGUAGES[self as usize]
    }

    fn named(name: &str) -> Option<Language> {
        [Language::System, Language::English, Language::Russian].into_iter().find(|language| language.name() == name)
    }
}

/// The theme: light or dark.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Theme {
    /// Light or dark as the system is, following it.
    #[default]
    System,
    Light,
    Dark,
}

impl Theme {
    fn name(self) -> &'static str {
        THEMES[self as usize]
    }

    fn named(name: &str) -> Option<Theme> {
        [Theme::System, Theme::Light, Theme::Dark].into_iter().find(|theme| theme.name() == name)
    }
}

/// The settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub language: Language,
    pub theme: Theme,
    /// Whether the windows and tabs of the last time open again; documents with unsaved changes
    /// come back anyway.
    pub restore_session: bool,
    /// The font of documents, a monospace one installed in the system; `None` for that of the
    /// system.
    pub font: Option<String>,
    pub font_size: u32,
    /// Columns between tab stops.
    pub tab_width: u32,
    pub word_wrap: bool,
    /// Whether spaces, tabs and line breaks are marked.
    pub invisibles: bool,
    pub indent_guides: bool,
    /// macOS: whether each window has the menu bar MigPad draws, besides that of the system.
    pub menu_bar: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            language: Language::System,
            theme: Theme::System,
            restore_session: true,
            font: None,
            font_size: 13,
            tab_width: 8,
            word_wrap: true,
            invisibles: false,
            indent_guides: false,
            menu_bar: false,
        }
    }
}

impl Settings {
    /// The setting `key` with its value here.
    pub fn get(&self, key: &str) -> Option<Setting> {
        Some(match key {
            "language" => Setting::Language(self.language),
            "theme" => Setting::Theme(self.theme),
            "restore_session" => Setting::RestoreSession(self.restore_session),
            "editor.font" => Setting::Font(self.font.clone()),
            "editor.font_size" => Setting::FontSize(self.font_size),
            "editor.tab_width" => Setting::TabWidth(self.tab_width),
            "view.word_wrap" => Setting::WordWrap(self.word_wrap),
            "view.invisibles" => Setting::Invisibles(self.invisibles),
            "view.indent_guides" => Setting::IndentGuides(self.indent_guides),
            "view.menu_bar" => Setting::MenuBar(self.menu_bar),
            _ => return None,
        })
    }

    /// Takes the value of `setting`.
    pub fn apply(&mut self, setting: Setting) {
        match setting {
            Setting::Language(language) => self.language = language,
            Setting::Theme(theme) => self.theme = theme,
            Setting::RestoreSession(restore) => self.restore_session = restore,
            Setting::Font(font) => self.font = font,
            Setting::FontSize(size) => self.font_size = size,
            Setting::TabWidth(width) => self.tab_width = width,
            Setting::WordWrap(wrap) => self.word_wrap = wrap,
            Setting::Invisibles(shown) => self.invisibles = shown,
            Setting::IndentGuides(shown) => self.indent_guides = shown,
            Setting::MenuBar(shown) => self.menu_bar = shown,
        }
    }
}

/// A setting with a value: a change to make.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Setting {
    Language(Language),
    Theme(Theme),
    RestoreSession(bool),
    /// A font family; `None` for that of the system.
    Font(Option<String>),
    /// A size within [`FONT_SIZES`].
    FontSize(u32),
    /// A width within [`TAB_WIDTHS`].
    TabWidth(u32),
    WordWrap(bool),
    Invisibles(bool),
    IndentGuides(bool),
    MenuBar(bool),
}

impl Setting {
    /// Its key in the file: `editor.tab_width` is `tab_width` in the table `[editor]`.
    pub fn key(&self) -> &'static str {
        match self {
            Setting::Language(_) => "language",
            Setting::Theme(_) => "theme",
            Setting::RestoreSession(_) => "restore_session",
            Setting::Font(_) => "editor.font",
            Setting::FontSize(_) => "editor.font_size",
            Setting::TabWidth(_) => "editor.tab_width",
            Setting::WordWrap(_) => "view.word_wrap",
            Setting::Invisibles(_) => "view.invisibles",
            Setting::IndentGuides(_) => "view.indent_guides",
            Setting::MenuBar(_) => "view.menu_bar",
        }
    }

    fn to_value(&self) -> Value {
        match self {
            Setting::Language(language) => language.name().into(),
            Setting::Theme(theme) => theme.name().into(),
            Setting::Font(font) => font.as_deref().unwrap_or_default().into(),
            Setting::FontSize(n) | Setting::TabWidth(n) => i64::from(*n).into(),
            Setting::RestoreSession(on)
            | Setting::WordWrap(on)
            | Setting::Invisibles(on)
            | Setting::IndentGuides(on)
            | Setting::MenuBar(on) => (*on).into(),
        }
    }

    /// The setting `key` with the value of `item`; `None` if it is not one the setting takes.
    fn read(key: &str, item: &Item) -> Option<Setting> {
        let value = item.as_value()?;
        let number = |range: &RangeInclusive<u32>| {
            value.as_integer().and_then(|n| u32::try_from(n).ok()).filter(|n| range.contains(n))
        };
        Some(match key {
            "language" => Setting::Language(Language::named(value.as_str()?)?),
            "theme" => Setting::Theme(Theme::named(value.as_str()?)?),
            "restore_session" => Setting::RestoreSession(value.as_bool()?),
            "editor.font" => {
                Setting::Font(Some(value.as_str()?.trim()).filter(|font| !font.is_empty()).map(str::to_owned))
            }
            "editor.font_size" => Setting::FontSize(number(&FONT_SIZES)?),
            "editor.tab_width" => Setting::TabWidth(number(&TAB_WIDTHS)?),
            "view.word_wrap" => Setting::WordWrap(value.as_bool()?),
            "view.invisibles" => Setting::Invisibles(value.as_bool()?),
            "view.indent_guides" => Setting::IndentGuides(value.as_bool()?),
            "view.menu_bar" => Setting::MenuBar(value.as_bool()?),
            _ => return None,
        })
    }
}

/// The keys of the settings and what each takes.
const KEYS: [(&str, Expected); 10] = [
    ("language", Expected::OneOf(LANGUAGES)),
    ("theme", Expected::OneOf(THEMES)),
    ("restore_session", Expected::Bool),
    ("editor.font", Expected::Text),
    ("editor.font_size", Expected::Number(FONT_SIZES)),
    ("editor.tab_width", Expected::Number(TAB_WIDTHS)),
    ("view.word_wrap", Expected::Bool),
    ("view.invisibles", Expected::Bool),
    ("view.indent_guides", Expected::Bool),
    ("view.menu_bar", Expected::Bool),
];

/// What a setting takes: what a value it could not take should have been.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expected {
    /// `true` or `false`.
    Bool,
    /// A whole number in the range.
    Number(RangeInclusive<u32>),
    /// One of the strings.
    OneOf(&'static [&'static str]),
    /// A string.
    Text,
}

/// What is wrong with the file of the settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    /// The file could not be read: the settings stay as they were, and it is not written over.
    Unreadable(String),
    /// The file is not TOML at `line`, from 1: the settings stay as they were, and it is not
    /// written over.
    Syntax { line: usize, message: String },
    /// The setting `key`, at `line`, has a value it cannot take: it keeps the value it had.
    Invalid { key: &'static str, line: Option<usize>, expected: Expected },
}

/// The file of the settings: the settings it gives, and its text, to write changes into.
#[derive(Clone, Debug)]
pub struct SettingsFile {
    settings: Settings,
    /// The text of the file; `None` if it could not be read or is not TOML, so as not to write it
    /// over.
    doc: Option<DocumentMut>,
    problems: Vec<Problem>,
}

impl Default for SettingsFile {
    /// No file yet: the defaults, and the text of a new file, which tells what each setting is.
    fn default() -> Self {
        Self::parse(&template(), &Settings::default())
    }
}

impl SettingsFile {
    /// The file at `path`; the defaults if there is none. What is wrong with it leaves the settings
    /// as in `previous`.
    pub fn read(path: &Path, previous: &Settings) -> SettingsFile {
        match fs::read_to_string(path) {
            Ok(text) => Self::parse(&text, previous),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Self::default(),
            Err(error) => SettingsFile {
                settings: previous.clone(),
                doc: None,
                problems: vec![Problem::Unreadable(error.to_string())],
            },
        }
    }

    /// The settings of `text`, the text of the file: a setting it does not have has its default, one
    /// with a value it cannot take — or all of them, if it is not TOML — the value in `previous`.
    pub fn parse(text: &str, previous: &Settings) -> SettingsFile {
        let parsed = match Document::parse(text) {
            Ok(parsed) => parsed,
            Err(error) => {
                let line = error.span().map_or(1, |span| line_of(text, span.start));
                let problem = Problem::Syntax { line, message: error.message().trim().to_owned() };
                return SettingsFile { settings: previous.clone(), doc: None, problems: vec![problem] };
            }
        };
        let mut settings = Settings::default();
        let mut problems = Vec::new();
        for (key, expected) in KEYS {
            let Some(item) = find(parsed.as_table(), key) else { continue };
            match Setting::read(key, item) {
                Some(setting) => settings.apply(setting),
                None => {
                    settings.apply(previous.get(key).expect("a key of the settings"));
                    let line = item.span().map(|span| line_of(text, span.start));
                    problems.push(Problem::Invalid { key, line, expected });
                }
            }
        }
        SettingsFile { settings, doc: Some(parsed.into_mut()), problems }
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// What is wrong with the file, as it was read; a setting changed since is not among them.
    pub fn problems(&self) -> &[Problem] {
        &self.problems
    }

    /// Whether changes can be written to the file: it is TOML.
    pub fn is_writable(&self) -> bool {
        self.doc.is_some()
    }

    /// Changes a setting, in the text of the file as well; tells whether the settings changed.
    pub fn set(&mut self, setting: Setting) -> bool {
        let key = setting.key();
        self.problems.retain(|problem| !matches!(problem, Problem::Invalid { key: wrong, .. } if *wrong == key));
        if let Some(doc) = &mut self.doc {
            put(doc.as_table_mut(), key, setting.to_value());
        }
        let before = self.settings.clone();
        self.settings.apply(setting);
        self.settings != before
    }

    /// The text of the file with the changes; `None` if it is not to be written over.
    pub fn to_toml(&self) -> Option<String> {
        self.doc.as_ref().map(DocumentMut::to_string)
    }

    /// Writes the file to `path`, whole, through a temporary file: in place of what a link there
    /// points to, so that a file kept with other dotfiles stays linked. Nothing is written for a
    /// file that is not to be written over.
    pub fn write(&self, path: &Path) -> io::Result<()> {
        let Some(text) = self.to_toml() else { return Ok(()) };
        write_atomically(&target(path), &text)
    }
}

/// The text of a new file: every setting with its default, and what it is.
pub fn template() -> String {
    let mut text = String::from(
        "# The settings of MigPad. The settings window writes them here, and you can edit them by hand:\n\
         # MigPad takes the changes once the file is saved. A setting that is not here, or has a value\n\
         # MigPad cannot take, has its default.\n\
         \n\
         # The language of the interface: \"en\", \"ru\", or \"system\" — Russian if the system is in\n\
         # Russian, English otherwise.\n\
         language = \"system\"\n\
         # \"light\", \"dark\", or \"system\" — as the system is.\n\
         theme = \"system\"\n\
         # Whether the windows and tabs of the last time open again. Documents with unsaved changes\n\
         # always come back.\n\
         restore_session = true\n\
         \n\
         [editor]\n\
         # A monospace font installed in the system; \"\" for that of the system.\n\
         font = \"\"\n\
         # The size of the font, from 6 to 72.\n\
         font_size = 13\n\
         # Columns between tab stops, from 1 to 16.\n\
         tab_width = 8\n\
         \n\
         # The View menu: each is on or off in every window.\n\
         [view]\n\
         word_wrap = true\n\
         invisibles = false\n\
         indent_guides = false\n",
    );
    if cfg!(target_os = "macos") {
        text += "# The menu bar in each window, besides that of the system.\nmenu_bar = false\n";
    }
    text
}

/// Where the file at `path` is written: the file a link there points to, if it is a link.
fn target(path: &Path) -> PathBuf {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => fs::canonicalize(path).unwrap_or_else(|_| path.into()),
        _ => path.into(),
    }
}

/// The item of `key`, such as `editor.tab_width`, in `table`.
fn find<'a>(table: &'a Table, key: &str) -> Option<&'a Item> {
    let (path, name) = key.rsplit_once('.').map_or((None, key), |(path, name)| (Some(path), name));
    let mut table: &dyn TableLike = table;
    for part in path.into_iter().flat_map(|path| path.split('.')) {
        table = table.get(part)?.as_table_like()?;
    }
    table.get(name)
}

/// Puts `value` at `key` in `table`: the tables on the way are made, and a value there keeps the
/// spaces and the comment around it.
fn put(table: &mut Table, key: &str, mut value: Value) {
    let (path, name) = key.rsplit_once('.').map_or((None, key), |(path, name)| (Some(path), name));
    let mut table: &mut dyn TableLike = table;
    for part in path.into_iter().flat_map(|path| path.split('.')) {
        let item = table.entry(part).or_insert_with(|| Item::Table(Table::new()));
        if !item.is_table_like() {
            *item = Item::Table(Table::new());
        }
        table = item.as_table_like_mut().expect("made a table above");
    }
    match table.get_mut(name) {
        Some(Item::Value(old)) => {
            *value.decor_mut() = old.decor().clone();
            *old = value;
        }
        _ => {
            table.insert(name, Item::Value(value));
        }
    }
}

/// The line of the byte at `offset` in `text`, from 1.
fn line_of(text: &str, offset: usize) -> usize {
    text.as_bytes()[..offset.min(text.len())].iter().filter(|&&byte| byte == b'\n').count() + 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    fn parse(text: &str) -> SettingsFile {
        SettingsFile::parse(text, &Settings::default())
    }

    fn read(path: &Path) -> SettingsFile {
        SettingsFile::read(path, &Settings::default())
    }

    #[test]
    fn no_file_has_the_defaults() {
        let dir = TempDir::new("settings-none");
        let file = read(&dir.0.join("settings.toml"));
        assert_eq!(file.settings(), &Settings::default());
        assert!(file.problems().is_empty());
        // The text of a new file has every setting with its default.
        assert_eq!(parse(&template()).settings(), &Settings::default());
        for (key, _) in KEYS {
            let item = find(Document::parse(template()).unwrap().as_table(), key).cloned();
            assert!(item.is_some() || key == "view.menu_bar", "{key} is not in the template");
        }
    }

    #[test]
    fn every_setting_is_read_and_written() {
        let changes = [
            Setting::Language(Language::Russian),
            Setting::Theme(Theme::Dark),
            Setting::RestoreSession(false),
            Setting::Font(Some("Fira Code".into())),
            Setting::FontSize(20),
            Setting::TabWidth(4),
            Setting::WordWrap(false),
            Setting::Invisibles(true),
            Setting::IndentGuides(true),
            Setting::MenuBar(true),
        ];
        let mut file = parse("");
        let mut expected = Settings::default();
        for change in changes {
            assert!(file.set(change.clone()), "{change:?} changes the settings");
            expected.apply(change.clone());
            assert!(!file.set(change), "the same value again changes nothing");
        }
        assert_eq!(file.settings(), &expected);
        let text = file.to_toml().unwrap();
        let again = parse(&text);
        assert_eq!(again.settings(), &expected, "{text}");
        assert!(again.problems().is_empty());
        assert!(text.contains("[editor]") && text.contains("tab_width = 4"), "{text}");
        // An empty font is that of the system.
        let mut file = parse("[editor]\nfont = \"  \"\n");
        assert_eq!(file.settings().font, None);
        file.set(Setting::Font(None));
        assert!(file.to_toml().unwrap().contains("font = \"\""));
    }

    #[test]
    fn values_out_of_place_have_their_defaults() {
        let text = "language = \"de\"\ntheme = 1\n\n[editor]\nfont_size = 100\ntab_width = 4\nfont = 5\n\n[view]\nword_wrap = \"yes\"\n";
        let file = parse(text);
        let expected = Settings { tab_width: 4, ..Settings::default() };
        assert_eq!(file.settings(), &expected);
        let problems = file.problems();
        assert_eq!(problems.len(), 5, "{problems:?}");
        assert_eq!(
            problems[0],
            Problem::Invalid { key: "language", line: Some(1), expected: Expected::OneOf(LANGUAGES) }
        );
        assert_eq!(problems[1], Problem::Invalid { key: "theme", line: Some(2), expected: Expected::OneOf(THEMES) });
        assert_eq!(problems[2], Problem::Invalid { key: "editor.font", line: Some(7), expected: Expected::Text });
        assert_eq!(
            problems[3],
            Problem::Invalid { key: "editor.font_size", line: Some(5), expected: Expected::Number(FONT_SIZES) }
        );
        assert_eq!(problems[4], Problem::Invalid { key: "view.word_wrap", line: Some(10), expected: Expected::Bool });
        // A table where a setting should be is one too.
        let file = parse("[language]\nname = \"ru\"\n");
        assert!(matches!(file.problems(), [Problem::Invalid { key: "language", .. }]));
    }

    #[test]
    fn what_is_wrong_changes_nothing() {
        let previous = Settings { theme: Theme::Dark, font_size: 20, tab_width: 2, ..Settings::default() };
        // A value out of place keeps the one it had; a setting the file has not has its default.
        let file = SettingsFile::parse("[editor]\nfont_size = 100\n", &previous);
        assert_eq!(file.settings(), &Settings { font_size: 20, ..Settings::default() });
        assert_eq!(file.problems().len(), 1);
        // A file that is not TOML leaves everything as it was.
        let file = SettingsFile::parse("[editor\n", &previous);
        assert_eq!(file.settings(), &previous);
        let dir = TempDir::new("settings-previous");
        let path = dir.0.join("settings.toml");
        fs::write(&path, b"theme = \"\xff\"\n").unwrap();
        assert_eq!(SettingsFile::read(&path, &previous).settings(), &previous);
        // No file is the defaults.
        assert_eq!(SettingsFile::read(&dir.0.join("none.toml"), &previous).settings(), &Settings::default());
        // Every key of the settings has its value.
        for (key, _) in KEYS {
            assert_eq!(previous.get(key).map(|setting| setting.key()), Some(key));
        }
    }

    #[test]
    fn a_setting_changed_is_no_longer_a_problem() {
        let mut file = parse("[editor]\ntab_width = 100 # too wide\n");
        assert_eq!(file.problems().len(), 1);
        // Even to the value it has now — the default — the file gets it.
        assert!(!file.set(Setting::TabWidth(8)));
        assert!(file.problems().is_empty());
        assert_eq!(file.to_toml().unwrap(), "[editor]\ntab_width = 8 # too wide\n");
    }

    #[test]
    fn a_file_that_is_not_toml_is_not_written_over() {
        let mut file = parse("language = \"ru\"\n\n[editor\ntab_width = 4\n");
        assert_eq!(file.settings(), &Settings::default());
        assert!(matches!(file.problems(), [Problem::Syntax { line: 3, .. }]), "{:?}", file.problems());
        assert!(!file.is_writable());
        // Changes act until the program ends.
        assert!(file.set(Setting::TabWidth(2)));
        assert_eq!(file.settings().tab_width, 2);
        assert_eq!(file.to_toml(), None);
        let dir = TempDir::new("settings-broken");
        let path = dir.0.join("settings.toml");
        fs::write(&path, "[editor\n").unwrap();
        read(&path).write(&path).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "[editor\n");
    }

    #[test]
    fn writing_keeps_what_the_user_wrote() {
        let text = "\
# My settings.
theme = \"dark\"   # at night too

[editor]
# Wide tabs.
tab_width = 4
my_own_key = \"kept\"

[plugins]
unknown = true
";
        let mut file = parse(text);
        assert!(file.problems().is_empty());
        file.set(Setting::Theme(Theme::Light));
        file.set(Setting::TabWidth(2));
        file.set(Setting::FontSize(15));
        file.set(Setting::WordWrap(false));
        let written = file.to_toml().unwrap();
        assert_eq!(
            written,
            "\
# My settings.
theme = \"light\"   # at night too

[editor]
# Wide tabs.
tab_width = 2
my_own_key = \"kept\"
font_size = 15

[plugins]
unknown = true

[view]
word_wrap = false
"
        );
        // Dotted keys and inline tables are settings too, and are changed where they are.
        let mut file = parse("editor.tab_width = 4\nview = { invisibles = true }\n");
        assert_eq!((file.settings().tab_width, file.settings().invisibles), (4, true));
        file.set(Setting::TabWidth(3));
        file.set(Setting::Invisibles(false));
        assert_eq!(file.to_toml().unwrap(), "editor.tab_width = 3\nview = { invisibles = false }\n");
    }

    #[test]
    fn the_file_is_written_whole_and_through_links() {
        let dir = TempDir::new("settings-write");
        let path = dir.0.join("settings.toml");
        let mut file = read(&path);
        file.set(Setting::TabWidth(2));
        file.write(&path).unwrap();
        assert_eq!(read(&path).settings().tab_width, 2);
        assert!(fs::read_to_string(&path).unwrap().starts_with("# The settings of MigPad."));
        #[cfg(unix)]
        {
            // A file kept with other dotfiles, linked from the folder of the data, stays linked.
            let dotfiles = dir.0.join("dotfiles");
            fs::create_dir(&dotfiles).unwrap();
            let kept = dotfiles.join("settings.toml");
            fs::write(&kept, "theme = \"dark\"\n").unwrap();
            let link = dir.0.join("linked.toml");
            std::os::unix::fs::symlink(&kept, &link).unwrap();
            let mut file = read(&link);
            file.set(Setting::TabWidth(3));
            file.write(&link).unwrap();
            assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
            assert_eq!(fs::read_to_string(&kept).unwrap(), "theme = \"dark\"\n\n[editor]\ntab_width = 3\n");
        }
    }

    #[test]
    fn an_unreadable_file_is_not_written_over() {
        let dir = TempDir::new("settings-unreadable");
        let path = dir.0.join("settings.toml");
        fs::write(&path, b"theme = \"dark\xff\"\n").unwrap();
        let file = read(&path);
        assert!(matches!(file.problems(), [Problem::Unreadable(_)]));
        assert!(!file.is_writable());
    }
}
