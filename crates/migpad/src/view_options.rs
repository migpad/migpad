//! How the windows show, as the View menu leaves it, kept between starts in `state/view.toml`: on
//! macOS, the menu bar MigPad draws in each window, which Windows and Linux always have.

use gpui::{App, Global, WindowHandle};
use migpad_core::state::view::ViewState;

use crate::commands::refresh_menus;
use crate::journals;
use crate::session;
use crate::windows;
use crate::workspace::Workspace;

#[derive(Default)]
struct View(ViewState);

impl Global for View {}

/// Reads how the windows showed the last time.
pub fn init(cx: &mut App) {
    let path = journals::data(cx).map(|data| data.state().join("view.toml"));
    let view = path.and_then(|path| {
        ViewState::read(&path).unwrap_or_else(|error| {
            eprintln!("MigPad could not read how its windows show: {error}");
            None
        })
    });
    cx.set_global(View(view.unwrap_or_default()));
}

/// Whether the windows have the menu bar MigPad draws: on Windows and Linux, where GPUI makes none,
/// always; on macOS, if View ▸ Menu Bar in Window is on. A debug build shows it on macOS with
/// `MIGPAD_MENU_BAR=1` too.
pub fn menu_bar(cx: &App) -> bool {
    !cfg!(target_os = "macos") || debug_menu_bar() || cx.try_global::<View>().is_some_and(|view| view.0.menu_bar)
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
    let view = &mut cx.default_global::<View>().0;
    view.menu_bar = !view.menu_bar;
    let shown = menu_bar(cx);
    let windows: Vec<WindowHandle<Workspace>> = windows::workspaces(cx).map(|(window, _)| window).collect();
    for window in windows {
        let _ = window.update(cx, |workspace, window, cx| workspace.set_menu_bar(shown, window, cx));
    }
    session::changed(cx);
    refresh_menus(cx);
}

/// How the windows show, as the file of the state keeps it.
pub fn to_toml(cx: &App) -> String {
    cx.try_global::<View>().map_or_else(ViewState::default, |view| view.0).to_toml()
}
