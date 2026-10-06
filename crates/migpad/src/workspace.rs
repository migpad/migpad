//! The root of a window: its tabs, each a document and its view, and the commands of the window,
//! which act on the window or on the document of the active tab whatever has the focus.

use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui::{
    App, AppContext, Context, DragMoveEvent, Entity, EntityId, ExternalPaths, FocusHandle, Focusable, MouseDownEvent,
    MouseUpEvent, PathPromptOptions, Render, ScrollHandle, SharedString, Subscription, Task, Window, WindowId, canvas,
    div, prelude::*, px, rgb,
};
use migpad_core::document::{Document, Format};
use migpad_core::encoding::Encoding;
use migpad_core::history::Selection;
use migpad_editor::EditorView;
use migpad_ui::notification::NotificationBar;
use migpad_ui::{Button, MenuBar, TabBar, TabInfo, theme};

use crate::commands::{Registry, own_menu_bar, update_menus};
use crate::keys;
use crate::modules::file::NewTab;
use crate::notices::{self, Notice, NoticeAction, Topic};
use crate::status::{self, COUNT_STEP, Loading, SelectionCount, count_chars};
use crate::strings::{Key, tr};
use crate::tabs::Tabs;
use crate::windows::{self, Closed, ClosedTab, ForTab, Opening};

/// The dialog of the system that opens files: several at once.
pub fn open_options() -> PathPromptOptions {
    PathPromptOptions { files: true, directories: false, multiple: true, prompt: Some(tr(Key::FileOpenButton).into()) }
}

/// How often the status bar shows how far a file has loaded.
const PROGRESS_TICK: Duration = Duration::from_millis(100);

/// A document in a tab of the window, and its view.
pub struct Tab {
    pub document: Entity<Document>,
    pub editor: Entity<EditorView>,
    /// The number of an untitled document: "Untitled", "Untitled 2"…
    untitled: Option<u32>,
    /// How far the file has loaded, while it is loading.
    loading: Option<Loading>,
    /// Where the view puts the selection once the file has loaded: that of a reopened tab, which
    /// the beginning shown while loading may not reach.
    pending_selection: Option<Selection>,
    /// Notifications over the text, each with its number in the window.
    notices: Vec<(u64, Notice)>,
    /// Whether the text was checked for bytes lost in reading: once it is loaded.
    losses_checked: bool,
    _observe: Subscription,
    /// Shows the progress of the loading in the status bar.
    _progress: Option<Task<()>>,
}

/// What the window shows in its title: the name of the active document, whether it has changes
/// to save, and its file.
#[derive(Clone, Default, PartialEq, Eq)]
struct Title {
    name: String,
    modified: bool,
    path: Option<PathBuf>,
}

pub struct Workspace {
    window_id: WindowId,
    /// The menu bar MigPad draws, on Windows and Linux.
    menu_bar: Option<Entity<MenuBar>>,
    tabs: Tabs<Tab>,
    tab_scroll: ScrollHandle,
    /// Whether the bar of tabs is yet to scroll to the active tab.
    reveal_tab: bool,
    title: Title,
    /// Counts the changes to the documents of the window: counts of selections go stale with them.
    revision: u64,
    selection_count: Option<SelectionCount>,
    /// Whether the window has drawn a frame: what its elements handle is known after that.
    drawn: bool,
    /// The number of the next notification.
    next_notice: u64,
    /// The files dragged over the window from another program, to open once they are dropped.
    dragged_files: Option<Vec<PathBuf>>,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    /// A window with one tab.
    pub fn new(first: ForTab, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // The check marks of the menus are those of the document of the active window.
        let activation = cx.observe_window_activation(window, |workspace, window, cx| {
            if window.is_window_active() {
                update_menus(Some(workspace), cx);
            }
        });
        // Colors follow the appearance of the system, unless a theme is chosen.
        let appearance = migpad_ui::theme::follow_system(window, cx);
        let window_id = window.window_handle().window_id();
        let untitled = Self::untitled_for(&first.document, [], window_id, cx);
        let tab = Tab::new(first.document, untitled, first.loading, window, cx);
        let menu_bar = own_menu_bar().then(|| {
            let workspace = cx.weak_entity();
            cx.new(|cx| {
                MenuBar::new(
                    move |target, window, cx| {
                        let Some(workspace) = workspace.upgrade() else { return Vec::new() };
                        cx.global::<Registry>().menu_bar(workspace.read(cx), target, window, cx)
                    },
                    window,
                    cx,
                )
            })
        });
        let mut workspace = Workspace {
            window_id,
            menu_bar,
            tabs: Tabs::new(tab),
            tab_scroll: ScrollHandle::new(),
            reveal_tab: true,
            title: Title::default(),
            revision: 0,
            selection_count: None,
            drawn: false,
            next_notice: 0,
            dragged_files: None,
            _subscriptions: vec![activation, appearance],
        };
        workspace.settle(0, first.selection, window, cx);
        workspace.update_title(window, cx);
        workspace
    }

    pub fn window_id(&self) -> WindowId {
        self.window_id
    }

    /// The view of the document that the commands act on: that of the active tab.
    pub fn editor(&self) -> &Entity<EditorView> {
        &self.tabs.active().editor
    }

    /// Whether the window has drawn a frame.
    pub fn has_drawn(&self) -> bool {
        self.drawn
    }

    /// The document of the active tab.
    pub fn document(&self) -> &Entity<Document> {
        &self.tabs.active().document
    }

    pub fn active_tab(&self) -> usize {
        self.tabs.active_index()
    }

    /// The title of the window: the name of the document of the active tab.
    pub fn title(&self) -> &str {
        &self.title.name
    }

    /// The numbers of the untitled documents of the window.
    pub fn untitled_numbers(&self) -> impl Iterator<Item = u32> + '_ {
        self.tabs.iter().filter_map(|tab| tab.untitled)
    }

    /// The tab of the file at `path`, if the window has it open.
    pub fn find(&self, path: &Path, cx: &App) -> Option<usize> {
        self.tabs
            .position(|tab| tab.document.read(cx).path.as_deref().is_some_and(|open| windows::same_file(open, path)))
    }

    /// The number of a document without a file: the lowest no untitled document of the windows has.
    fn untitled_for(
        document: &Entity<Document>,
        own: impl IntoIterator<Item = u32>,
        window: WindowId,
        cx: &App,
    ) -> Option<u32> {
        document.read(cx).path.is_none().then(|| windows::untitled_number(own, Some(window), cx))
    }

    /// Adds a tab at the end and switches to it.
    pub fn add_tab(&mut self, tab: ForTab, window: &mut Window, cx: &mut Context<Self>) {
        let numbers: Vec<u32> = self.untitled_numbers().collect();
        let untitled = Self::untitled_for(&tab.document, numbers, self.window_id, cx);
        let index = self.tabs.push(Tab::new(tab.document, untitled, tab.loading, window, cx));
        self.settle(index, tab.selection, window, cx);
        self.activate(index, window, cx);
    }

    /// Puts a new tab in order: the selection it comes with, at once or once its file has loaded,
    /// and a warning of bytes lost in reading.
    fn settle(&mut self, index: usize, selection: Option<Selection>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(selection) = selection
            && let Some(tab) = self.tabs.get_mut(index)
        {
            if tab.document.read(cx).is_preview() {
                tab.pending_selection = Some(selection);
            } else {
                tab.editor.update(cx, |editor, cx| editor.select(selection, window, cx));
            }
        }
        self.check_losses(index, cx);
    }

    /// Shows a notification over the text of the active tab.
    pub fn notify(&mut self, notice: Notice, cx: &mut Context<Self>) {
        let index = self.tabs.active_index();
        self.notify_tab(index, notice, cx);
    }

    /// Shows a notification over the text of the tab at `index`, in place of one about the same.
    fn notify_tab(&mut self, index: usize, notice: Notice, cx: &mut Context<Self>) {
        let id = self.next_notice;
        self.next_notice += 1;
        if let Some(tab) = self.tabs.get_mut(index) {
            if let Some(topic) = notice.topic {
                tab.notices.retain(|(_, old)| old.topic != Some(topic));
            }
            tab.notices.push((id, notice));
            cx.notify();
        }
    }

    /// Closes the notifications about `topic` of the tab at `index`: what they told is over.
    fn clear_notices(&mut self, index: usize, topic: Topic, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get_mut(index) {
            tab.notices.retain(|(_, notice)| notice.topic != Some(topic));
            cx.notify();
        }
    }

    /// A button of the notification `id` of the tab with `document`: what the button says is done,
    /// and the notification closes — but for showing the first place, after which the rest of its
    /// buttons are still to choose from.
    fn run_notice_action(
        &mut self,
        document: EntityId,
        id: u64,
        action: NoticeAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_of(document) else { return };
        if !matches!(action, NoticeAction::ShowFirst(_)) {
            self.close_notice(document, id, cx);
        }
        match action {
            NoticeAction::SaveInUtf8 => {
                let format = self.tabs.get(index).map(|tab| tab.document.read(cx).format);
                let format = format.map(|format| Format { encoding: Encoding::UTF_8, bom: false, ..format });
                self.save(index, format, false, window, cx);
            }
            NoticeAction::ShowFirst(pos) => {
                self.activate(index, window, cx);
                self.editor().update(cx, |editor, cx| editor.select(Selection::caret(pos), window, cx));
            }
            NoticeAction::SaveReplacing => {
                self.save(index, None, true, window, cx);
            }
            NoticeAction::SaveAs => self.save_as(index, window, cx),
        }
    }

    /// Opens files chosen in the dialog of the system into tabs.
    pub fn open_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(open_options());
        cx.spawn_in(window, async move |workspace, cx| {
            if let Ok(Ok(Some(paths))) = chosen.await {
                let _ = workspace.update_in(cx, |workspace, window, cx| workspace.open_paths(&paths, window, cx));
            }
        })
        .detach();
    }

    /// Saves the document of the active tab: to its file, or to one chosen if it has none.
    pub fn save_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let index = self.tabs.active_index();
        self.save(index, None, false, window, cx);
    }

    pub fn save_as_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let index = self.tabs.active_index();
        self.save_as(index, window, cx);
    }

    /// Saves the document of the tab at `index` to its file in `format`, or in its own format; an
    /// untitled one goes to a file chosen in the dialog of the system. With `accept_losses`
    /// characters that cannot be written are replaced; without, the file is not written and a
    /// notification tells why ([ADR 0015]). Returns whether the file was written.
    pub fn save(
        &mut self,
        index: usize,
        format: Option<Format>,
        accept_losses: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(tab) = self.tabs.get(index) else { return false };
        let Some(path) = tab.document.read(cx).path.clone() else {
            self.save_as(index, window, cx);
            return false;
        };
        self.save_to(index, path, format, accept_losses, cx)
    }

    /// Saves the document of the tab at `index` to `path`.
    fn save_to(
        &mut self,
        index: usize,
        path: PathBuf,
        format: Option<Format>,
        accept_losses: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(document) = self.tabs.get(index).map(|tab| tab.document.clone()) else { return false };
        let format = format.unwrap_or(document.read(cx).format);
        let result = document.update(cx, |doc, cx| {
            let result = doc.save(&path, format, accept_losses);
            if result.is_ok() {
                cx.notify();
            }
            result
        });
        match result {
            Ok(()) => {
                self.clear_notices(index, Topic::Save, cx);
                cx.add_recent_document(&path);
                true
            }
            Err(error) => {
                let notice = notices::save_failed(document.read(cx), &path, format.encoding.name(), &error);
                if let Some(notice) = notice {
                    self.notify_tab(index, notice, cx);
                }
                false
            }
        }
    }

    /// Saves the document of the tab at `index` to a file chosen in the dialog of the system,
    /// next to its own file or in the home folder, under its name.
    pub fn save_as(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(index) else { return };
        let document = tab.document.entity_id();
        let (folder, name) = match &tab.document.read(cx).path {
            Some(path) => {
                (path.parent().map(Path::to_path_buf), path.file_name().map(|name| name.to_string_lossy().into_owned()))
            }
            None => (None, Some(format!("{}.txt", Self::tab_title(tab, cx)))),
        };
        let folder = folder.or_else(std::env::home_dir).unwrap_or_default();
        let chosen = cx.prompt_for_new_path(&folder, name.as_deref());
        cx.spawn_in(window, async move |workspace, cx| {
            if let Ok(Ok(Some(path))) = chosen.await {
                let _ = workspace.update(cx, |workspace, cx| {
                    if let Some(index) = workspace.tab_of(document) {
                        workspace.save_to(index, path, None, false, cx);
                    }
                });
            }
        })
        .detach();
    }

    /// Shows a notification over the text of the tab with `document`.
    fn notify_document(&mut self, document: EntityId, notice: Notice, cx: &mut Context<Self>) {
        if let Some(index) = self.tab_of(document) {
            self.notify_tab(index, notice, cx);
        }
    }

    fn tab_of(&self, document: EntityId) -> Option<usize> {
        self.tabs.position(|tab| tab.document.entity_id() == document)
    }

    /// Closes the notification `id` of the tab with `document`.
    fn close_notice(&mut self, document: EntityId, id: u64, cx: &mut Context<Self>) {
        if let Some(index) = self.tab_of(document)
            && let Some(tab) = self.tabs.get_mut(index)
        {
            tab.notices.retain(|(notice, _)| *notice != id);
            cx.notify();
        }
    }

    /// Tells of bytes that could not be read in the encoding of the file of a tab, once it is loaded.
    fn check_losses(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(index) else { return };
        let doc = tab.document.read(cx);
        if tab.losses_checked || doc.is_preview() {
            return;
        }
        let notice = notices::decode_losses(doc);
        if let Some(tab) = self.tabs.get_mut(index) {
            tab.losses_checked = true;
        }
        if let Some(notice) = notice {
            self.notify_tab(index, notice, cx);
        }
    }

    /// A new tab with an untitled document.
    pub fn new_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let document = cx.new(|_| Document::new());
        self.add_tab(ForTab { document, loading: None, selection: None }, window, cx);
    }

    /// Opens files into tabs: a file open in a window already comes forward there.
    pub fn open_paths(&mut self, paths: &[impl AsRef<Path>], window: &mut Window, cx: &mut Context<Self>) {
        for path in paths {
            let path = path.as_ref();
            if let Some(index) = self.find(path, cx) {
                self.activate(index, window, cx);
                continue;
            }
            if let Some((other, index)) = windows::find_open(path, cx) {
                let _ = other.update(cx, |workspace, window, cx| {
                    workspace.activate(index, window, cx);
                    window.activate_window();
                });
                continue;
            }
            match windows::open_document(path, cx) {
                Opening::Document(document, loading) => {
                    self.add_tab(ForTab { document, loading, selection: None }, window, cx)
                }
                Opening::Failed(notice) => self.notify(notice, cx),
            }
        }
    }

    /// Switches to the tab at `index`: its view takes the focus, the bar scrolls to show it.
    pub fn activate(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.tabs.activate(index) {
            return;
        }
        self.reveal_tab = true;
        window.focus(&self.editor().focus_handle(cx), cx);
        self.update_title(window, cx);
        update_menus(Some(self), cx);
        cx.notify();
    }

    pub fn activate_next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.tabs.activate_next();
        self.activate(self.tabs.active_index(), window, cx);
    }

    pub fn activate_previous(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.tabs.activate_previous();
        self.activate(self.tabs.active_index(), window, cx);
    }

    /// Switches to the tab at `index` from the left, or to the last one if there are fewer tabs, as
    /// Cmd+1…8 do; `None` is the last tab, as Cmd+9.
    pub fn select(&mut self, index: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let last = self.tabs.len() - 1;
        self.activate(index.map_or(last, |index| index.min(last)), window, cx);
    }

    /// Moves a tab dragged from `from` to `to`.
    pub fn move_tab(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        self.tabs.move_tab(from, to);
        cx.notify();
    }

    /// Closes the tab at `index` without asking: changes to save stay among the closed tabs. The
    /// last tab closes the window.
    pub fn close_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.len() == 1 {
            self.close_window(window, cx);
            return;
        }
        let was_active = index == self.tabs.active_index();
        let Some(tab) = self.tabs.remove(index) else { return };
        windows::remember_tab(tab.closed(cx), cx);
        if was_active {
            let active = self.tabs.active_index();
            self.activate(active, window, cx);
        } else {
            cx.notify();
        }
    }

    /// Alt or F10: brings the keyboard to the menu bar, or back. Not when the window is not active:
    /// Alt released after switching windows with Alt+Tab is not for the menus.
    pub fn toggle_menu_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(menu_bar) = &self.menu_bar
            && window.is_window_active()
        {
            menu_bar.update(cx, |menu_bar, cx| menu_bar.toggle(window, cx));
        }
    }

    /// Closes the window; its tabs go among the closed ones.
    pub fn close_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.remember_tabs(cx);
        window.remove_window();
    }

    /// Keeps the window among the closed ones, with its tabs: it comes back whole.
    pub fn remember_tabs(&mut self, cx: &mut App) {
        let tabs = self.tabs.iter().map(|tab| tab.closed(cx)).collect();
        windows::remember_window(tabs, self.tabs.active_index(), cx);
    }

    /// Opens what closed last again: a tab here, or a closed window as a window of its own.
    pub fn reopen_closed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match windows::take_closed(cx) {
            Some(Closed::Tab(tab)) => self.reopen(tab, window, cx),
            // Once this window is done with the action: a new window is placed and numbered by it.
            Some(Closed::Window { tabs, active }) => cx.defer(move |cx| windows::reopen_window(tabs, active, cx)),
            None => {}
        }
    }

    /// Opens a closed tab again, where its file is open if a window has it: there the kept changes
    /// take the place of a tab without changes, or come next to one with changes of its own.
    pub fn reopen(&mut self, closed: ClosedTab, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = closed.path.clone() {
            if let Some(index) = self.find(&path, cx) {
                return self.reopen_over(index, closed, window, cx);
            }
            if let Some((other, _)) = windows::find_open(&path, cx) {
                let _ = other.update(cx, |workspace, window, cx| {
                    workspace.reopen(closed, window, cx);
                    window.activate_window();
                });
                return;
            }
        }
        let selection = Some(closed.selection);
        let tab = match (closed.document, closed.path) {
            (Some(document), _) => ForTab { document, loading: None, selection },
            (None, Some(path)) => match windows::open_document(&path, cx) {
                Opening::Document(document, loading) => ForTab { document, loading, selection },
                Opening::Failed(notice) => return self.notify(notice, cx),
            },
            (None, None) => return,
        };
        self.add_tab(tab, window, cx);
    }

    /// Opens a closed tab whose file is open in the tab at `index`.
    fn reopen_over(&mut self, index: usize, closed: ClosedTab, window: &mut Window, cx: &mut Context<Self>) {
        let Some(document) = closed.document else { return self.activate(index, window, cx) };
        let tab = ForTab { document, loading: None, selection: Some(closed.selection) };
        let open_has_changes = self.tabs.get(index).is_some_and(|tab| tab.document.read(cx).is_modified());
        if open_has_changes {
            return self.add_tab(tab, window, cx);
        }
        let replaced = Tab::new(tab.document, None, None, window, cx);
        self.tabs.replace(index, replaced);
        self.settle(index, tab.selection, window, cx);
        self.activate(index, window, cx);
    }

    /// The name of the document of a tab: its file, or "Untitled 2".
    fn tab_title(tab: &Tab, cx: &App) -> String {
        match (&tab.document.read(cx).path, tab.untitled) {
            (Some(path), _) => {
                path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned())
            }
            (None, Some(number)) if number > 1 => format!("{} {number}", tr(Key::FileUntitled)),
            (None, _) => tr(Key::FileUntitled).to_owned(),
        }
    }

    /// Shows the name of the active document in the title of the window, and whether it has changes
    /// to save: on macOS a dot in the close button, elsewhere an asterisk.
    fn update_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tab = self.tabs.active();
        let doc = tab.document.read(cx);
        let title = Title { name: Self::tab_title(tab, cx), modified: doc.is_modified(), path: doc.path.clone() };
        if title == self.title {
            return;
        }
        let renamed = title.name != self.title.name;
        if cfg!(target_os = "macos") {
            window.set_window_title(&title.name);
            window.set_window_edited(title.modified);
        } else {
            let mark = if title.modified { "*" } else { "" };
            window.set_window_title(&format!("{mark}{} — MigPad", title.name));
        }
        window.set_document_path(title.path.as_deref());
        self.title = title;
        if renamed {
            // The list of windows in the Window menu shows the new name.
            update_menus(Some(self), cx);
        }
    }

    /// A document of the window has changed: its text, or its file, or it has loaded.
    fn document_changed(&mut self, document: EntityId, window: &mut Window, cx: &mut Context<Self>) {
        self.revision += 1;
        if let Some(index) = self.tab_of(document) {
            // A reopened tab goes back to where it was once its file has loaded.
            let tab = self.tabs.get_mut(index).expect("found above");
            if !tab.document.read(cx).is_preview()
                && let Some(selection) = tab.pending_selection.take()
            {
                tab.editor.update(cx, |editor, cx| editor.select(selection, window, cx));
            }
            self.check_losses(index, cx);
        }
        self.update_title(window, cx);
        cx.notify();
    }

    /// Files dragged from another program over the window: they open once dropped. The drop itself
    /// is taken in [`Workspace::drop_files`], since GPUI does not drop on an element after typing.
    fn drag_files(&mut self, event: &DragMoveEvent<ExternalPaths>, cx: &mut Context<Self>) {
        self.dragged_files = Some(event.drag(cx).paths().to_vec());
    }

    fn drop_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(paths) = self.dragged_files.take() else { return };
        if cx.stop_active_drag(window) {
            self.open_paths(&paths, window, cx);
        }
    }

    /// The notifications of the active tab.
    fn notification_bars(&self, cx: &mut Context<Self>) -> Vec<NotificationBar> {
        let tab = self.tabs.active();
        let document = tab.document.entity_id();
        tab.notices
            .iter()
            .map(|(id, notice)| {
                let (workspace, id) = (cx.entity().downgrade(), *id);
                let actions = notice.actions.iter().enumerate().map(|(i, &action)| {
                    let workspace = workspace.clone();
                    Button::new(SharedString::from(format!("notice-{id}-{i}")), action.label()).on_click(
                        move |_, window, cx| {
                            let _ = workspace.update(cx, |workspace, cx| {
                                workspace.run_notice_action(document, id, action, window, cx)
                            });
                        },
                    )
                });
                let bar = NotificationBar::new(("notice", id), notice.severity, notice.message.clone());
                actions.fold(bar, NotificationBar::action).on_close(tr(Key::NoticeClose), move |_, cx| {
                    let _ = workspace.update(cx, |workspace, cx| workspace.close_notice(document, id, cx));
                })
            })
            .collect()
    }

    /// The characters of the selection of the active tab, or `None` while a long one is counted a
    /// part a frame.
    fn selection_chars(&mut self, cx: &mut Context<Self>) -> Option<Option<usize>> {
        let tab = self.tabs.active();
        let selection = tab.editor.read(cx).selection();
        let range = selection.anchor.min(selection.head)..selection.anchor.max(selection.head);
        if range.is_empty() {
            self.selection_count = None;
            return None;
        }
        let key = (tab.document.entity_id(), range.clone(), self.revision);
        if let Some(count) = &self.selection_count
            && count.key == key
        {
            return Some(count.done());
        }
        let text = tab.document.read(cx).text();
        if range.len() <= COUNT_STEP {
            let chars = count_chars(text, range.clone());
            self.selection_count = Some(SelectionCount { key, counted_to: range.end, chars, _task: None });
            return Some(Some(chars));
        }
        let task = cx.spawn(async move |workspace, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(1)).await;
                let done = workspace.update(cx, |workspace, cx| workspace.count_selection_step(cx));
                if done.unwrap_or(true) {
                    break;
                }
            }
        });
        self.selection_count = Some(SelectionCount { key, counted_to: range.start, chars: 0, _task: Some(task) });
        Some(None)
    }

    /// Counts the next part of a long selection; returns whether the count is over: done, or stale.
    fn count_selection_step(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(count) = &mut self.selection_count else { return true };
        let (document, _, revision) = count.key.clone();
        let Some(tab) = self.tabs.iter().find(|tab| tab.document.entity_id() == document) else { return true };
        if revision != self.revision {
            return true;
        }
        let done = count.step(tab.document.read(cx).text());
        if done {
            cx.notify();
        }
        done
    }

    fn status_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> migpad_ui::StatusBar {
        let selection = self.selection_chars(cx);
        let tab = self.tabs.active();
        let caret = tab.editor.read(cx).caret_position(cx);
        if caret.1.is_none() {
            // The column of a caret far into a long line is found a part a frame.
            window.request_animation_frame();
        }
        status::status_bar(tab.document.read(cx), caret, selection, tab.loading.as_ref())
    }

    /// The bar of tabs, shown even with one tab; its button opens a new tab, as File > New does.
    fn tab_bar(&self, window: &Window, cx: &mut Context<Self>) -> TabBar {
        let tabs = self
            .tabs
            .iter()
            .map(|tab| {
                let doc = tab.document.read(cx);
                TabInfo {
                    title: Self::tab_title(tab, cx).into(),
                    tooltip: doc.path.as_ref().map(|path| SharedString::from(path.display().to_string())),
                    modified: doc.is_modified(),
                }
            })
            .collect();
        let workspace = cx.entity().downgrade();
        let (select, close, moved, new) = (workspace.clone(), workspace.clone(), workspace.clone(), workspace);
        let new_keys = keys::for_action(&NewTab, &self.editor().focus_handle(cx), window).map(SharedString::from);
        TabBar::new(cx.entity_id(), tabs, self.tabs.active_index(), self.tab_scroll.clone())
            .close_label(tr(Key::FileCloseTab))
            .new_label(tr(Key::FileNew), new_keys)
            .on_select(move |index, window, cx| {
                let _ = select.update(cx, |workspace, cx| workspace.activate(index, window, cx));
            })
            .on_close(move |index, window, cx| {
                let _ = close.update(cx, |workspace, cx| workspace.close_tab(index, window, cx));
            })
            .on_move(move |(from, to), _, cx| {
                let _ = moved.update(cx, |workspace, cx| workspace.move_tab(from, to, cx));
            })
            .on_new(move |(), window, cx| {
                let _ = new.update(cx, |workspace, cx| workspace.new_tab(window, cx));
            })
    }
}

impl Tab {
    fn new(
        document: Entity<Document>,
        untitled: Option<u32>,
        loading: Option<Loading>,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> Tab {
        let editor = cx.new(|cx| {
            let mut editor = EditorView::new(document.clone(), window, cx);
            // Lines wrap to the width of the window until View > Word Wrap turns it off; those of a
            // large file never do.
            editor.set_word_wrap(true, cx);
            editor
        });
        let observe = cx.observe_in(&document, window, |workspace, document, window, cx| {
            workspace.document_changed(document.entity_id(), window, cx)
        });
        // The status bar shows how far the file has loaded, a few times a second; a load that
        // stopped short is told over the text.
        let progress = loading.clone().map(|loading| {
            let (document, id) = (document.downgrade(), document.entity_id());
            cx.spawn(async move |workspace, cx| {
                loop {
                    cx.background_executor().timer(PROGRESS_TICK).await;
                    let failure = loading.failure.borrow_mut().take();
                    if let Some(notice) = failure {
                        let _ = workspace.update(cx, |workspace, cx| workspace.notify_document(id, notice, cx));
                        break;
                    }
                    let preview = document.read_with(cx, |doc, _| doc.is_preview()).unwrap_or(false);
                    if !preview || loading.stopped.get() || workspace.update(cx, |_, cx| cx.notify()).is_err() {
                        break;
                    }
                }
            })
        });
        Tab {
            document,
            editor,
            untitled,
            loading,
            pending_selection: None,
            notices: Vec::new(),
            losses_checked: false,
            _observe: observe,
            _progress: progress,
        }
    }

    /// What is kept of the tab once it is closed: the document, if it has changes to save.
    fn closed(&self, cx: &App) -> ClosedTab {
        let doc = self.document.read(cx);
        ClosedTab {
            document: doc.is_modified().then(|| self.document.clone()),
            path: doc.path.clone(),
            selection: self.pending_selection.unwrap_or_else(|| self.editor.read(cx).selection()),
        }
    }
}

impl Focusable for Workspace {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.editor().focus_handle(cx)
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme(cx);
        if self.reveal_tab {
            // The bar scrolls once it knows its width: in the first frame it does not.
            self.tab_scroll.scroll_to_item(self.tabs.active_index());
            if self.tab_scroll.bounds().size.width > px(0.) {
                self.reveal_tab = false;
            } else {
                window.request_animation_frame();
            }
        }
        let handlers = cx.global::<Registry>().window_handlers();
        let root = div()
            .key_context("Workspace")
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(theme.bar))
            .text_color(rgb(theme.text))
            .text_size(migpad_ui::text_size())
            .on_drag_move(
                cx.listener(|workspace, event: &DragMoveEvent<ExternalPaths>, _, cx| workspace.drag_files(event, cx)),
            );
        let root = handlers.iter().fold(root, |root, handler| handler(root, cx));
        let root = match self.menu_bar.clone() {
            // Alt with a letter opens the menu with that mnemonic; Alt pressed and released alone
            // brings the keyboard to the menus; Alt held shows the mnemonics.
            Some(menu_bar) => {
                let (keys, modifiers, presses, wheel) =
                    (menu_bar.clone(), menu_bar.clone(), menu_bar.clone(), menu_bar.clone());
                root.capture_key_down(move |event, window, cx| {
                    keys.update(cx, |menu_bar, _| menu_bar.other_input());
                    let held = &event.keystroke.modifiers;
                    if held.alt && !held.control && !held.platform && !keys.read(cx).is_active() {
                        let opened =
                            keys.update(cx, |menu_bar, cx| menu_bar.open_by_mnemonic(&event.keystroke, window, cx));
                        if opened {
                            cx.stop_propagation();
                        }
                    }
                })
                .on_modifiers_changed(move |event, window, cx| {
                    modifiers.update(cx, |menu_bar, cx| menu_bar.modifiers_changed(&event.modifiers, window, cx));
                })
                .capture_any_mouse_down(move |_, _, cx| presses.update(cx, |menu_bar, _| menu_bar.other_input()))
                .on_scroll_wheel(move |_, _, cx| wheel.update(cx, |menu_bar, _| menu_bar.other_input()))
                .child(menu_bar)
            }
            None => root,
        };
        let status_bar = self.status_bar(window, cx);
        if !self.drawn {
            // Once the first frame is drawn, the menus know which of their commands can act: until
            // then the window has no elements to ask.
            cx.on_next_frame(window, |workspace, _, cx| {
                workspace.drawn = true;
                cx.notify();
            });
        }
        // Files dropped on the window, whatever the input was before; a press forgets a drag that
        // left the window.
        let workspace = cx.weak_entity();
        let drops = canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                let (dropped, pressed) = (workspace.clone(), workspace);
                window.on_mouse_event(move |_: &MouseUpEvent, phase, window, cx| {
                    if phase == gpui::DispatchPhase::Capture {
                        let _ = dropped.update(cx, |workspace, cx| workspace.drop_files(window, cx));
                    }
                });
                window.on_mouse_event(move |_: &MouseDownEvent, phase, _, cx| {
                    if phase == gpui::DispatchPhase::Capture {
                        let _ = pressed.update(cx, |workspace, _| workspace.dragged_files = None);
                    }
                });
            },
        )
        .absolute()
        .size_0();
        root.child(self.tab_bar(window, cx))
            .children(self.notification_bars(cx))
            .child(div().flex_1().min_h_0().child(self.editor().clone()))
            .child(status_bar)
            .child(drops)
    }
}
