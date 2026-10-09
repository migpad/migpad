//! The settings window: the settings of MigPad as fields, each change in effect at once and written
//! to `settings.toml` ([ADR 0011]); there is no OK button. On macOS it is a window of the system, as
//! the settings of the applications there are ([ADR 0014]); on Windows and Linux MigPad draws it. A
//! debug build on macOS opens the one MigPad draws with `MIGPAD_OWN_SETTINGS=1`, to check it.

use gpui::App;
use migpad_core::settings::{Language, Theme};
use migpad_editor::system_font;

use crate::strings::{Key, fill, tr};

/// The languages to choose from, in their order in the window.
pub const LANGUAGES: [Language; 3] = [Language::System, Language::English, Language::Russian];
/// The themes to choose from, in their order in the window.
pub const THEMES: [Theme; 3] = [Theme::System, Theme::Light, Theme::Dark];

/// Opens the settings window, or brings it forward.
pub fn open(cx: &mut App) {
    #[cfg(target_os = "macos")]
    if !own_window() {
        crate::native_settings::show(cx);
        return;
    }
    crate::own_settings::show(cx);
}

/// The settings changed — in the window, or elsewhere: the window shows them, in the language of
/// the interface now.
pub fn refresh(cx: &mut App) {
    #[cfg(target_os = "macos")]
    crate::native_settings::refresh(cx);
    crate::own_settings::refresh(cx);
}

/// Whether a debug build on macOS opens the window MigPad draws, with `MIGPAD_OWN_SETTINGS=1`.
#[cfg(target_os = "macos")]
fn own_window() -> bool {
    cfg!(debug_assertions) && std::env::var_os("MIGPAD_OWN_SETTINGS").is_some()
}

/// The title of the window: "Settings" on macOS, "Параметры" in Russian on Windows and Linux, as
/// the systems call such windows.
pub fn title() -> &'static str {
    tr(if cfg!(target_os = "macos") { Key::SettingsTitleMacos } else { Key::SettingsTitle })
}

/// What the window calls a language: each by its own name.
pub fn language_label(language: Language) -> String {
    match language {
        Language::System => tr(Key::SettingsSystem).to_owned(),
        Language::English => "English".to_owned(),
        Language::Russian => "Русский".to_owned(),
    }
}

pub fn theme_label(theme: Theme) -> &'static str {
    tr(match theme {
        Theme::System => Key::SettingsSystem,
        Theme::Light => Key::SettingsLight,
        Theme::Dark => Key::SettingsDark,
    })
}

/// What the window calls a font: the font of the system with its name.
pub fn font_label(font: Option<&str>) -> String {
    match font {
        Some(font) => font.to_owned(),
        None => fill(Key::SettingsSystemFont, &[("font", system_font())]),
    }
}
