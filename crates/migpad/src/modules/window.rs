//! The window and its tabs: switching tabs; minimizing and zooming the window and the list of
//! windows in the Window menu of macOS.

use gpui::actions;

use crate::commands::{ActivateWindow, Command, MenuId, Module, Registry, by_os};
use crate::strings::Key;
use crate::view_options;

actions!(window, [Minimize, Zoom, NextTab, PreviousTab, LastTab, ToggleMenuBar]);

/// Switches to a tab by its place from the left, from zero.
#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = window, no_json)]
pub struct SelectTab(pub usize);

/// Cmd+1…8 on macOS, Ctrl+1…8 elsewhere, as browsers have them.
const SELECT_KEYS: [[&str; 3]; 8] = [
    ["cmd-1", "ctrl-1", "ctrl-1"],
    ["cmd-2", "ctrl-2", "ctrl-2"],
    ["cmd-3", "ctrl-3", "ctrl-3"],
    ["cmd-4", "ctrl-4", "ctrl-4"],
    ["cmd-5", "ctrl-5", "ctrl-5"],
    ["cmd-6", "ctrl-6", "ctrl-6"],
    ["cmd-7", "ctrl-7", "ctrl-7"],
    ["cmd-8", "ctrl-8", "ctrl-8"],
];

/// The identifiers of the commands of Cmd+1…8.
const SELECT_IDS: [&str; 8] = [
    "window.tab_1",
    "window.tab_2",
    "window.tab_3",
    "window.tab_4",
    "window.tab_5",
    "window.tab_6",
    "window.tab_7",
    "window.tab_8",
];

pub struct WindowModule;

impl Module for WindowModule {
    fn id(&self) -> &'static str {
        "window"
    }

    fn register(&self, registry: &mut Registry) {
        let menu = |group| Some((MenuId::Window, group));
        // On Windows and Linux the title bar of the window has these.
        let macos = |group| cfg!(target_os = "macos").then_some((MenuId::Window, group));
        registry.add(
            Command::new("window.minimize", Key::WindowMinimize, Minimize).keys(by_os(&["cmd-m"], &[], &[])),
            macos(0),
        );
        registry.add(Command::new("window.zoom", Key::WindowZoom, Zoom), macos(0));
        let next = Command::new("window.next_tab", Key::WindowNextTab, NextTab).keys(by_os(
            &["cmd-}", "ctrl-tab"],
            &["ctrl-tab", "ctrl-pagedown"],
            &["ctrl-tab", "ctrl-pagedown"],
        ));
        registry.add(next, menu(1));
        let previous = Command::new("window.previous_tab", Key::WindowPreviousTab, PreviousTab).keys(by_os(
            &["cmd-{", "ctrl-shift-tab"],
            &["ctrl-shift-tab", "ctrl-pageup"],
            &["ctrl-shift-tab", "ctrl-pageup"],
        ));
        registry.add(previous, menu(1));
        for (i, (id, keys)) in SELECT_IDS.iter().zip(&SELECT_KEYS).enumerate() {
            let keys = by_os(&keys[0..1], &keys[1..2], &keys[2..3]);
            registry.add(Command::new(id, Key::WindowSelectTab, SelectTab(i)).keys(keys), None);
        }
        let last = Command::new("window.last_tab", Key::WindowSelectTab, LastTab).keys(by_os(
            &["cmd-9"],
            &["ctrl-9"],
            &["ctrl-9"],
        ));
        registry.add(last, None);
        if cfg!(target_os = "macos") {
            registry.add_window_list(MenuId::Window, 2);
        }
        // F10 brings the keyboard to the menu bar MigPad draws, as Alt pressed and released alone
        // does — the menu bar watches for that itself.
        let menu_keys: &[&str] = if view_options::menu_keys() { &["f10"] } else { &[] };
        registry.add(Command::new("window.menu_bar", Key::WindowMenu, ToggleMenuBar).keys(menu_keys), None);

        registry.on_window_action(|_, _: &Minimize, window, _| window.minimize_window());
        registry.on_window_action(|_, _: &Zoom, window, _| window.zoom_window());
        registry.on_window_action(|workspace, _: &NextTab, window, cx| workspace.activate_next(window, cx));
        registry.on_window_action(|workspace, _: &PreviousTab, window, cx| workspace.activate_previous(window, cx));
        registry
            .on_window_action(|workspace, select: &SelectTab, window, cx| workspace.select(Some(select.0), window, cx));
        registry.on_window_action(|workspace, _: &LastTab, window, cx| workspace.select(None, window, cx));
        registry.on_window_action(|workspace, _: &ToggleMenuBar, window, cx| workspace.toggle_menu_bar(window, cx));
        registry.on_app_action(|action: &ActivateWindow, cx| {
            if let Some(window) = cx.windows().into_iter().find(|window| window.window_id() == action.0) {
                let _ = window.update(cx, |_, window, _| window.activate_window());
            }
        });
    }
}
