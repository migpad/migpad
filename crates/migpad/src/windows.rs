//! The windows of the application and the documents in their tabs: opening a window, a file into
//! a tab, and the tabs closed lately.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::Ordering;

use gpui::{
    App, AppContext, Bounds, Entity, Focusable, Global, Pixels, Point, TitlebarOptions, WindowBounds, WindowHandle,
    WindowId, WindowOptions, point, px, size,
};
use migpad_core::document::{Document, OpenAs, OpenError, Opened, open};
use migpad_core::history::Selection;

use crate::commands::update_menus;
use crate::notices::{self, Notice};
use crate::status::Loading;
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
    pub path: Option<PathBuf>,
    pub selection: Selection,
}

impl ClosedTab {
    /// Whether there is something to open again: not an untitled document without text.
    fn is_worth_keeping(&self) -> bool {
        self.document.is_some() || self.path.is_some()
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
    /// Whether it keeps changes to save: there is no other copy of them.
    fn has_changes(&self) -> bool {
        match self {
            Closed::Tab(tab) => tab.document.is_some(),
            Closed::Window { tabs, .. } => tabs.iter().any(|tab| tab.document.is_some()),
        }
    }
}

/// The tabs and windows closed lately.
#[derive(Default)]
struct ClosedList(ClosedTabs<Closed>);

impl Global for ClosedList {}

fn remember(closed: Closed, cx: &mut App) {
    // Changes to save are kept however many tabs close after them.
    cx.default_global::<ClosedList>().0.push(closed, Closed::has_changes);
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

/// The tab or the window closed last, if any.
pub fn take_closed(cx: &mut App) -> Option<Closed> {
    cx.default_global::<ClosedList>().0.pop()
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

/// A document for a tab of a new window, and where its view puts the selection.
pub struct ForTab {
    pub document: Entity<Document>,
    pub loading: Option<Loading>,
    pub selection: Option<Selection>,
}

/// Opens the file at `path` into a document: a large one shows its beginning at once and the
/// whole text once the background load is done.
pub fn open_document(path: &Path, cx: &mut App) -> Opening {
    let path = std::path::absolute(path).unwrap_or_else(|_| path.to_owned());
    match open(&path, OpenAs::Detect { tld: None }) {
        Ok(Opened::Complete(loaded)) => {
            cx.add_recent_document(&path);
            Opening::Document(cx.new(|_| loaded), None)
        }
        Ok(Opened::Partial { preview, loader }) => {
            cx.add_recent_document(&path);
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
            cx.spawn(async move |cx| {
                match loading.await {
                    Ok(loaded) => {
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
            Opening::Document(cx.new(|_| document), None)
        }
        Err(error) => Opening::Failed(notices::open_failed(&path, &error)),
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
        match open_document(path, cx) {
            Opening::Document(document, loading) => tabs.push(ForTab { document, loading, selection: None }),
            Opening::Failed(notice) => failures.push(notice),
        }
    }
    if !paths.is_empty() && opened.is_empty() {
        return None;
    }
    open_window_with(tabs, failures, cx)
}

/// Opens a window with `tabs`, the last one active, or an untitled one, and `notices` in the tab
/// shown.
pub fn open_window_with(tabs: Vec<ForTab>, notices: Vec<Notice>, cx: &mut App) -> Option<WindowHandle<Workspace>> {
    let options = window_options(cx);
    let window = cx
        .open_window(options, |window, cx| {
            let workspace = cx.new(|cx| {
                let mut tabs = tabs.into_iter();
                let first = tabs.next().unwrap_or_else(|| ForTab {
                    document: cx.new(|_| Document::new()),
                    loading: None,
                    selection: None,
                });
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
            // nothing is asked and everything comes back at the next start ([ADR 0020]).
            window.on_window_should_close(cx, |window, cx| {
                let Some(Some(workspace)) = window.root::<Workspace>() else { return true };
                let quitting = !cfg!(target_os = "macos") && cx.windows().len() == 1;
                if quitting || !workspace.read(cx).has_changes(cx) {
                    workspace.update(cx, |workspace, cx| workspace.remember_tabs(cx));
                    return true;
                }
                workspace.update(cx, |workspace, cx| workspace.close_window(window, cx));
                false
            });
        })
        .ok();
    update_menus(None, cx);
    Some(window)
}

/// Opens what closed last without a window to put it in: a closed window again, or a tab in a new
/// window — what Reopen Closed Tab does without windows.
pub fn reopen_in_new_window(cx: &mut App) {
    let closed = match take_closed(cx) {
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
    let selection = Some(closed.selection);
    let (tabs, notices) = match (closed.document, closed.path) {
        (Some(document), _) => (vec![ForTab { document, loading: None, selection }], Vec::new()),
        (None, Some(path)) => match open_document(&path, cx) {
            Opening::Document(document, loading) => (vec![ForTab { document, loading, selection }], Vec::new()),
            Opening::Failed(notice) => (Vec::new(), vec![notice]),
        },
        (None, None) => return,
    };
    open_window_with(tabs, notices, cx);
}

/// Opens a closed window again, with its tabs and the one that was active. A file opened in another
/// window since stays there, unless its tab kept changes to save.
pub fn reopen_window(tabs: Vec<ClosedTab>, active: usize, cx: &mut App) {
    let mut reopened = Vec::new();
    let mut notices = Vec::new();
    let mut active_tab = None;
    for (i, closed) in tabs.into_iter().enumerate() {
        if closed.document.is_none() && closed.path.as_deref().is_some_and(|path| find_open(path, cx).is_some()) {
            continue;
        }
        let selection = Some(closed.selection);
        let tab = match (closed.document, closed.path) {
            (Some(document), _) => ForTab { document, loading: None, selection },
            (None, Some(path)) => match open_document(&path, cx) {
                Opening::Document(document, loading) => ForTab { document, loading, selection },
                Opening::Failed(notice) => {
                    notices.push(notice);
                    continue;
                }
            },
            (None, None) => continue,
        };
        if i == active {
            active_tab = Some(reopened.len());
        }
        reopened.push(tab);
    }
    if reopened.is_empty() && notices.is_empty() {
        return;
    }
    if let Some(window) = open_window_with(reopened, notices, cx)
        && let Some(index) = active_tab
    {
        let _ = window.update(cx, |workspace, window, cx| workspace.activate(index, window, cx));
    }
}

/// Brings forward the tab with the file at `path`, if a window has it open.
fn activate_open(path: &Path, cx: &mut App) -> bool {
    let Some((window, index)) = find_open(path, cx) else { return false };
    window
        .update(cx, |workspace, window, cx| {
            workspace.activate(index, window, cx);
            window.activate_window();
        })
        .is_ok()
}

/// A new window: down and to the right of the active one, or in the middle of the screen.
fn window_options(cx: &mut App) -> WindowOptions {
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
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions { title: Some("MigPad".into()), ..Default::default() }),
        ..Default::default()
    }
}
