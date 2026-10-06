//! Documents and windows: a new tab or window, closing them, and opening a closed tab again.

use gpui::{App, actions};

use crate::commands::{Command, MenuId, Module, Registry, by_os};
use crate::strings::Key;
use crate::windows;

actions!(file, [NewTab, NewWindow, ReopenClosed, CloseTab, CloseWindow]);

pub struct FileModule;

impl Module for FileModule {
    fn id(&self) -> &'static str {
        "file"
    }

    fn register(&self, registry: &mut Registry) {
        let menu = |group| Some((MenuId::File, group));
        let new = Command::new("file.new", Key::FileNew, NewTab).keys(by_os(&["cmd-n"], &["ctrl-n"], &["ctrl-n"]));
        registry.add(new, menu(0));
        registry.add_to_toolbar("file.new", 0);
        let new_window = Command::new("file.new_window", Key::FileNewWindow, NewWindow).keys(by_os(
            &["cmd-shift-n"],
            &["ctrl-shift-n"],
            &["ctrl-shift-n"],
        ));
        registry.add(new_window, menu(0));
        let reopen = Command::new("file.reopen_closed", Key::FileReopenClosed, ReopenClosed).keys(by_os(
            &["cmd-shift-t"],
            &["ctrl-shift-t"],
            &["ctrl-shift-t"],
        ));
        registry.add(reopen, menu(1));
        let close_tab = Command::new("file.close_tab", Key::FileCloseTab, CloseTab).keys(by_os(
            &["cmd-w"],
            &["ctrl-w", "ctrl-f4"],
            &["ctrl-w"],
        ));
        registry.add(close_tab, menu(2));
        let close_window = Command::new("file.close_window", Key::FileCloseWindow, CloseWindow).keys(by_os(
            &["cmd-shift-w"],
            &["ctrl-shift-w"],
            &["ctrl-shift-w"],
        ));
        registry.add(close_window, menu(2));

        registry.on_window_action(|workspace, _: &NewTab, window, cx| workspace.new_tab(window, cx));
        registry.on_window_action(|workspace, _: &ReopenClosed, window, cx| workspace.reopen_closed(window, cx));
        registry.on_window_action(|workspace, _: &CloseTab, window, cx| {
            let active = workspace.active_tab();
            workspace.close_tab(active, window, cx);
        });
        registry.on_window_action(|workspace, _: &CloseWindow, window, cx| workspace.close_window(window, cx));
        // Without a window — on macOS, where MigPad stays open without them — and for a new window
        // from any of them.
        registry.on_app_action(|_: &NewTab, cx| open_window(cx));
        registry.on_app_action(|_: &NewWindow, cx| open_window(cx));
        registry.on_app_action(|_: &ReopenClosed, cx| {
            if !windows::has_closed(cx) {
                return;
            }
            if let Some(window) = windows::open_window(&[], cx) {
                // The tab comes in place of the untitled one the new window has.
                let _ = window.update(cx, |workspace, window, cx| {
                    workspace.reopen_closed(window, cx);
                    workspace.drop_first_tab(window, cx);
                });
            }
        });
    }
}

fn open_window(cx: &mut App) {
    windows::open_window(&[], cx);
}
