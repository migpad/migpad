//! Strings of the interface in its language: English or Russian, as the settings choose — by
//! default Russian on a Russian system. The tables are `locales/*.toml`, which `build.rs` turns
//! into code; a translation the user put in the folder of the data, `locales/<language>.toml`, is
//! read as MigPad starts and takes the place of the strings it has.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::table::{self, Entry, FormsError};

include!(concat!(env!("OUT_DIR"), "/strings.rs"));

/// A language of the interface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    English,
    Russian,
}

impl Language {
    /// Its code, the name of its table: `en`, `ru`.
    pub fn code(self) -> &'static str {
        match self {
            Language::English => "en",
            Language::Russian => "ru",
        }
    }

    /// The language of the system: Russian if the language the user prefers first is Russian,
    /// English otherwise.
    pub fn of_system() -> Language {
        Self::of_locale(&sys_locale::get_locale().unwrap_or_default())
    }

    /// The language for a locale such as `ru`, `ru-RU` or `en_US`.
    fn of_locale(locale: &str) -> Language {
        let language = locale.split(['-', '_']).next().unwrap_or_default();
        if language.eq_ignore_ascii_case("ru") { Language::Russian } else { Language::English }
    }

    /// The language of the interface now.
    pub fn current() -> Language {
        if RUSSIAN.load(Ordering::Relaxed) { Language::Russian } else { Language::English }
    }
}

/// Whether the interface is in Russian rather than English.
static RUSSIAN: AtomicBool = AtomicBool::new(false);

/// Held by tests that change the language, as tests run at the same time.
#[cfg(test)]
pub static LANGUAGE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Shows the interface in `language` from now on; menus built before keep their strings.
pub fn set_language(language: Language) {
    RUSSIAN.store(language == Language::Russian, Ordering::Relaxed);
}

/// The string of `key` in the language of the interface, without the mark of its mnemonic.
pub fn tr(key: Key) -> &'static str {
    string(key, Language::current())
}

/// Where the mnemonic of the string of `key` is, in bytes of [`tr`]: the letter that chooses it
/// in the menus of Windows and Linux.
pub fn mnemonic(key: Key) -> Option<usize> {
    let language = Language::current();
    if let Some((_, mnemonic)) = user(language).and_then(|user| user.texts[key as usize]) {
        return mnemonic;
    }
    match language {
        Language::English => EN_MNEMONICS[key as usize],
        Language::Russian => RU_MNEMONICS[key as usize],
    }
}

/// The string of `key` for the number `n`, in the form the language has for it, with `{count}`
/// filled with `n` and the other placeholders with `values`: 1 line, 2 lines; 1 строка, 2 строки,
/// 5 строк.
pub fn plural(key: Plural, n: u64, values: &[(&str, &str)]) -> String {
    let language = Language::current();
    let form = form(language, n);
    let template = match user(language).and_then(|user| user.plurals[key as usize].as_ref()) {
        Some(forms) => forms[form],
        None => match language {
            Language::English => EN_PLURALS[key as usize][form],
            Language::Russian => RU_PLURALS[key as usize][form],
        },
    };
    let count = number(n);
    let values: Vec<(&str, &str)> = [("count", count.as_str())].into_iter().chain(values.iter().copied()).collect();
    fill_in(template, &values)
}

/// The form a number takes in `language`, by its place among [`table::forms`]: in Russian one for
/// 1, 21, 101, few for 2–4, 22–24, many for the rest — 5–20, 25–30, 111–114; in English one for 1
/// and other for the rest.
fn form(language: Language, n: u64) -> usize {
    match language {
        Language::English => usize::from(n != 1),
        Language::Russian => match (n % 10, n % 100) {
            (1, rest) if rest != 11 => 0,
            (2..=4, rest) if !(12..=14).contains(&rest) => 1,
            _ => 2,
        },
    }
}

/// The strings of a translation the user put in the folder of the data, by key; `None` where it
/// has none, or one MigPad cannot take.
struct UserStrings {
    texts: Vec<Option<(&'static str, Option<usize>)>>,
    plurals: Vec<Option<Vec<&'static str>>>,
}

static USER: [OnceLock<UserStrings>; 2] = [OnceLock::new(), OnceLock::new()];

fn user(language: Language) -> Option<&'static UserStrings> {
    USER[language as usize].get()
}

/// Why a string of a translation is not taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unfit {
    /// MigPad has no string with its key.
    UnknownKey,
    /// Its placeholders are not those of the string in English.
    Placeholders,
    /// A string where MigPad has the forms of a number, the forms where it has a string, or other
    /// forms than the language has.
    Forms,
    /// A "&" before nothing, or a second mnemonic.
    StrayAmpersand,
}

/// Takes the translation `text` for `language` in place of the strings built in, as MigPad starts:
/// once for each language. Returns the strings not taken and why, or why the text is not a table
/// of strings.
pub fn use_translation(language: Language, text: &str) -> Result<Vec<(String, Unfit)>, String> {
    let (strings, unfit) = translation(language, text)?;
    // The strings live as long as the program: a table read once.
    let _ = USER[language as usize].set(strings);
    Ok(unfit)
}

/// The strings of the translation `text` for `language` that MigPad can take, and those it cannot.
fn translation(language: Language, text: &str) -> Result<(UserStrings, Vec<(String, Unfit)>), String> {
    let entries = table::parse(text)?;
    let mut strings = UserStrings { texts: vec![None; KEYS.len()], plurals: vec![None; PLURAL_KEYS.len()] };
    let mut unfit = Vec::new();
    for (key, entry) in entries {
        let text_at = KEYS.iter().position(|name| *name == key);
        let plural_at = PLURAL_KEYS.iter().position(|name| *name == key);
        let taken = match (entry, text_at, plural_at) {
            (Entry::Text(string), Some(at), _) => {
                if table::placeholders(&string) != table::placeholders(EN[at]) {
                    Err(Unfit::Placeholders)
                } else {
                    match table::mnemonic(&string) {
                        Some((plain, mnemonic)) => {
                            strings.texts[at] = Some((Box::leak(plain.into_boxed_str()), mnemonic));
                            Ok(())
                        }
                        None => Err(Unfit::StrayAmpersand),
                    }
                }
            }
            (Entry::Plural(forms), _, Some(at)) => {
                let base = table::placeholders(EN_PLURALS[at][0]);
                match table::plural_forms(language.code(), &forms, &base) {
                    Ok(ordered) => match ordered.iter().map(|form| table::mnemonic(form)).collect::<Option<Vec<_>>>() {
                        Some(plain) if plain.iter().all(|(_, mnemonic)| mnemonic.is_none()) => {
                            let leaked = plain.into_iter().map(|(form, _)| &*Box::leak(form.into_boxed_str()));
                            strings.plurals[at] = Some(leaked.collect());
                            Ok(())
                        }
                        _ => Err(Unfit::StrayAmpersand),
                    },
                    Err(FormsError::Placeholders) => Err(Unfit::Placeholders),
                    Err(_) => Err(Unfit::Forms),
                }
            }
            (_, None, None) => Err(Unfit::UnknownKey),
            _ => Err(Unfit::Forms),
        };
        if let Err(why) = taken {
            unfit.push((key, why));
        }
    }
    Ok((strings, unfit))
}

/// The string of `key` with its placeholders filled: `("count", "3")` puts 3 for `{count}`.
pub fn fill(key: Key, values: &[(&str, &str)]) -> String {
    fill_in(tr(key), values)
}

fn fill_in(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}').map(|close| open + close) else { break };
        out.push_str(&rest[..open]);
        let name = &rest[open + 1..close];
        match values.iter().find(|(key, _)| *key == name) {
            Some((_, value)) => out.push_str(value),
            None => out.push_str(&rest[open..=close]),
        }
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out
}

/// A number with its digits grouped by threes as the language does: 1,234,567 or 1 234 567.
pub fn number(n: u64) -> String {
    group(n, Language::current())
}

/// A share as percent, as the language writes it: 42% or 42 %.
pub fn percent(n: u64) -> String {
    match Language::current() {
        Language::English => format!("{n}%"),
        Language::Russian => format!("{n}\u{a0}%"),
    }
}

fn group(n: u64, language: Language) -> String {
    let separator = match language {
        Language::English => ',',
        // A narrow no-break space, as Russian typography has it.
        Language::Russian => '\u{202f}',
    };
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 * 3);
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(separator);
        }
        out.push(digit);
    }
    out
}

fn string(key: Key, language: Language) -> &'static str {
    if let Some((string, _)) = user(language).and_then(|user| user.texts[key as usize]) {
        return string;
    }
    match language {
        Language::English => EN[key as usize],
        Language::Russian => RU[key as usize],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn russian_locales_show_russian() {
        for locale in ["ru", "ru-RU", "ru_UA", "RU"] {
            assert_eq!(Language::of_locale(locale), Language::Russian, "{locale}");
        }
        for locale in ["en-US", "uk-UA", "rus", ""] {
            assert_eq!(Language::of_locale(locale), Language::English, "{locale}");
        }
    }

    #[test]
    fn strings_come_from_the_tables_without_marks() {
        assert_eq!(string(Key::EditUndo, Language::English), "Undo");
        assert_eq!(string(Key::EditUndo, Language::Russian), "Отменить");
        assert_eq!(string(Key::EditSelectAll, Language::Russian), "Выбрать все");
        assert_eq!(string(Key::EditCut, Language::English), "Cut");
        assert_eq!(EN_MNEMONICS[Key::EditCut as usize], Some(2));
        // "Вы&резать": two letters of two bytes each before it.
        assert_eq!(RU_MNEMONICS[Key::EditCut as usize], Some(4));
        assert_eq!(EN_MNEMONICS[Key::AppQuit as usize], None);
    }

    #[test]
    fn placeholders_take_their_values() {
        assert_eq!(fill_in("Ln {line}, Col {column}", &[("column", "5"), ("line", "12")]), "Ln 12, Col 5");
        assert_eq!(fill_in("{a}{a} {b}", &[("a", "x")]), "xx {b}");
        assert_eq!(fill_in("no {end", &[("end", "x")]), "no {end");
    }

    #[test]
    fn numbers_take_the_forms_of_the_language() {
        let russian = |n| form(Language::Russian, n);
        // One: 1, 21, 101; few: 2–4, 22–24; many: 0, 5–20, 25–30, 111–114.
        for n in [1, 21, 31, 101, 1001] {
            assert_eq!(russian(n), 0, "{n}");
        }
        for n in [2, 3, 4, 22, 24, 102, 1004] {
            assert_eq!(russian(n), 1, "{n}");
        }
        for n in [0, 5, 11, 12, 14, 15, 20, 25, 100, 111, 112, 114] {
            assert_eq!(russian(n), 2, "{n}");
        }
        assert_eq!([0, 1, 2, 21].map(|n| form(Language::English, n)), [1, 0, 1, 1]);
        let _lock = LANGUAGE_LOCK.lock();
        set_language(Language::Russian);
        assert_eq!(plural(Plural::SettingsTabWidthUnit, 4, &[]), "пробела");
        assert_eq!(plural(Plural::SettingsTabWidthUnit, 8, &[]), "пробелов");
        set_language(Language::English);
        assert_eq!(plural(Plural::SettingsTabWidthUnit, 1, &[]), "space");
        assert_eq!(plural(Plural::SettingsTabWidthUnit, 8, &[]), "spaces");
    }

    #[test]
    fn a_translation_takes_the_place_of_the_strings_it_has() {
        let text = r#"
[edit]
undo = "&Back"
redo = "Again {count}"
cut = "&Cu&t"
mine = "Mine"

[settings]
tab_width_unit = { one = "tab", other = "tabs" }
"#;
        let (strings, unfit) = translation(Language::English, text).unwrap();
        assert_eq!(strings.texts[Key::EditUndo as usize], Some(("Back", Some(0))));
        assert_eq!(strings.texts[Key::EditRedo as usize], None);
        assert_eq!(strings.texts[Key::EditCopy as usize], None, "what it has not stays built in");
        assert_eq!(strings.plurals[Plural::SettingsTabWidthUnit as usize], Some(vec!["tab", "tabs"]));
        assert_eq!(
            unfit,
            [
                ("edit.redo".to_owned(), Unfit::Placeholders),
                ("edit.cut".to_owned(), Unfit::StrayAmpersand),
                ("edit.mine".to_owned(), Unfit::UnknownKey),
            ]
        );
        // The forms are those of the language: Russian has three.
        let text = "[settings]\ntab_width_unit = { one = \"таб\", other = \"табов\" }\nlanguage = { one = \"a\", few = \"b\", many = \"c\" }\n";
        let (strings, unfit) = translation(Language::Russian, text).unwrap();
        assert_eq!(strings.plurals[Plural::SettingsTabWidthUnit as usize], None);
        assert_eq!(
            unfit,
            [("settings.tab_width_unit".to_owned(), Unfit::Forms), ("settings.language".to_owned(), Unfit::Forms)]
        );
        assert!(translation(Language::Russian, "[edit\n").is_err());
    }

    #[test]
    fn digits_are_grouped_by_language() {
        assert_eq!(group(0, Language::English), "0");
        assert_eq!(group(999, Language::English), "999");
        assert_eq!(group(1000, Language::English), "1,000");
        assert_eq!(group(1234567, Language::English), "1,234,567");
        assert_eq!(group(1234567, Language::Russian), "1\u{202f}234\u{202f}567");
    }
}
