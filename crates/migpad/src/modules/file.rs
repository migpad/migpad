//! Documents and windows: a new tab or window, opening and saving files, the recent files, closing
//! tabs and windows, and opening closed ones again.

use std::path::PathBuf;

use gpui::{App, actions};

use crate::commands::{Command, MenuId, Module, Registry, SubItem, by_os};
use crate::recent;
use crate::strings::Key;
use crate::windows;
use crate::workspace::{Workspace, open_options};

actions!(file, [NewTab, NewWindow, Open, Save, SaveAs, ReopenClosed, CloseTab, CloseWindow, ClearRecent]);

/// Opens a recent file, in the encoding it had and with the selection where it was.
#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = file, no_json)]
pub struct OpenRecent(pub PathBuf);

/// Opens again the tab or the window closed this many before the last: 0 is the last.
#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = file, no_json)]
pub struct ReopenClosedAt(pub usize);

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
        let open = Command::new("file.open", Key::FileOpen, Open).keys(by_os(&["cmd-o"], &["ctrl-o"], &["ctrl-o"]));
        registry.add(open, menu(1));
        registry.add(Command::new("file.open_recent", Key::FileRecent, OpenRecent(PathBuf::new())), None);
        registry.add(Command::new("file.clear_recent", Key::FileClearRecent, ClearRecent), None);
        registry.add_submenu(Key::FileRecent, recent_items, MenuId::File, 1);
        let reopen = Command::new("file.reopen_closed", Key::FileReopenClosed, ReopenClosed).keys(by_os(
            &["cmd-shift-t"],
            &["ctrl-shift-t"],
            &["ctrl-shift-t"],
        ));
        registry.add(reopen, menu(1));
        registry.add(Command::new("file.reopen_closed_at", Key::FileRecentlyClosed, ReopenClosedAt(0)), None);
        registry.add_submenu(Key::FileRecentlyClosed, closed_items, MenuId::File, 1);
        // A file still loading cannot be saved: its beginning would replace it.
        let save = Command::new("file.save", Key::FileSave, Save)
            .keys(by_os(&["cmd-s"], &["ctrl-s"], &["ctrl-s"]))
            .enabled(|workspace, cx| !workspace.document().read(cx).is_preview());
        registry.add(save, menu(2));
        let save_as = Command::new("file.save_as", Key::FileSaveAs, SaveAs)
            .keys(by_os(&["cmd-shift-s"], &["ctrl-shift-s"], &["ctrl-shift-s"]))
            .enabled(|workspace, cx| !workspace.document().read(cx).is_preview());
        registry.add(save_as, menu(2));
        let close_tab = Command::new("file.close_tab", Key::FileCloseTab, CloseTab).keys(by_os(
            &["cmd-w"],
            &["ctrl-w", "ctrl-f4"],
            &["ctrl-w"],
        ));
        registry.add(close_tab, menu(3));
        let close_window = Command::new("file.close_window", Key::FileCloseWindow, CloseWindow).keys(by_os(
            &["cmd-shift-w"],
            &["ctrl-shift-w"],
            &["ctrl-shift-w"],
        ));
        registry.add(close_window, menu(3));

        registry.on_window_action(|workspace, _: &NewTab, window, cx| workspace.new_tab(window, cx));
        registry.on_window_action(|workspace, _: &Open, window, cx| workspace.open_dialog(window, cx));
        registry.on_window_action(|workspace, _: &Save, window, cx| {
            if !workspace.document().read(cx).is_preview() {
                workspace.save_active(window, cx);
            }
        });
        registry.on_window_action(|workspace, _: &SaveAs, window, cx| {
            if !workspace.document().read(cx).is_preview() {
                workspace.save_as_active(window, cx);
            }
        });
        registry.on_window_action(|workspace, _: &ReopenClosed, window, cx| workspace.reopen_closed(0, window, cx));
        registry
            .on_window_action(|workspace, at: &ReopenClosedAt, window, cx| workspace.reopen_closed(at.0, window, cx));
        registry
            .on_window_action(|workspace, file: &OpenRecent, window, cx| workspace.open_recent(&file.0, window, cx));
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
        registry.on_app_action(|_: &ReopenClosed, cx| cx.defer(|cx| windows::reopen_in_new_window(0, cx)));
        registry.on_app_action(|at: &ReopenClosedAt, cx| {
            let index = at.0;
            cx.defer(move |cx| windows::reopen_in_new_window(index, cx));
        });
        registry.on_app_action(|file: &OpenRecent, cx| {
            let path = file.0.clone();
            cx.defer(move |cx| windows::open_recent_in_new_window(&path, cx));
        });
        registry.on_app_action(|_: &ClearRecent, cx| recent::clear(cx));
        registry.on_app_action(|_: &Open, cx| {
            let chosen = cx.prompt_for_paths(open_options());
            cx.spawn(async move |cx| {
                if let Ok(Ok(Some(paths))) = chosen.await {
                    cx.update(|cx| windows::open_window(&paths, cx));
                }
            })
            .detach();
        });
    }
}

fn open_window(cx: &mut App) {
    windows::open_window(&[], cx);
}

/// File ▸ Recent Files: the files, the last first, and Clear List.
fn recent_items(_: Option<&Workspace>, cx: &App) -> Vec<SubItem> {
    let files = recent::files(cx);
    if files.is_empty() {
        return vec![SubItem::Empty(Key::FileNoRecent)];
    }
    let mut items: Vec<SubItem> = files
        .into_iter()
        .enumerate()
        .map(|(i, file)| SubItem::Listed {
            label: recent::label(&file.path),
            number: i + 1,
            action: Box::new(OpenRecent(file.path)),
        })
        .collect();
    items.push(SubItem::Separator);
    items.push(SubItem::Command { label: Key::FileClearRecent, action: Box::new(ClearRecent) });
    items
}

/// File ▸ Recently Closed: the tabs and windows closed lately, the last first.
fn closed_items(_: Option<&Workspace>, cx: &App) -> Vec<SubItem> {
    let labels = windows::closed_labels(cx);
    if labels.is_empty() {
        return vec![SubItem::Empty(Key::FileNoClosed)];
    }
    let item = |(i, label)| SubItem::Listed { label, number: i + 1, action: Box::new(ReopenClosedAt(i)) };
    labels.into_iter().enumerate().map(item).collect()
}
