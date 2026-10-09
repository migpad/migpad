//! The menu bar MigPad draws in each window: Windows and Linux always have it, macOS if View ▸ Menu
//! Bar in Window is on — a setting.

use gpui::{App, WindowHandle};
use migpad_core::settings::Setting;

use crate::settings;
use crate::windows;
use crate::workspace::Workspace;

/// Whether the windows have the menu bar MigPad draws: on Windows and Linux, where GPUI makes none,
/// always; on macOS, if View ▸ Menu Bar in Window is on. A debug build shows it on macOS with
/// `MIGPAD_MENU_BAR=1` too.
pub fn menu_bar(cx: &App) -> bool {
    !cfg!(target_os = "macos") || debug_menu_bar() || settings::get(cx).menu_bar
}

/// Whether the keys of the menu bar MigPad draws are on: Alt alone, Alt with a mnemonic, F10. Not
/// on macOS, where Option types characters; a debug build tries them there with `MIGPAD_MENU_BAR=1`.
pub fn menu_keys() -> bool {
    !cfg!(target_os = "macos") || debug_menu_bar()
}

fn debug_menu_bar() -> bool {
    cfg!(debug_assertions) && std::env::var_os("MIGPAD_MENU_BAR").is_some()
}

/// View ▸ Menu Bar in Window on macOS: shows the menu bar in each window, or hides it.
pub fn toggle_menu_bar_in_window(cx: &mut App) {
    let shown = settings::get(cx).menu_bar;
    settings::set(Setting::MenuBar(!shown), cx);
}

/// The setting of the menu bar changed: each window shows it, or hides it.
pub fn menu_bar_changed(cx: &mut App) {
    let shown = menu_bar(cx);
    let windows: Vec<WindowHandle<Workspace>> = windows::workspaces(cx).map(|(window, _)| window).collect();
    for window in windows {
        let _ = window.update(cx, |workspace, window, cx| workspace.set_menu_bar(shown, window, cx));
    }
}
