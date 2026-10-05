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
}

/// Whether the interface is in Russian rather than English.
static RUSSIAN: AtomicBool = AtomicBool::new(false);

/// Shows the interface in `language` from now on; menus built before keep their strings.
pub fn set_language(language: Language) {
    RUSSIAN.store(language == Language::Russian, Ordering::Relaxed);
}

/// The string of `key` in the language of the interface.
pub fn tr(key: Key) -> &'static str {
    string(key, if RUSSIAN.load(Ordering::Relaxed) { Language::Russian } else { Language::English })
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
    fn strings_come_from_the_tables() {
        assert_eq!(string(Key::EditUndo, Language::English), "Undo");
        assert_eq!(string(Key::EditUndo, Language::Russian), "Отменить");
        assert_eq!(string(Key::EditSelectAll, Language::Russian), "Выбрать все");
    }
}
