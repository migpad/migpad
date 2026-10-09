//! What the system draws for MigPad — title bars, menus, its windows on macOS — light or dark as
//! the theme is: a theme chosen in the settings is that of these parts too, not only of what
//! MigPad draws.

use migpad_ui::ThemeMode;

/// Makes what the system draws light or dark as `mode` is, or as the system is.
#[cfg(target_os = "macos")]
pub fn set(mode: ThemeMode) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSAppearance, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSApplication};

    let Some(mtm) = MainThreadMarker::new() else { return };
    // SAFETY: names of AppKit, constant for the life of the program.
    let name = match mode {
        ThemeMode::System => None,
        ThemeMode::Light => Some(unsafe { NSAppearanceNameAqua }),
        ThemeMode::Dark => Some(unsafe { NSAppearanceNameDarkAqua }),
    };
    let appearance = name.and_then(NSAppearance::appearanceNamed);
    NSApplication::sharedApplication(mtm).setAppearance(appearance.as_deref());
}

/// Windows and Linux: GPUI draws the title bars of the windows of MigPad there.
#[cfg(not(target_os = "macos"))]
pub fn set(_: ThemeMode) {}
