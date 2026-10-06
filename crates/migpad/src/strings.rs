//! Strings of the interface in its language: English, or Russian on a Russian system. The tables
//! are `locales/*.toml`, which `build.rs` turns into code.

use std::sync::atomic::{AtomicBool, Ordering};

include!(concat!(env!("OUT_DIR"), "/strings.rs"));

/// A language of the interface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    English,
    Russian,
}

impl Language {
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
    fn current() -> Language {
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
    match Language::current() {
        Language::English => EN_MNEMONICS[key as usize],
        Language::Russian => RU_MNEMONICS[key as usize],
    }
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
    fn digits_are_grouped_by_language() {
        assert_eq!(group(0, Language::English), "0");
        assert_eq!(group(999, Language::English), "999");
        assert_eq!(group(1000, Language::English), "1,000");
        assert_eq!(group(1234567, Language::English), "1,234,567");
        assert_eq!(group(1234567, Language::Russian), "1\u{202f}234\u{202f}567");
    }
}
