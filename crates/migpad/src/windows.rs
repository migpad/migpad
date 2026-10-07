//! The windows of the application and the documents in their tabs: opening a window, a file into
//! a tab, and the tabs closed lately.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::Ordering;

use gpui::{
    App, AppContext, Bounds, DisplayId, Entity, Focusable, Global, Pixels, Point, TitlebarOptions, WindowBounds,
    WindowHandle, WindowId, WindowOptions, point, px, size,
};
use migpad_core::document::{Document, DocumentId, OpenAs, OpenError, Opened, open};
use migpad_core::encoding::Encoding;
use migpad_core::history::Selection;
use migpad_core::state::TabState;
use migpad_core::state::closed::ClosedState;
use migpad_core::state::recent::RecentFile;

use crate::commands::{refresh_menus, update_menus};
use crate::journals;
use crate::notices::{self, Notice};
use crate::recent;
use crate::session::{self, Recovery};
use crate::status::Loading;
use crate::strings::{Key, fill, tr};
use crate::tabs::{ClosedTabs, lowest_free};
use crate::workspace::Workspace;

/// The size of a new window.
const WINDOW_SIZE: (f32, f32) = (1000., 700.);
/// How far a new window is from the active one: down and to the right, as a cascade.
const CASCADE: f32 = 26.;

/// A tab closed lately, to open again.
pub struct ClosedTab {
    /// The document, kept while it has changes to save; one without them comes again from its file.
    pub document: Option<Entity<Document>>,
    /// The journal of a document that was kept for its changes when MigPad last quit: the
    /// document comes again from it.
    pub journal: Option<DocumentId>,
    pub path: Option<PathBuf>,
    /// The encoding its file was in: it opens so again, not guessed anew.
    pub encoding: Option<Encoding>,
    pub selection: Selection,
}

impl ClosedTab {
    /// Whether there is something to open again: not an untitled document without text.
    fn is_worth_keeping(&self) -> bool {
        self.has_changes() || self.path.is_some()
    }

    /// Whether it keeps changes to save: there is no other copy of them.
    fn has_changes(&self) -> bool {
        self.document.is_some() || self.journal.is_some()
    }

    fn to_state(&self, cx: &App) -> TabState {
        let document = self.document.as_ref().map(|document| document.read(cx).id()).or(self.journal);
        TabState { document, path: self.path.clone(), encoding: self.encoding, selection: self.selection }
    }

    fn from_state(tab: TabState) -> ClosedTab {
        let (journal, path, encoding, selection) = (tab.document, tab.path, tab.encoding, tab.selection);
        ClosedTab { document: None, journal, path, encoding, selection }
    }
}

/// What closed lately: a tab, or a window with its tabs, which comes back whole.
pub enum Closed {
    Tab(ClosedTab),
    /// The tabs of a closed window, in their order, and which one was active.
    Window {
        tabs: Vec<ClosedTab>,
        active: usize,
    },
}

impl Closed {
    fn tabs(&self) -> &[ClosedTab] {
        match self {
            Closed::Tab(tab) => std::slice::from_ref(tab),
            Closed::Window { tabs, .. } => tabs,
        }
    }

    /// Whether it keeps changes to save: there is no other copy of them.
    fn has_changes(&self) -> bool {
        self.tabs().iter().any(ClosedTab::has_changes)
    }
}

/// The tabs and windows closed lately.
#[derive(Default)]
struct ClosedList(ClosedTabs<Closed>);

impl Global for ClosedList {}

fn remember(closed: Closed, cx: &mut App) {
    // Changes to save are kept however many tabs close after them.
    cx.default_global::<ClosedList>().0.push(closed, Closed::has_changes);
    session::changed(cx);
    refresh_menus(cx);
}

/// Remembers a closed tab, unless there is nothing to open again.
pub fn remember_tab(tab: ClosedTab, cx: &mut App) {
    if tab.is_worth_keeping() {
        remember(Closed::Tab(tab), cx);
    }
}

/// Remembers a closed window with those of its tabs there is something to open again in.
pub fn remember_window(tabs: Vec<ClosedTab>, active: usize, cx: &mut App) {
    let active = tabs[..active.min(tabs.len())].iter().filter(|tab| tab.is_worth_keeping()).count();
    let tabs: Vec<ClosedTab> = tabs.into_iter().filter(ClosedTab::is_worth_keeping).collect();
    if !tabs.is_empty() {
        let active = active.min(tabs.len() - 1);
        remember(Closed::Window { tabs, active }, cx);
    }
}

/// The tab or the window closed `index`-th from the last — the last is 0 — if any.
pub fn take_closed(index: usize, cx: &mut App) -> Option<Closed> {
    let closed = cx.default_global::<ClosedList>().0.take(index);
    session::changed(cx);
    refresh_menus(cx);
    closed
}

/// The tabs and windows closed lately as File ▸ Recently Closed shows them, the last first: a
/// file by its name and folder, a window by the names of its tabs.
pub fn closed_labels(cx: &App) -> Vec<String> {
    let Some(closed) = cx.try_global::<ClosedList>() else { return Vec::new() };
    let name = |tab: &ClosedTab| match tab.path.as_deref().and_then(Path::file_name) {
        Some(name) => name.to_string_lossy().into_owned(),
        None => tr(Key::FileUntitled).to_owned(),
    };
    closed
        .0
        .iter()
        .map(|closed| match closed {
            Closed::Tab(tab) => tab.path.as_deref().map_or_else(|| name(tab), recent::label),
            Closed::Window { tabs, .. } => {
                let mut names: Vec<String> = tabs.iter().take(3).map(name).collect();
                if tabs.len() > 3 {
                    names.push("…".to_owned());
                }
                fill(Key::FileClosedWindow, &[("tabs", &names.join(", "))])
            }
        })
        .collect()
}

/// The tabs and windows closed lately as the state keeps them, the last closed first.
pub fn closed_state(cx: &App) -> Vec<ClosedState> {
    let Some(closed) = cx.try_global::<ClosedList>() else { return Vec::new() };
    let tabs = |tabs: &[ClosedTab]| tabs.iter().map(|tab| tab.to_state(cx)).collect();
    closed
        .0
        .iter()
        .map(|closed| match closed {
            Closed::Tab(tab) => ClosedState::Tab(tab.to_state(cx)),
            Closed::Window { tabs: closed, active } => ClosedState::Window { tabs: tabs(closed), active: *active },
        })
        .collect()
}

/// The tabs and windows closed lately, as the state kept them, the last closed first.
pub fn restore_closed(closed: Vec<ClosedState>, cx: &mut App) {
    let mut list = ClosedList::default();
    // The oldest first, as they closed.
    for entry in closed.into_iter().rev() {
        let entry = match entry {
            ClosedState::Tab(tab) => Closed::Tab(ClosedTab::from_state(tab)),
            ClosedState::Window { tabs, active } => {
                Closed::Window { tabs: tabs.into_iter().map(ClosedTab::from_state).collect(), active }
            }
        };
        list.0.push(entry, Closed::has_changes);
    }
    cx.set_global(list);
}

/// The documents of the program: those in the windows, and those of closed tabs kept for their
/// changes.
pub fn open_documents(cx: &App) -> Vec<Entity<Document>> {
    let open = workspaces(cx).flat_map(|(_, workspace)| workspace.documents().cloned().collect::<Vec<_>>());
    let closed = cx.try_global::<ClosedList>().into_iter().flat_map(|closed| closed.0.iter());
    let closed = closed.flat_map(|closed| closed.tabs().iter().filter_map(|tab| tab.document.clone()));
    open.chain(closed).collect()
}

/// The workspaces of the open windows, except one that is being updated now.
pub fn workspaces(cx: &App) -> impl Iterator<Item = (WindowHandle<Workspace>, &Workspace)> {
    cx.windows().into_iter().filter_map(|window| {
        let window = window.downcast::<Workspace>()?;
        Some((window, window.read(cx).ok()?))
    })
}

/// The number for a new untitled document: the lowest that no untitled document has, in this
/// window — `own` — and in the others.
pub fn untitled_number(own: impl IntoIterator<Item = u32>, own_window: Option<WindowId>, cx: &App) -> u32 {
    let others = workspaces(cx)
        .filter(|(window, _)| Some(window.window_id()) != own_window)
        .flat_map(|(_, workspace)| workspace.untitled_numbers().collect::<Vec<_>>());
    lowest_free(own.into_iter().chain(others))
}

/// What a path is opened as.
pub enum Opening {
    /// The document; a large one still loading in the background, as far as `Loading` tells.
    Document(Entity<Document>, Option<Loading>),
    Failed(Notice),
}

/// A document for a tab, where its view puts the selection, and what the tab tells over its text.
pub struct ForTab {
    pub document: Entity<Document>,
    pub loading: Option<Loading>,
    pub selection: Option<Selection>,
    pub notice: Option<Notice>,
}

impl ForTab {
    /// A tab for `document`, which tells nothing.
    pub fn new(document: Entity<Document>, loading: Option<Loading>, selection: Option<Selection>) -> Self {
        ForTab { document, loading, selection, notice: None }
    }
}

/// Opens the file at `path` that the user chose: in a document, as [`open_document`] does, and
/// among the recent files, of the system and of MigPad.
pub fn open_file(path: &Path, cx: &mut App) -> Opening {
    open_file_as(path, OpenAs::Detect { tld: None }, cx)
}

/// Opens the file at `path` as [`open_file`] does, as `open_as` tells.
pub fn open_file_as(path: &Path, open_as: OpenAs, cx: &mut App) -> Opening {
    let opening = open_document_as(path, open_as, cx);
    note_opened(path, &opening, cx);
    opening
}

/// Opens a recent file in the encoding it had.
pub fn open_recent(file: &RecentFile, cx: &mut App) -> Opening {
    let open_as =
        file.encoding.as_deref().and_then(Encoding::for_name).map_or(OpenAs::Detect { tld: None }, OpenAs::Encoding);
    let opening = open_document_as(&file.path, open_as, cx);
    note_opened(&file.path, &opening, cx);
    opening
}

fn note_opened(path: &Path, opening: &Opening, cx: &mut App) {
    if let Opening::Document(document, _) = opening
        && let Some(path) = document.read(cx).path.clone().or_else(|| std::path::absolute(path).ok())
    {
        let encoding = document.read(cx).format.encoding;
        cx.add_recent_document(&path);
        recent::opened(&path, encoding, cx);
    }
}

/// Opens the file at `path` into a document as `open_as` tells — in the encoding found, or in a
/// given one: a large one shows its beginning at once and the whole text once the background load
/// is done.
pub fn open_document_as(path: &Path, open_as: OpenAs, cx: &mut App) -> Opening {
    let path = std::path::absolute(path).unwrap_or_else(|_| path.to_owned());
    match open(&path, open_as) {
        Ok(Opened::Complete(loaded)) => Opening::Document(journals::document(loaded, cx), None),
        Ok(Opened::Partial { preview, loader }) => {
            let document = cx.new(|_| preview);
            let progress = Loading {
                read: loader.progress(),
                total: loader.file_len(),
                stopped: Rc::new(Cell::new(false)),
                failure: Rc::new(RefCell::new(None)),
            };
            // A tab closed while its file loads stops the loading.
            let cancel = loader.cancel_flag();
            cx.observe_release(&document, move |_, _| cancel.store(true, Ordering::Relaxed)).detach();
            let loading = cx.background_executor().spawn(async move { loader.load() });
            let (target, stopped, failure) = (document.downgrade(), progress.stopped.clone(), progress.failure.clone());
            let journals = journals::dir(cx);
            cx.spawn(async move |cx| {
                match loading.await {
                    Ok(mut loaded) => {
                        // The preview could not be edited; the whole text can, with a journal.
                        if let Some(dir) = journals {
                            loaded.journal_in(dir);
                        }
                        let _ = target.update(cx, |doc, cx| {
                            *doc = loaded;
                            cx.notify();
                        });
                    }
                    Err(OpenError::Cancelled) => {}
                    // The beginning stays, read-only, and the tab tells why the rest did not come.
                    Err(error) => *failure.borrow_mut() = Some(notices::load_failed(&path, &error)),
                }
                stopped.set(true);
            })
            .detach();
            Opening::Document(document, Some(progress))
        }
        // A file that is not there yet: an empty document, which saving creates.
        Err(OpenError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound && !path.is_dir() => {
            let mut document = Document::new();
            document.path = Some(path);
            Opening::Document(journals::document(document, cx), None)
        }
        Err(error) => Opening::Failed(notices::open_failed(&path, &error)),
    }
}

/// What a closed tab opens again as.
pub enum Reopening {
    Tab(ForTab),
    /// It could not open: why.
    Failed(Notice),
    /// There is nothing to open.
    Nothing,
}

/// Opens a closed tab again: its document kept for its changes, or recovered from its journal,
/// or read from its file.
pub fn reopening(closed: ClosedTab, cx: &mut App) -> Reopening {
    let selection = Some(closed.selection);
    if let Some(document) = closed.document {
        return Reopening::Tab(ForTab::new(document, None, selection));
    }
    let mut failure = None;
    if let Some(id) = closed.journal {
        match session::recover(id, closed.path.as_deref(), cx) {
            Recovery::Document(document, notice) => {
                return Reopening::Tab(ForTab { document, loading: None, selection, notice });
            }
            Recovery::Failed(notice) => failure = Some(notice),
            Recovery::FromFile => {}
        }
    }
    let Some(path) = closed.path else { return failure.map_or(Reopening::Nothing, Reopening::Failed) };
    let open_as = closed.encoding.map_or(OpenAs::Detect { tld: None }, OpenAs::Encoding);
    match open_file_as(&path, open_as, cx) {
        Opening::Document(document, loading) => {
            Reopening::Tab(ForTab { document, loading, selection, notice: failure })
        }
        Opening::Failed(notice) => Reopening::Failed(notice),
    }
}

/// Whether two paths are the same file: the same path once symbolic links and `..` are resolved,
/// or the same absolute path for a file that does not exist.
pub fn same_file(a: &Path, b: &Path) -> bool {
    let resolve = |path: &Path| {
        std::fs::canonicalize(path).or_else(|_| std::path::absolute(path)).unwrap_or_else(|_| path.to_owned())
    };
    a == b || resolve(a) == resolve(b)
}

/// The window used last: the active one, or the one in front, or any.
pub fn last_active(cx: &App) -> Option<WindowHandle<Workspace>> {
    let active = cx.active_window().and_then(|window| window.downcast::<Workspace>());
    let in_front = || cx.window_stack().into_iter().flatten().find_map(|window| window.downcast::<Workspace>());
    active.or_else(in_front).or_else(|| workspaces(cx).next().map(|(window, _)| window))
}

/// The window and the tab that have the file at `path` open, if one does.
pub fn find_open(path: &Path, cx: &App) -> Option<(WindowHandle<Workspace>, usize)> {
    workspaces(cx).find_map(|(window, workspace)| Some((window, workspace.find(path, cx)?)))
}

/// Opens a window with a tab for each of `paths`, the last one active, or an untitled one; a path
/// already open in a window brings that tab forward instead, and none opens if all of them are.
pub fn open_window(paths: &[PathBuf], cx: &mut App) -> Option<WindowHandle<Workspace>> {
    let mut tabs = Vec::new();
    let mut opened: Vec<&Path> = Vec::new();
    let mut failures = Vec::new();
    for path in paths {
        if activate_open(path, cx) || opened.iter().any(|opened| same_file(opened, path)) {
            continue;
        }
        opened.push(path);
        match open_file(path, cx) {
            Opening::Document(document, loading) => tabs.push(ForTab::new(document, loading, None)),
            Opening::Failed(notice) => failures.push(notice),
        }
    }
    if !paths.is_empty() && opened.is_empty() {
        return None;
    }
    open_window_with(tabs, failures, None, cx)
}

/// Opens a window with `tabs`, the last one active, or an untitled one, and `notices` in the tab
/// shown; at `place` and on its screen, or down and to the right of the active window.
pub fn open_window_with(
    tabs: Vec<ForTab>,
    notices: Vec<Notice>,
    place: Option<(WindowBounds, Option<DisplayId>)>,
    cx: &mut App,
) -> Option<WindowHandle<Workspace>> {
    let options = window_options(place, cx);
    let window = cx
        .open_window(options, |window, cx| {
            let workspace = cx.new(|cx| {
                let mut tabs = tabs.into_iter();
                let first =
                    tabs.next().unwrap_or_else(|| ForTab::new(journals::document(Document::new(), cx), None, None));
                let mut workspace = Workspace::new(first, window, cx);
                for tab in tabs {
                    workspace.add_tab(tab, window, cx);
                }
                for notice in notices {
                    workspace.notify(notice, cx);
                }
                workspace
            });
            window.focus(&workspace.focus_handle(cx), cx);
            workspace
        })
        .map_err(|error| eprintln!("MigPad could not open a window: {error:#}"))
        .ok()?;
    window
        .update(cx, |_, window, cx| {
            // The button of the window closes it as Close Window does, asking about changes to
            // save first — but the last window on Windows and Linux, which is quitting: then
            // nothing is asked, the journals stay, and the window comes back at the next start
            // with everything in it ([ADR 0020]).
            window.on_window_should_close(cx, |window, cx| {
                let Some(Some(workspace)) = window.root::<Workspace>() else { return true };
                // Quitting goes on once this window is done with the question whether to close.
                if !cfg!(target_os = "macos") && cx.windows().len() == 1 {
                    cx.defer(session::quit);
                    return false;
                }
                if workspace.read(cx).has_changes(cx) {
                    workspace.update(cx, |workspace, cx| workspace.close_window(window, cx));
                    return false;
                }
                workspace.update(cx, |workspace, cx| workspace.remember_tabs(cx));
                true
            });
        })
        .ok();
    update_menus(None, cx);
    Some(window)
}

/// Opens what closed `index`-th from the last without a window to put it in: a closed window
/// again, or a tab in a new window — what Reopen Closed Tab does without windows.
pub fn reopen_in_new_window(index: usize, cx: &mut App) {
    let closed = match take_closed(index, cx) {
        Some(Closed::Tab(tab)) => tab,
        Some(Closed::Window { tabs, active }) => return reopen_window(tabs, active, cx),
        None => return,
    };
    if let Some((window, _)) = closed.path.as_deref().and_then(|path| find_open(path, cx)) {
        // The file is open in a window: the tab comes there.
        let _ = window.update(cx, |workspace, window, cx| {
            workspace.reopen(closed, window, cx);
            window.activate_window();
        });
        return;
    }
    let (tabs, notices) = match reopening(closed, cx) {
        Reopening::Tab(tab) => (vec![tab], Vec::new()),
        Reopening::Failed(notice) => (Vec::new(), vec![notice]),
        Reopening::Nothing => return,
    };
    open_window_with(tabs, notices, None, cx);
}

/// Opens a closed window again, with its tabs and the one that was active. A file opened in another
/// window since stays there, unless its tab kept changes to save.
pub fn reopen_window(tabs: Vec<ClosedTab>, active: usize, cx: &mut App) {
    let mut reopened = Vec::new();
    let mut notices = Vec::new();
    let mut active_tab = None;
    for (i, closed) in tabs.into_iter().enumerate() {
        if !closed.has_changes() && closed.path.as_deref().is_some_and(|path| find_open(path, cx).is_some()) {
            continue;
        }
        let tab = match reopening(closed, cx) {
            Reopening::Tab(tab) => tab,
            Reopening::Failed(notice) => {
                notices.push(notice);
                continue;
            }
            Reopening::Nothing => continue,
        };
        if i == active {
            active_tab = Some(reopened.len());
        }
        reopened.push(tab);
    }
    if reopened.is_empty() && notices.is_empty() {
        return;
    }
    if let Some(window) = open_window_with(reopened, notices, None, cx)
        && let Some(index) = active_tab
    {
        let _ = window.update(cx, |workspace, window, cx| workspace.activate(index, window, cx));
    }
}

/// Opens a recent file without a window to put it in: in a new window, or where it is open.
pub fn open_recent_in_new_window(path: &Path, cx: &mut App) {
    let Some(file) = recent::find(path, cx) else { return };
    if activate_open(&file.path, cx) {
        return;
    }
    let (tabs, notices) = if !file.path.exists() {
        recent::forget(&file.path, cx);
        (Vec::new(), vec![notices::recent_gone(&recent::label(&file.path))])
    } else {
        match open_recent(&file, cx) {
            Opening::Document(document, loading) => {
                (vec![ForTab::new(document, loading, Some(file.selection))], Vec::new())
            }
            Opening::Failed(notice) => (Vec::new(), vec![notice]),
        }
    };
    open_window_with(tabs, notices, None, cx);
}

/// Brings forward the tab with the file at `path`, if a window has it open.
pub fn activate_open(path: &Path, cx: &mut App) -> bool {
    let Some((window, index)) = find_open(path, cx) else { return false };
    window
        .update(cx, |workspace, window, cx| {
            workspace.activate(index, window, cx);
            window.activate_window();
        })
        .is_ok()
}

/// A new window: at `place` and on its screen, or down and to the right of the active one, or in
/// the middle of the screen.
fn window_options(place: Option<(WindowBounds, Option<DisplayId>)>, cx: &mut App) -> WindowOptions {
    let titlebar = Some(TitlebarOptions { title: Some("MigPad".into()), ..Default::default() });
    if let Some((window_bounds, display_id)) = place {
        return WindowOptions { window_bounds: Some(window_bounds), display_id, titlebar, ..Default::default() };
    }
    let size = size(px(WINDOW_SIZE.0), px(WINDOW_SIZE.1));
    let bounds = cx
        .active_window()
        // The size of the content: the bounds of a window have its title bar too, and a window
        // opened with them would be taller.
        .and_then(|window| {
            window.update(cx, |_, window, _| Bounds::new(window.bounds().origin, window.viewport_size())).ok()
        })
        .map(|active: Bounds<Pixels>| {
            let origin: Point<Pixels> = active.origin + point(px(CASCADE), px(CASCADE));
            Bounds::new(origin, active.size)
        })
        .unwrap_or_else(|| Bounds::centered(None, size, cx));
    WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), titlebar, ..Default::default() }
}
