//! The root of a window: its tabs, each a document and its view, and the commands of the window,
//! which act on the window or on the document of the active tab whatever has the focus.

use std::path::Path;
use std::time::Duration;

use gpui::{
    App, AppContext, Context, Entity, EntityId, ExternalPaths, FocusHandle, Focusable, Global, Render, ScrollHandle,
    SharedString,
    Subscription, Task, Window, WindowId, div, prelude::*, px, rgb,
};
use migpad_core::document::Document;
use migpad_editor::EditorView;
use migpad_ui::notification::NotificationBar;
use migpad_ui::{TabBar, TabInfo, Toolbar, theme};

use crate::commands::{Registry, update_menus};
use crate::notices::{self, Notice};
use crate::status::{self, COUNT_STEP, Loading, SelectionCount, count_chars};
use crate::strings::{Key, tr};
use crate::tabs::Tabs;
use crate::windows::{self, ClosedTab, Opening};

/// Whether the windows show the toolbar: they do until it is hidden.
struct ToolbarHidden(bool);

impl Global for ToolbarHidden {}

pub fn toolbar_visible(cx: &App) -> bool {
    !cx.try_global::<ToolbarHidden>().is_some_and(|hidden| hidden.0)
}

/// Shows or hides the toolbar of every window.
pub fn set_toolbar_visible(visible: bool, cx: &mut App) {
    cx.set_global(ToolbarHidden(!visible));
    cx.refresh_windows();
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
    /// Notifications over the text, each with its number in the window.
    notices: Vec<(u64, Notice)>,
    /// Whether the text was checked for bytes lost in reading: once it is loaded.
    losses_checked: bool,
    _observe: Subscription,
    /// Shows the progress of the loading in the status bar.
    _progress: Option<Task<()>>,
}

pub struct Workspace {
    window_id: WindowId,
    tabs: Tabs<Tab>,
    tab_scroll: ScrollHandle,
    /// Whether the bar of tabs is yet to scroll to the active tab.
    reveal_tab: bool,
    /// The title the window shows, and whether its document has changes to save.
    title: (String, bool),
    /// Counts the changes to the documents of the window: counts of selections go stale with them.
    revision: u64,
    selection_count: Option<SelectionCount>,
    /// Whether the window has drawn a frame: what its elements handle is known after that.
    drawn: bool,
    /// The number of the next notification.
    next_notice: u64,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    /// A window with one tab: `document`, untitled with the number `untitled`, or a file.
    pub fn new(
        document: Entity<Document>,
        untitled: Option<u32>,
        loading: Option<Loading>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // The check marks of the menus are those of the document of the active window.
        let activation = cx.observe_window_activation(window, |workspace, window, cx| {
            if window.is_window_active() {
                update_menus(Some(workspace), cx);
            }
        });
        // Colors follow the appearance of the system, unless a theme is chosen.
        let appearance = migpad_ui::theme::follow_system(window, cx);
        let first = Tab::new(document, untitled, loading, window, cx);
        let mut workspace = Workspace {
            window_id: window.window_handle().window_id(),
            tabs: Tabs::new(first),
            tab_scroll: ScrollHandle::new(),
            reveal_tab: true,
            title: (String::new(), false),
            revision: 0,
            selection_count: None,
            drawn: false,
            next_notice: 0,
            _subscriptions: vec![activation, appearance],
        };
        workspace.check_losses(0, cx);
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
        &self.title.0
    }

    /// The numbers of the untitled documents of the window.
    pub fn untitled_numbers(&self) -> impl Iterator<Item = u32> + '_ {
        self.tabs.iter().filter_map(|tab| tab.untitled)
    }

    /// The tab of the file at `path`, if the window has it open.
    pub fn find(&self, path: &Path, cx: &App) -> Option<usize> {
        self.tabs.position(|tab| tab.document.read(cx).path.as_deref().is_some_and(|open| windows::same_file(open, path)))
    }

    /// Adds a tab with `document` at the end and switches to it.
    pub fn add_tab(
        &mut self,
        document: Entity<Document>,
        untitled: Option<u32>,
        loading: Option<Loading>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tab = Tab::new(document, untitled, loading, window, cx);
        let index = self.tabs.push(tab);
        self.check_losses(index, cx);
        self.activate(index, window, cx);
    }

    /// Shows a notification over the text of the active tab.
    pub fn notify(&mut self, notice: Notice, cx: &mut Context<Self>) {
        let index = self.tabs.active_index();
        self.notify_tab(index, notice, cx);
    }

    fn notify_tab(&mut self, index: usize, notice: Notice, cx: &mut Context<Self>) {
        let id = self.next_notice;
        self.next_notice += 1;
        if let Some(tab) = self.tabs.get_mut(index) {
            tab.notices.push((id, notice));
            cx.notify();
        }
    }

    /// Closes the notification `id` of the tab with `document`.
    fn close_notice(&mut self, document: EntityId, id: u64, cx: &mut Context<Self>) {
        if let Some(index) = self.tabs.position(|tab| tab.document.entity_id() == document)
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
        let number = windows::untitled_number(self.untitled_numbers(), Some(self.window_id), cx);
        let document = cx.new(|_| Document::new());
        self.add_tab(document, Some(number), None, window, cx);
    }

    /// Opens files into tabs: a file open in a window already comes forward there.
    pub fn open_paths(&mut self, paths: &[impl AsRef<Path>], window: &mut Window, cx: &mut Context<Self>) {
        for path in paths {
            let path = path.as_ref();
            if let Some(index) = self.find(path, cx) {
                self.activate(index, window, cx);
                continue;
            }
            let elsewhere = windows::workspaces(cx).find_map(|(other, workspace)| Some((other, workspace.find(path, cx)?)));
            if let Some((other, index)) = elsewhere {
                let _ = other.update(cx, |workspace, window, cx| {
                    workspace.activate(index, window, cx);
                    window.activate_window();
                });
                continue;
            }
            match windows::open_document(path, cx) {
                Opening::Document(document, loading) => self.add_tab(document, None, loading, window, cx),
                Opening::Failed(path, error) => self.notify(notices::open_failed(&path, &error), cx),
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
        windows::remember_closed(tab.closed(cx), cx);
        if was_active {
            let active = self.tabs.active_index();
            self.activate(active, window, cx);
        } else {
            cx.notify();
        }
    }

    /// Drops the first tab without keeping it among the closed ones: the untitled tab of a new
    /// window, once another one came.
    pub fn drop_first_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.len() > 1 && self.tabs.remove(0).is_some() {
            let active = self.tabs.active_index();
            self.activate(active, window, cx);
        }
    }

    /// Closes the window; its tabs go among the closed ones.
    pub fn close_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.remember_tabs(cx);
        window.remove_window();
    }

    /// Keeps the tabs of the window among the closed ones, the active one last: it comes back first.
    pub fn remember_tabs(&mut self, cx: &mut App) {
        let active = self.tabs.active_index();
        let tabs = self.tabs.iter().enumerate();
        let closed: Vec<ClosedTab> =
            tabs.clone().filter(|(i, _)| *i != active).chain(tabs.filter(|(i, _)| *i == active)).map(|(_, tab)| tab.closed(cx)).collect();
        for tab in closed {
            windows::remember_closed(tab, cx);
        }
    }

    /// Opens the tab closed last again; one with a file that is open now comes forward instead.
    pub fn reopen_closed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(closed) = windows::take_closed(cx) else { return };
        let (document, loading) = match (closed.document, &closed.path) {
            (Some(document), _) => (document, None),
            (None, Some(path)) => match self.find(path, cx) {
                Some(index) => return self.activate(index, window, cx),
                None => match windows::open_document(path, cx) {
                    Opening::Document(document, loading) => (document, loading),
                    Opening::Failed(path, error) => return self.notify(notices::open_failed(&path, &error), cx),
                },
            },
            (None, None) => return,
        };
        let untitled = document
            .read(cx)
            .path
            .is_none()
            .then(|| windows::untitled_number(self.untitled_numbers(), Some(self.window_id), cx));
        self.add_tab(document, untitled, loading, window, cx);
        self.editor().update(cx, |editor, cx| editor.select(closed.selection, window, cx));
    }

    /// The name of the document of a tab: its file, or "Untitled 2".
    fn tab_title(tab: &Tab, cx: &App) -> String {
        match (&tab.document.read(cx).path, tab.untitled) {
            (Some(path), _) => path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned()),
            (None, Some(number)) if number > 1 => format!("{} {number}", tr(Key::FileUntitled)),
            (None, _) => tr(Key::FileUntitled).to_owned(),
        }
    }

    /// Shows the name of the active document in the title of the window, and whether it has changes
    /// to save: on macOS a dot in the close button, elsewhere an asterisk.
    fn update_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tab = self.tabs.active();
        let name = Self::tab_title(tab, cx);
        let doc = tab.document.read(cx);
        let modified = doc.is_modified();
        let title = (name, modified);
        if title == self.title {
            return;
        }
        let renamed = title.0 != self.title.0;
        if cfg!(target_os = "macos") {
            window.set_window_title(&title.0);
            window.set_window_edited(modified);
        } else {
            let mark = if modified { "*" } else { "" };
            window.set_window_title(&format!("{mark}{} — MigPad", title.0));
        }
        window.set_document_path(doc.path.as_deref());
        self.title = title;
        if renamed {
            // The list of windows in the Window menu shows the new name.
            update_menus(Some(self), cx);
        }
    }

    /// A document of the window has changed: its text, or its file, or it has loaded.
    fn document_changed(&mut self, document: EntityId, window: &mut Window, cx: &mut Context<Self>) {
        self.revision += 1;
        if let Some(index) = self.tabs.position(|tab| tab.document.entity_id() == document) {
            self.check_losses(index, cx);
        }
        self.update_title(window, cx);
        cx.notify();
    }

    /// The notifications of the active tab.
    fn notification_bars(&self, cx: &mut Context<Self>) -> Vec<NotificationBar> {
        let tab = self.tabs.active();
        let document = tab.document.entity_id();
        tab.notices
            .iter()
            .map(|(id, notice)| {
                let (workspace, id) = (cx.entity().downgrade(), *id);
                NotificationBar::new(("notice", id), notice.severity, notice.message.clone()).on_close(
                    tr(Key::NoticeClose),
                    move |_, cx| {
                        let _ = workspace.update(cx, |workspace, cx| workspace.close_notice(document, id, cx));
                    },
                )
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
            self.selection_count = Some(SelectionCount { key, counted_to: range.end, chars, task: None });
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
        self.selection_count = Some(SelectionCount { key, counted_to: range.start, chars: 0, task: Some(task) });
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

    fn status_bar(&mut self, cx: &mut Context<Self>) -> migpad_ui::StatusBar {
        let selection = self.selection_chars(cx);
        let tab = self.tabs.active();
        let caret = tab.editor.read(cx).caret_position(cx);
        status::status_bar(tab.document.read(cx), caret, selection, tab.loading.as_ref())
    }

    fn tab_bar(&self, cx: &mut Context<Self>) -> TabBar {
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
        TabBar::new(cx.entity_id(), tabs, self.tabs.active_index(), self.tab_scroll.clone())
            .close_label(tr(Key::FileCloseTab))
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
        let editor = cx.new(|cx| EditorView::new(document.clone(), window, cx));
        let observe = cx.observe_in(&document, window, |workspace, document, window, cx| {
            workspace.document_changed(document.entity_id(), window, cx)
        });
        // The status bar shows how far the file has loaded, a few times a second.
        let progress = loading.is_some().then(|| {
            let document = document.downgrade();
            cx.spawn(async move |workspace, cx| {
                loop {
                    cx.background_executor().timer(PROGRESS_TICK).await;
                    let loading = document.read_with(cx, |doc, _| doc.is_preview()).unwrap_or(false);
                    if !loading || workspace.update(cx, |_, cx| cx.notify()).is_err() {
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
            selection: self.editor.read(cx).selection(),
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
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(theme.bar))
            .text_color(rgb(theme.text))
            .text_size(migpad_ui::text_size())
            // Files dropped on the window open in tabs.
            .on_drop(cx.listener(|workspace, paths: &ExternalPaths, window, cx| {
                workspace.open_paths(paths.paths(), window, cx);
            }));
        let root = handlers.iter().fold(root, |root, handler| handler(root, cx));
        let toolbar = toolbar_visible(cx).then(|| Toolbar::new(cx.global::<Registry>().toolbar(self, window, cx)));
        let status_bar = self.status_bar(cx);
        if !self.drawn {
            // Once the first frame is drawn, the toolbar knows which of its commands can act.
            self.drawn = true;
            window.request_animation_frame();
        }
        root.children(toolbar)
            .child(self.tab_bar(cx))
            .children(self.notification_bars(cx))
            .child(div().flex_1().min_h_0().child(self.editor().clone()))
            .child(status_bar)
    }
}
