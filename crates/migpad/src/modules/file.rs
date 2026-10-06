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
        // from any of them. The window opens once the action is handled: the window it came from
        // is being updated until then, and its place and tabs cannot be read.
        registry.on_app_action(|_: &NewTab, cx| cx.defer(open_window));
        registry.on_app_action(|_: &NewWindow, cx| cx.defer(open_window));
        registry.on_app_action(|_: &ReopenClosed, cx| cx.defer(windows::reopen_in_new_window));
    }
}

fn open_window(cx: &mut App) {
    windows::open_window(&[], cx);
}
