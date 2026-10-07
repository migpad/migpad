//! The root of a window: its tabs, each a document and its view, and the commands of the window,
//! which act on the window or on the document of the active tab whatever has the focus.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gpui::{
    App, AppContext, Context, DragMoveEvent, Entity, EntityId, ExternalPaths, FocusHandle, Focusable, MouseDownEvent,
    MouseUpEvent, PathPromptOptions, Pixels, Point, PromptButton, PromptLevel, Render, ScrollHandle, SharedString,
    Subscription, Task, Window, WindowHandle, WindowId, canvas, div, prelude::*, px, rgb,
};
use migpad_core::document::{Document, Fingerprint, Format, OpenAs, Opened, open};
use migpad_core::encoding::Encoding;
use migpad_core::history::Selection;
use migpad_core::state::TabState;
use migpad_core::state::session::{Rect, WindowMode, WindowState};
use migpad_core::text::TextStore;
use migpad_editor::{ContextMenuEvent, EditorView};
use migpad_ui::notification::NotificationBar;
use migpad_ui::{Button, ContextMenu, ItemSpec, MenuBar, TabBar, TabInfo, theme};

use crate::commands::{Registry, update_menus};
use crate::find::FindBar;
use crate::go_to::GoToBar;
use crate::journals;
use crate::keys;
use crate::modules::file::NewTab;
use crate::notices::{self, Notice, NoticeAction, Topic};
use crate::recent;
use crate::session;
use crate::status::{self, COUNT_STEP, Loading, SelectionCount, count_chars};
use crate::strings::{Key, fill, tr};
use crate::tabs::Tabs;
use crate::view_options;
use crate::windows::{self, Closed, ClosedTab, ForTab, Opening, Reopening};

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
    /// Whether its changes are to be dropped: the question on closing it was answered so.
    discarded: bool,
    /// Whether the text was checked for bytes lost in reading: once it is loaded.
    losses_checked: bool,
    /// Flushes the journal to the disk a moment after the last edit.
    journal_sync: Option<Task<()>>,
    /// Whether its file is not there any more: another program removed it.
    missing: bool,
    _observe: Subscription,
    /// Shows the progress of the loading in the status bar.
    _progress: Option<Task<()>>,
    /// Opens the context menu of the text.
    _context_menu: Subscription,
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
    /// Whether a question is open over the window: one at a time — a second one would wait behind
    /// it on macOS, and take its place on Linux.
    asking: bool,
    /// Where the window is, for the session: its bounds, how it shows, and its screen.
    place: (Rect, WindowMode, Option<String>),
    /// Compares the files of the documents with the disk, after the window comes back.
    file_check: Option<Task<()>>,
    /// The find bar, once it was opened, and whether it shows.
    find_bar: Option<Entity<FindBar>>,
    find_shown: bool,
    /// The bar of Go to Line, once it was opened, and whether it shows.
    go_to_bar: Option<Entity<GoToBar>>,
    go_to_shown: bool,
    /// The context menu open over the window until it closes, and the view it is of.
    context_menu: Option<(Entity<ContextMenu>, EntityId, Subscription)>,
    _subscriptions: Vec<Subscription>,
}

/// What a text the context menu is of can do: its commands are gray where they have nothing to do.
#[derive(Clone, Copy, Debug, Default)]
struct TextState {
    selected: bool,
    /// Not the preview of a file still loading, which takes no edits.
    editable: bool,
    can_undo: bool,
    can_redo: bool,
    empty: bool,
    /// Whether the clipboard has text to paste.
    clipboard: bool,
}

impl TextState {
    /// Whether the command `id` of the context menu has something to do in the text.
    fn can(&self, id: &str) -> bool {
        match id {
            "edit.undo" => self.editable && self.can_undo,
            "edit.redo" => self.editable && self.can_redo,
            "edit.cut" | "edit.delete" => self.editable && self.selected,
            "edit.copy" => self.selected,
            "edit.paste" => self.editable && self.clipboard,
            "edit.select_all" => !self.empty,
            _ => true,
        }
    }
}

/// The answer to "Save the changes?" on closing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SaveAnswer {
    Save,
    DontSave,
    Cancel,
}

impl Workspace {
    /// A window with one tab.
    pub fn new(first: ForTab, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // The check marks of the menus are those of the document of the active window; the
        // journals of a window that goes to the background reach the disk.
        let activation = cx.observe_window_activation(window, |workspace, window, cx| {
            if window.is_window_active() {
                update_menus(Some(workspace), cx);
                session::activated(workspace.window_id, cx);
                // Back in the window: other programs may have changed its files meanwhile.
                workspace.check_files(cx);
            } else {
                for tab in workspace.tabs.iter() {
                    journals::sync(&tab.document, cx);
                }
            }
        });
        // Colors follow the appearance of the system, unless a theme is chosen.
        let appearance = migpad_ui::theme::follow_system(window, cx);
        // The session knows where the window is.
        let moved = cx.observe_window_bounds(window, |workspace, window, cx| {
            workspace.place = session::place(window, cx);
            workspace.record_session(cx);
        });
        let window_id = window.window_handle().window_id();
        let untitled = Self::untitled_for(&first.document, [], window_id, cx);
        let tab = Tab::new(first.document, untitled, first.loading, window, cx);
        let menu_bar = view_options::menu_bar(cx).then(|| Self::make_menu_bar(window, cx));
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
            asking: false,
            place: session::place(window, cx),
            file_check: None,
            find_bar: None,
            find_shown: false,
            go_to_bar: None,
            go_to_shown: false,
            context_menu: None,
            _subscriptions: vec![activation, appearance, moved],
        };
        workspace.settle(0, first.selection, first.notice, window, cx);
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

    /// The menu bar MigPad draws over the tabs, of the menus of the registry.
    fn make_menu_bar(window: &mut Window, cx: &mut Context<Self>) -> Entity<MenuBar> {
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
    }

    /// Shows the menu bar MigPad draws over the tabs, or hides it: on macOS, View ▸ Menu Bar in
    /// Window. A hidden bar gives the focus it had back to the text.
    pub fn set_menu_bar(&mut self, shown: bool, window: &mut Window, cx: &mut Context<Self>) {
        match (shown, self.menu_bar.is_some()) {
            (true, false) => self.menu_bar = Some(Self::make_menu_bar(window, cx)),
            (false, true) => {
                let had_focus = self.menu_bar.take().is_some_and(|bar| bar.read(cx).is_active());
                if had_focus {
                    window.focus(&self.editor().focus_handle(cx), cx);
                }
            }
            _ => return,
        }
        cx.notify();
    }

    /// Shows the find bar over the text, made the first time.
    pub fn show_find_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<FindBar> {
        let workspace = cx.weak_entity();
        let bar = self.find_bar.get_or_insert_with(|| cx.new(|cx| FindBar::new(workspace, window, cx))).clone();
        if !self.find_shown {
            self.find_shown = true;
            cx.notify();
        }
        bar
    }

    /// Hides the find bar; tells whether it showed.
    pub fn hide_find_bar(&mut self, cx: &mut Context<Self>) -> bool {
        let shown = std::mem::take(&mut self.find_shown);
        if shown {
            cx.notify();
        }
        shown
    }

    /// The view that typing goes to: the field of a bar with the focus, or the view of the document.
    pub fn input_target(&self, window: &Window, cx: &App) -> Entity<EditorView> {
        let find = self.find_bar_shown().and_then(|bar| bar.read(cx).focused_field(window, cx));
        let go_to = || self.go_to_bar_shown().and_then(|bar| bar.read(cx).focused_field(window, cx));
        find.or_else(go_to).unwrap_or_else(|| self.editor().clone())
    }

    /// Shows the bar of Go to Line over the text, made the first time.
    pub fn show_go_to_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<GoToBar> {
        let workspace = cx.weak_entity();
        let bar = self.go_to_bar.get_or_insert_with(|| cx.new(|cx| GoToBar::new(workspace, window, cx))).clone();
        if !self.go_to_shown {
            self.go_to_shown = true;
            cx.notify();
        }
        bar
    }

    /// Hides the bar of Go to Line; tells whether it showed.
    pub fn hide_go_to_bar(&mut self, cx: &mut Context<Self>) -> bool {
        let shown = std::mem::take(&mut self.go_to_shown);
        if shown {
            cx.notify();
        }
        shown
    }

    /// The bar of Go to Line, if it shows.
    pub fn go_to_bar_shown(&self) -> Option<&Entity<GoToBar>> {
        self.go_to_bar.as_ref().filter(|_| self.go_to_shown)
    }

    /// Opens the context menu of the text of `target` — a document or a field — as `event` asks:
    /// its commands act there, those with nothing to do there gray.
    pub fn open_context_menu(
        &mut self,
        target: &Entity<EditorView>,
        event: &ContextMenuEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = target.read(cx);
        let doc = view.document().read(cx);
        let selection = view.selection();
        let text = TextState {
            selected: selection.anchor != selection.head,
            editable: !doc.is_preview(),
            can_undo: doc.can_undo(),
            can_redo: doc.can_redo(),
            empty: doc.text().is_empty(),
            clipboard: cx.read_from_clipboard().and_then(|item| item.text()).is_some_and(|text| !text.is_empty()),
        };
        let items = cx.global::<Registry>().context_menu(|id| text.can(id));
        self.show_menu(items, event.position, event.keyboard, Some(target.entity_id()), window, cx);
    }

    /// Shows a menu of `items` at `position` in the window, over the text of the view `of` if it
    /// is of one: the menu of the system on macOS, one MigPad draws on Windows and Linux. The
    /// action chosen goes to where the focus is. `keyboard` if a key opened it.
    pub fn show_menu(
        &mut self,
        items: Vec<ItemSpec>,
        position: Point<Pixels>,
        keyboard: bool,
        of: Option<EntityId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        #[cfg(target_os = "macos")]
        {
            let _ = (keyboard, of);
            // Once the event that asked for it is handled: the menu runs a loop of its own.
            cx.spawn_in(window, async move |_, cx| {
                let chosen = crate::native_menu::pop_up(&items, position);
                if let Some(action) = chosen.and_then(|path| migpad_ui::action_at(&items, &path)) {
                    let _ = cx.update(|window, cx| window.dispatch_action(action, cx));
                }
            })
            .detach();
        }
        #[cfg(not(target_os = "macos"))]
        {
            let menu = cx.new(|cx| ContextMenu::new(items, position, keyboard, window, cx));
            let closed = cx.subscribe_in(&menu, window, |workspace, _, _: &gpui::DismissEvent, _, cx| {
                workspace.context_menu = None;
                cx.notify();
            });
            self.context_menu = Some((menu, of.unwrap_or(cx.entity_id()), closed));
            cx.notify();
        }
    }

    /// The find bar, if it shows.
    pub fn find_bar_shown(&self) -> Option<&Entity<FindBar>> {
        self.find_bar.as_ref().filter(|_| self.find_shown)
    }

    /// The document of the active tab.
    pub fn document(&self) -> &Entity<Document> {
        &self.tabs.active().document
    }

    /// The documents of the tabs.
    pub fn documents(&self) -> impl Iterator<Item = &Entity<Document>> {
        self.tabs.iter().map(|tab| &tab.document)
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
        self.settle(index, tab.selection, tab.notice, window, cx);
        self.activate(index, window, cx);
    }

    /// Puts a new tab in order: the selection it comes with, at once or once its file has loaded,
    /// what it tells over its text, and a warning of bytes lost in reading.
    fn settle(
        &mut self,
        index: usize,
        selection: Option<Selection>,
        notice: Option<Notice>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(notice) = notice {
            self.notify_tab(index, notice, cx);
        }
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
        // A save that failed — Save As… too — is done again to the file it aimed at.
        let target = self.tabs.get(index).and_then(|tab| {
            tab.notices.iter().find(|(notice, _)| *notice == id).and_then(|(_, notice)| notice.path.clone())
        });
        if !matches!(action, NoticeAction::ShowFirst(_)) {
            self.close_notice(document, id, cx);
        }
        match action {
            NoticeAction::SaveInUtf8 => {
                let format = self.tabs.get(index).map(|tab| tab.document.read(cx).format);
                let format = format.map(|format| Format { encoding: Encoding::UTF_8, bom: false, ..format });
                self.save_into(index, target, format, false, window, cx).detach();
            }
            NoticeAction::ShowFirst(pos) => {
                self.activate(index, window, cx);
                // The character itself is selected: it shows better than a caret beside it.
                let text = self.document().read(cx).text();
                let pos = pos.min(text.len());
                let selection = Selection { anchor: pos, head: text.next_char_boundary(pos, text.len()) };
                self.editor().update(cx, |editor, cx| editor.select(selection, window, cx));
            }
            NoticeAction::SaveReplacing => {
                // What would be lost is replaced in the text first, as the file gets it: the window
                // shows what is saved, and undo brings the characters back.
                let Some(tab) = self.tabs.get(index) else { return };
                let (document, editor) = (tab.document.clone(), tab.editor.clone());
                let selection = editor.read(cx).selection();
                let replaced = document.update(cx, |doc, cx| {
                    let result = doc.replace_losses(doc.format.encoding, selection, Instant::now());
                    cx.notify();
                    result
                });
                let Ok(after) = replaced else { return };
                editor.update(cx, |editor, cx| editor.select(after, window, cx));
                self.save_into(index, target, None, true, window, cx).detach();
            }
            NoticeAction::SaveAs => self.save_as(index, window, cx).detach(),
            NoticeAction::LoadFromDisk => self.load_from_disk(index, window, cx),
            NoticeAction::KeepMine => self.keep_mine(index, cx),
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
        self.save(index, None, false, window, cx).detach();
    }

    pub fn save_as_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let index = self.tabs.active_index();
        self.save_as(index, window, cx).detach();
    }

    /// Saves the document of the tab at `index` to its file in `format`, or in its own format; an
    /// untitled one goes to a file chosen in the dialog of the system. With `accept_losses`
    /// characters that cannot be written are replaced; without, the file is not written and a
    /// notification tells why ([ADR 0015]). Returns whether the file was written — once the dialog
    /// is answered, for an untitled one.
    pub fn save(
        &mut self,
        index: usize,
        format: Option<Format>,
        accept_losses: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        let Some(tab) = self.tabs.get(index) else { return Task::ready(false) };
        let doc = tab.document.read(cx);
        let Some(path) = doc.path.clone() else { return self.save_as(index, window, cx) };
        // Nothing to write: the file is as it was read or saved, and so is the text.
        if format.is_none() && !doc.is_modified() && doc.disk.is_some() && Fingerprint::of_path(&path).ok() == doc.disk
        {
            return Task::ready(true);
        }
        Task::ready(self.save_to(index, path, format, accept_losses, cx))
    }

    /// Saves the document of the tab at `index` to `target`, the file a failed save aimed at, or as
    /// [`Workspace::save`] does without one.
    fn save_into(
        &mut self,
        index: usize,
        target: Option<PathBuf>,
        format: Option<Format>,
        accept_losses: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        match target {
            Some(path) => Task::ready(self.save_to(index, path, format, accept_losses, cx)),
            None => self.save(index, format, accept_losses, window, cx),
        }
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
                // An untitled document has a name now: its number is free for a new one; a file that
                // was gone is there again.
                if let Some(tab) = self.tabs.get_mut(index) {
                    tab.untitled = None;
                    tab.missing = false;
                }
                self.clear_notices(index, Topic::Save, cx);
                self.clear_notices(index, Topic::Disk, cx);
                cx.add_recent_document(&path);
                if let Some(tab) = self.tabs.get(index) {
                    let selection = tab.editor.read(cx).selection();
                    recent::saved(&path, format.encoding, selection, cx);
                }
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
    /// next to its own file or in the home folder, under its name; tells whether it was saved.
    pub fn save_as(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) -> Task<bool> {
        let Some(tab) = self.tabs.get(index) else { return Task::ready(false) };
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
            let Ok(Ok(Some(path))) = chosen.await else { return false };
            workspace
                .update(cx, |workspace, cx| {
                    workspace.tab_of(document).is_some_and(|index| workspace.save_to(index, path, None, false, cx))
                })
                .unwrap_or(false)
        })
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
        let document = journals::document(Document::new(), cx);
        self.add_tab(ForTab::new(document, None, None), window, cx);
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
            match windows::open_file(path, cx) {
                Opening::Document(document, loading) => self.add_tab(ForTab::new(document, loading, None), window, cx),
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

    /// Closes the tab at `index`; if its document has changes, asks first whether to save them
    /// ([ADR 0020]): the tab stays if the question is cancelled or the document is not saved. The
    /// last tab closes the window.
    pub fn close_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(index) else { return };
        if !tab.has_changes(cx) {
            return self.close_tab_now(index, window, cx);
        }
        if self.asking {
            return;
        }
        let (document, title) = (tab.document.entity_id(), Self::tab_title(tab, cx));
        // The tab asked about is the one shown.
        self.activate(index, window, cx);
        let question = fill(Key::CloseQuestion, &[("file", &title)]);
        let answer = self.ask_to_save(&question, tr(Key::CloseDetail), tr(Key::CloseSave), window, cx);
        cx.spawn_in(window, async move |workspace, cx| {
            let close = match answer.await {
                SaveAnswer::Save => {
                    let save =
                        workspace.update_in(cx, |workspace, window, cx| workspace.save_document(document, window, cx));
                    let Ok(save) = save else { return };
                    save.await
                }
                SaveAnswer::DontSave => workspace.update(cx, |workspace, _| workspace.discard(document)).is_ok(),
                SaveAnswer::Cancel => false,
            };
            if close {
                let _ = workspace.update_in(cx, |workspace, window, cx| {
                    if let Some(index) = workspace.tab_of(document) {
                        workspace.close_tab_now(index, window, cx);
                    }
                });
            }
        })
        .detach();
    }

    /// Asks over the window whether to save the changes before closing: `question`, `detail`, and
    /// the buttons `save`, Don't Save and Cancel. Escape cancels; the answer is Cancel too if the
    /// question goes away unanswered.
    fn ask_to_save(
        &mut self,
        question: &str,
        detail: &str,
        save: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<SaveAnswer> {
        let buttons = [
            PromptButton::new(save),
            PromptButton::new(tr(Key::CloseDontSave)),
            PromptButton::cancel(tr(Key::CloseCancel)),
        ];
        self.asking = true;
        let answer = window.prompt(PromptLevel::Warning, question, Some(detail), &buttons, cx);
        cx.spawn(async move |workspace, cx| {
            let answer = answer.await;
            let _ = workspace.update(cx, |workspace, _| workspace.asking = false);
            match answer {
                Ok(0) => SaveAnswer::Save,
                Ok(1) => SaveAnswer::DontSave,
                _ => SaveAnswer::Cancel,
            }
        })
    }

    /// Closes the tab at `index` as it is: its changes, if any, were saved or are to be dropped. The
    /// last tab closes the window.
    fn close_tab_now(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.len() == 1 {
            self.remember_tabs(cx);
            window.remove_window();
            return;
        }
        let was_active = index == self.tabs.active_index();
        let Some(tab) = self.tabs.remove(index) else { return };
        windows::remember_tab(tab.close(cx), cx);
        if was_active {
            let active = self.tabs.active_index();
            self.activate(active, window, cx);
        } else {
            cx.notify();
        }
    }

    /// Saves the document `document`: to its file, or to one chosen if it has none. Tells whether
    /// it was saved, once the dialog of an untitled one is answered.
    fn save_document(&mut self, document: EntityId, window: &mut Window, cx: &mut Context<Self>) -> Task<bool> {
        match self.tab_of(document) {
            Some(index) => self.save(index, None, false, window, cx),
            None => Task::ready(false),
        }
    }

    /// Drops the changes of the document `document` as its tab closes: they were not to be saved.
    fn discard(&mut self, document: EntityId) {
        if let Some(index) = self.tab_of(document)
            && let Some(tab) = self.tabs.get_mut(index)
        {
            tab.discarded = true;
        }
    }

    /// Whether a document of the window has changes to save.
    pub fn has_changes(&self, cx: &App) -> bool {
        self.tabs.iter().any(|tab| tab.has_changes(cx))
    }

    /// The documents of the window with changes to save, and their names.
    pub fn changed_documents(&self, cx: &App) -> Vec<(Entity<Document>, String)> {
        self.tabs
            .iter()
            .filter(|tab| tab.has_changes(cx))
            .map(|tab| (tab.document.clone(), Self::tab_title(tab, cx)))
            .collect()
    }

    /// Asks before quitting whether to save the documents `unkept` — of this window and others —
    /// whose changes no journal keeps: they would be lost. Save saves each in turn, its tab shown,
    /// and quits once all are saved; Don't Save quits.
    pub fn ask_before_quitting(
        &mut self,
        unkept: Vec<(WindowHandle<Workspace>, EntityId, String)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.asking {
            return;
        }
        let files: Vec<String> = unkept.iter().map(|(_, _, name)| fill(Key::CloseFile, &[("file", name)])).collect();
        let detail = fill(Key::CloseQuitDetail, &[("files", &files.join(", "))]);
        let answer = self.ask_to_save(tr(Key::CloseQuitQuestion), &detail, tr(Key::CloseSaveAll), window, cx);
        cx.spawn(async move |_, cx| {
            match answer.await {
                SaveAnswer::Save => {
                    for (window, document, _) in unkept {
                        let save = window.update(cx, |workspace, window, cx| {
                            if let Some(index) = workspace.tab_of(document) {
                                workspace.activate(index, window, cx);
                                window.activate_window();
                            }
                            workspace.save_document(document, window, cx)
                        });
                        let Ok(save) = save else { return };
                        if !save.await {
                            return;
                        }
                    }
                }
                SaveAnswer::DontSave => {}
                SaveAnswer::Cancel => return,
            }
            cx.update(session::quit_now);
        })
        .detach();
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

    /// Closes the window; its tabs go among the closed ones. Changes to save are asked about
    /// first, in one question for all the documents that have them ([ADR 0020]).
    pub fn close_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // The last window on Windows and Linux: closing it is quitting, which keeps the window for
        // the next start, as its button does — once this window is done with the action.
        if !cfg!(target_os = "macos") && cx.windows().len() == 1 {
            cx.defer(session::quit);
            return;
        }
        let changed: Vec<(EntityId, String)> = self
            .tabs
            .iter()
            .filter(|tab| tab.has_changes(cx))
            .map(|tab| (tab.document.entity_id(), Self::tab_title(tab, cx)))
            .collect();
        if changed.is_empty() {
            self.remember_tabs(cx);
            window.remove_window();
            return;
        }
        if self.asking {
            return;
        }
        let (question, detail, save) = match changed.as_slice() {
            [(document, title)] => {
                if let Some(index) = self.tab_of(*document) {
                    self.activate(index, window, cx);
                }
                let question = fill(Key::CloseQuestion, &[("file", title)]);
                (question, tr(Key::CloseDetail).to_owned(), tr(Key::CloseSave))
            }
            _ => {
                let files: Vec<String> =
                    changed.iter().map(|(_, title)| fill(Key::CloseFile, &[("file", title)])).collect();
                let detail = fill(Key::CloseWindowDetail, &[("files", &files.join(", "))]);
                (tr(Key::CloseWindowQuestion).to_owned(), detail, tr(Key::CloseSaveAll))
            }
        };
        let answer = self.ask_to_save(&question, &detail, save, window, cx);
        cx.spawn_in(window, async move |workspace, cx| {
            match answer.await {
                SaveAnswer::Save => {
                    // Each in turn, its tab shown: the dialog of an untitled one and what went
                    // wrong are about the document in view. One not saved keeps the window open.
                    for (document, _) in changed {
                        let save = workspace.update_in(cx, |workspace, window, cx| {
                            if let Some(index) = workspace.tab_of(document) {
                                workspace.activate(index, window, cx);
                            }
                            workspace.save_document(document, window, cx)
                        });
                        let Ok(save) = save else { return };
                        if !save.await {
                            return;
                        }
                    }
                }
                SaveAnswer::DontSave => {
                    let _ = workspace.update(cx, |workspace, _| {
                        for (document, _) in &changed {
                            workspace.discard(*document);
                        }
                    });
                }
                SaveAnswer::Cancel => return,
            }
            let _ = workspace.update_in(cx, |workspace, window, cx| {
                workspace.remember_tabs(cx);
                window.remove_window();
            });
        })
        .detach();
    }

    /// The window closes: it is kept among the closed ones, with its tabs, and comes back whole.
    pub fn remember_tabs(&mut self, cx: &mut App) {
        let tabs = self.tabs.iter().map(|tab| tab.close(cx)).collect();
        windows::remember_window(tabs, self.tabs.active_index(), cx);
    }

    /// Opens what closed `index`-th from the last — the last is 0 — again: a tab here, or a closed
    /// window as a window of its own.
    pub fn reopen_closed(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        match windows::take_closed(index, cx) {
            Some(Closed::Tab(tab)) => self.reopen(tab, window, cx),
            // Once this window is done with the action: a new window is placed and numbered by it.
            Some(Closed::Window { tabs, active }) => cx.defer(move |cx| windows::reopen_window(tabs, active, cx)),
            None => {}
        }
    }

    /// Opens a recent file: in the encoding it had and with the selection where it was, or brings
    /// it forward where it is open. A file that is not there any more leaves the list.
    pub fn open_recent(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let Some(file) = recent::find(path, cx) else { return };
        if !file.path.exists() {
            recent::forget(&file.path, cx);
            return self.notify(notices::recent_gone(&recent::label(&file.path)), cx);
        }
        if let Some(index) = self.find(&file.path, cx) {
            return self.activate(index, window, cx);
        }
        if windows::activate_open(&file.path, cx) {
            return;
        }
        match windows::open_recent(&file, cx) {
            Opening::Document(document, loading) => {
                self.add_tab(ForTab::new(document, loading, Some(file.selection)), window, cx)
            }
            Opening::Failed(notice) => self.notify(notice, cx),
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
        match windows::reopening(closed, cx) {
            Reopening::Tab(tab) => self.add_tab(tab, window, cx),
            Reopening::Failed(notice) => self.notify(notice, cx),
            Reopening::Nothing => {}
        }
    }

    /// Opens a closed tab whose file is open in the tab at `index`.
    fn reopen_over(&mut self, index: usize, closed: ClosedTab, window: &mut Window, cx: &mut Context<Self>) {
        if closed.document.is_none() && closed.journal.is_none() {
            return self.activate(index, window, cx);
        }
        let tab = match windows::reopening(closed, cx) {
            Reopening::Tab(tab) => tab,
            Reopening::Failed(notice) => return self.notify_tab(index, notice, cx),
            Reopening::Nothing => return self.activate(index, window, cx),
        };
        let open_has_changes = self.tabs.get(index).is_some_and(|tab| tab.document.read(cx).is_modified());
        if open_has_changes {
            return self.add_tab(tab, window, cx);
        }
        let replaced = Tab::new(tab.document, None, tab.loading, window, cx);
        // The document replaced had no changes: it goes with its journal.
        let gone = self.tabs.replace(index, replaced);
        gone.document.update(cx, |doc, _| doc.remove_journal());
        self.settle(index, tab.selection, tab.notice, window, cx);
        self.activate(index, window, cx);
    }

    /// The file of the tab at `index`, changed by another program, opens as it is on disk; the
    /// text of the tab goes among the closed tabs with its changes: it is not lost.
    fn load_from_disk(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(index) else { return };
        let Some(path) = tab.document.read(cx).path.clone() else { return };
        let selection = tab.pending_selection.unwrap_or_else(|| tab.editor.read(cx).selection());
        // Its text, if it has changes, goes among the closed tabs, not lost.
        let document = tab.has_changes(cx).then(|| tab.document.clone());
        let encoding = Some(tab.document.read(cx).format.encoding);
        let mine = ClosedTab { document, journal: None, path: Some(path.clone()), encoding, selection };
        match windows::open_document(&path, cx) {
            Opening::Document(document, loading) => {
                let replaced = Tab::new(document, None, loading, window, cx);
                self.tabs.replace(index, replaced);
                windows::remember_tab(mine, cx);
                self.settle(index, Some(selection), None, window, cx);
                self.activate(index, window, cx);
            }
            Opening::Failed(notice) => self.notify_tab(index, notice, cx),
        }
    }

    /// Compares the files of the documents with what they were when read or saved, in the
    /// background, and acts on those that other programs changed or removed.
    fn check_files(&mut self, cx: &mut Context<Self>) {
        let files: Vec<(EntityId, PathBuf, Option<Fingerprint>)> = self
            .tabs
            .iter()
            .filter_map(|tab| {
                let doc = tab.document.read(cx);
                (!doc.is_preview()).then(|| Some((tab.document.entity_id(), doc.path.clone()?, doc.disk)))?
            })
            .collect();
        if files.is_empty() {
            return;
        }
        let check = cx.background_spawn(async move {
            files.into_iter().map(|(id, path, known)| (id, known, Fingerprint::of_path(&path))).collect::<Vec<_>>()
        });
        self.file_check = Some(cx.spawn(async move |workspace, cx| {
            let checked = check.await;
            let _ = workspace.update(cx, |workspace, cx| workspace.files_checked(checked, cx));
        }));
    }

    /// What [`Workspace::check_files`] found: a document without changes whose file changed is
    /// read again, one with changes tells; a file that is gone is marked on its tab.
    fn files_checked(
        &mut self,
        checked: Vec<(EntityId, Option<Fingerprint>, std::io::Result<Fingerprint>)>,
        cx: &mut Context<Self>,
    ) {
        for (document, known, now) in checked {
            let Some(index) = self.tab_of(document) else { continue };
            let Some(tab) = self.tabs.get_mut(index) else { continue };
            let now = match now {
                Ok(now) => now,
                // A file that was there when it was read or saved; a new one is not yet.
                // The text is the only copy now: it has changes to save, which closing asks about and
                // the next start brings back.
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    if known.is_some() && !tab.missing {
                        tab.missing = true;
                        tab.document.update(cx, |doc, cx| {
                            doc.keep_over_file(None);
                            cx.notify();
                        });
                    }
                    continue;
                }
                Err(_) => continue,
            };
            if std::mem::take(&mut tab.missing) {
                cx.notify();
            }
            // Changed since this check began: nothing to tell about the file as it was.
            if Some(now) == known || tab.document.read(cx).disk != known {
                continue;
            }
            if tab.document.read(cx).is_modified() {
                self.tell_changed(index, cx);
            } else {
                self.reload(index, cx);
            }
        }
    }

    /// Tells over the text of the tab at `index` that another program changed its file while it
    /// has changes of its own — once.
    fn tell_changed(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(index) else { return };
        if tab.notices.iter().any(|(_, notice)| notice.topic == Some(Topic::Disk)) {
            return;
        }
        let notice = notices::changed_on_disk(&Self::tab_title(tab, cx));
        self.notify_tab(index, notice, cx);
    }

    /// Reads the file of the tab at `index` again, in the background, into its document: the view
    /// keeps the caret and the scroll. If the document gets changes meanwhile, they stay and the
    /// tab tells of the file instead.
    fn reload(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(index) else { return };
        let Some(path) = tab.document.read(cx).path.clone() else { return };
        let document = tab.document.clone();
        // Saved meanwhile, the document is not the one read over.
        let disk = document.read(cx).disk;
        let journals = journals::dir(cx);
        let read = cx.background_spawn({
            let path = path.clone();
            async move {
                match open(&path, OpenAs::Detect { tld: None })? {
                    Opened::Complete(doc) => Ok(doc),
                    Opened::Partial { loader, .. } => loader.load(),
                }
            }
        });
        cx.spawn(async move |workspace, cx| {
            let read = read.await;
            let _ = workspace.update(cx, |workspace, cx| {
                let Some(index) = workspace.tab_of(document.entity_id()) else { return };
                let mut loaded = match read {
                    Ok(loaded) => loaded,
                    Err(error) => return workspace.notify_tab(index, notices::open_failed(&path, &error), cx),
                };
                let replaced = document.update(cx, |doc, cx| {
                    if doc.path.as_deref() != Some(path.as_path()) || doc.disk != disk {
                        return None;
                    }
                    if doc.is_modified() {
                        return Some(false);
                    }
                    doc.remove_journal();
                    if let Some(dir) = journals {
                        loaded.journal_in(dir);
                    }
                    *doc = loaded;
                    cx.notify();
                    Some(true)
                });
                match replaced {
                    // The text of the file may have bytes lost in reading.
                    Some(true) => {
                        if let Some(tab) = workspace.tabs.get_mut(index) {
                            tab.losses_checked = false;
                        }
                        workspace.check_losses(index, cx);
                    }
                    Some(false) => workspace.tell_changed(index, cx),
                    // Saved, or saved elsewhere, meanwhile: the next check compares again.
                    None => {}
                }
            });
        })
        .detach();
    }

    /// The text of the tab at `index` stays over its file changed by another program: saving
    /// writes it over that.
    fn keep_mine(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(index) else { return };
        tab.document.update(cx, |doc, cx| {
            let disk = doc.path.as_deref().and_then(|path| Fingerprint::of_path(path).ok());
            doc.keep_over_file(disk);
            cx.notify();
        });
    }

    /// The window as the session keeps it.
    fn window_state(&self, cx: &App) -> WindowState {
        let tabs = self
            .tabs
            .iter()
            .map(|tab| {
                let doc = tab.document.read(cx);
                let selection = tab.pending_selection.unwrap_or_else(|| tab.editor.read(cx).selection());
                let encoding = doc.path.as_ref().map(|_| doc.format.encoding);
                TabState { document: Some(doc.id()), path: doc.path.clone(), encoding, selection }
            })
            .collect();
        let (bounds, mode, display) = self.place.clone();
        WindowState { bounds, mode, display, tabs, active: self.tabs.active_index() }
    }

    /// Tells the session how the window is now.
    fn record_session(&self, cx: &mut App) {
        session::record(self.window_id, self.window_state(cx), cx);
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
            self.journal_changed(index, cx);
        }
        self.update_title(window, cx);
        cx.notify();
    }

    /// After a change of the document of the tab at `index`: its journal reaches the disk a moment
    /// after the last edit, and an error that stopped it is told.
    fn journal_changed(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(index) else { return };
        let document = tab.document.downgrade();
        tab.journal_sync = Some(cx.spawn(async move |_, cx| {
            cx.background_executor().timer(journals::SYNC_DELAY).await;
            if let Some(document) = document.upgrade() {
                cx.update(|cx| journals::sync(&document, cx));
            }
        }));
        let Some(error) = tab.document.update(cx, |doc, _| doc.take_journal_error()) else { return };
        let notice = notices::journal_failed(&Self::tab_title(tab, cx), &error);
        self.notify_tab(index, notice, cx);
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
                let tooltip = doc.path.as_ref().map(|path| match tab.missing {
                    true => fill(Key::FileMissing, &[("path", &path.display().to_string())]),
                    false => path.display().to_string(),
                });
                TabInfo {
                    title: Self::tab_title(tab, cx).into(),
                    tooltip: tooltip.map(SharedString::from),
                    modified: doc.is_modified(),
                    missing: tab.missing,
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
        let context_menu = cx.subscribe_in(&editor, window, |workspace, editor, event, window, cx| {
            workspace.open_context_menu(editor, event, window, cx)
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
            discarded: false,
            losses_checked: false,
            journal_sync: None,
            missing: false,
            _observe: observe,
            _progress: progress,
            _context_menu: context_menu,
        }
    }

    /// Whether the document has changes to save, not dropped by the question on closing it.
    fn has_changes(&self, cx: &App) -> bool {
        !self.discarded && self.document.read(cx).is_modified()
    }

    /// Closes the tab: what is kept of it, see [`Tab::closed`]. A document not kept goes with its
    /// journal: what it had is in its file, or was dropped.
    fn close(&self, cx: &mut App) -> ClosedTab {
        let closed = self.closed(cx);
        // The recent files remember where it was and in which encoding.
        let doc = self.document.read(cx);
        if let Some(path) = doc.path.clone() {
            let encoding = doc.format.encoding;
            recent::closed(&path, encoding, closed.selection, cx);
        }
        if closed.document.is_none() {
            self.document.update(cx, |doc, _| doc.remove_journal());
        }
        closed
    }

    /// What is kept of the tab once it is closed: the document, if it has changes to save.
    fn closed(&self, cx: &App) -> ClosedTab {
        let doc = self.document.read(cx);
        ClosedTab {
            document: self.has_changes(cx).then(|| self.document.clone()),
            journal: None,
            path: doc.path.clone(),
            encoding: doc.path.as_ref().map(|_| doc.format.encoding),
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
        // Whatever changed in the window — tabs, documents, selections — the session hears of it.
        self.record_session(cx);
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
        // The selection of the documents shows in full color while the find bar works on them, and
        // that of the text of a context menu while it is open, as the menu has the focus.
        let menu_of = self.context_menu.as_ref().map(|(_, target, _)| *target);
        for tab in self.tabs.iter() {
            let emphasized = self.find_shown || menu_of == Some(tab.editor.entity_id());
            if tab.editor.read(cx).is_emphasized() != emphasized {
                tab.editor.update(cx, |editor, cx| editor.set_emphasized(emphasized, cx));
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
            // Mouse only on macOS, where Option types characters.
            Some(menu_bar) if !view_options::menu_keys() => root.child(menu_bar),
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
            .children(self.find_bar_shown().cloned())
            .children(self.go_to_bar_shown().cloned())
            .children(self.context_menu.as_ref().map(|(menu, ..)| menu.clone()))
            .child(div().flex_1().min_h_0().child(self.editor().clone()))
            .child(status_bar)
            .child(drops)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_context_menu_grays_what_has_nothing_to_do() {
        let none = TextState { editable: true, empty: true, ..TextState::default() };
        let all = ["edit.undo", "edit.redo", "edit.cut", "edit.copy", "edit.paste", "edit.delete", "edit.select_all"];
        assert!(all.iter().all(|id| !none.can(id)), "an empty text without a selection, history or clipboard");
        let selected = TextState { selected: true, ..none };
        assert!(selected.can("edit.cut") && selected.can("edit.copy") && selected.can("edit.delete"));
        let full = TextState { selected: true, can_undo: true, can_redo: true, clipboard: true, ..none };
        assert!(all.iter().filter(|id| **id != "edit.select_all").all(|id| full.can(id)));
        // The preview of a file still loading: copying only.
        let preview = TextState { editable: false, ..full };
        let can: Vec<&str> = all.iter().copied().filter(|id| preview.can(id)).collect();
        assert_eq!(can, ["edit.copy"]);
        assert!(TextState { empty: false, ..none }.can("edit.select_all"));
    }
}
