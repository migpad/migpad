//! The windows of the application and the documents in their tabs: opening a window, a file into
//! a tab, and the tabs closed lately.

use std::path::{Path, PathBuf};

use gpui::{
    App, AppContext, Bounds, Entity, Focusable, Global, Pixels, Point, TitlebarOptions, WindowBounds, WindowHandle,
    WindowId, WindowOptions, point, px, size,
};
use migpad_core::document::{Document, OpenAs, OpenError, Opened, open};
use migpad_core::history::Selection;

use crate::commands::update_menus;
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

/// The tabs closed lately, of all windows.
#[derive(Default)]
struct Closed(ClosedTabs<ClosedTab>);

impl Global for Closed {}

/// Remembers a closed tab, unless there is nothing to open again: an untitled document without text.
pub fn remember_closed(tab: ClosedTab, cx: &mut App) {
    if tab.document.is_some() || tab.path.is_some() {
        cx.default_global::<Closed>().0.push(tab);
    }
}

/// Whether there is a closed tab to open again.
pub fn has_closed(cx: &App) -> bool {
    cx.try_global::<Closed>().is_some_and(|closed| !closed.0.is_empty())
}

/// The tab closed last, if any.
pub fn take_closed(cx: &mut App) -> Option<ClosedTab> {
    cx.default_global::<Closed>().0.pop()
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
    /// The document; a large one still loading in the background.
    Document(Entity<Document>),
    Failed(PathBuf, OpenError),
}

/// Opens the file at `path` into a document: a large one shows its beginning at once and the
/// whole text once the background load is done.
pub fn open_document(path: &Path, cx: &mut App) -> Opening {
    let path = std::path::absolute(path).unwrap_or_else(|_| path.to_owned());
    match open(&path, OpenAs::Detect { tld: None }) {
        Ok(Opened::Complete(loaded)) => Opening::Document(cx.new(|_| loaded)),
        Ok(Opened::Partial { preview, loader }) => {
            let document = cx.new(|_| preview);
            let loading = cx.background_executor().spawn(async move { loader.load() });
            let target = document.clone();
            cx.spawn(async move |cx| match loading.await {
                Ok(loaded) => target.update(cx, |doc, cx| {
                    *doc = loaded;
                    cx.notify();
                }),
                Err(error) => eprintln!("{}: {error}", path.display()),
            })
            .detach();
            Opening::Document(document)
        }
        Err(error) => Opening::Failed(path, error),
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

/// Opens a window with a tab for each of `paths`, the last one active, or an untitled one; a path
/// already open in a window brings that tab forward instead.
pub fn open_window(paths: &[PathBuf], cx: &mut App) -> Option<WindowHandle<Workspace>> {
    let mut opened = Vec::new();
    for path in paths {
        if activate_open(path, cx) {
            continue;
        }
        if opened.iter().any(|(_, opened_path): &(Opening, PathBuf)| same_file(opened_path, path)) {
            continue;
        }
        opened.push((open_document(path, cx), path.clone()));
    }
    if !paths.is_empty() && opened.is_empty() {
        return None;
    }
    let documents: Vec<Entity<Document>> = opened
        .into_iter()
        .filter_map(|(opening, _)| match opening {
            Opening::Document(document) => Some(document),
            Opening::Failed(path, error) => {
                eprintln!("{}: {error}", path.display());
                None
            }
        })
        .collect();
    let options = window_options(cx);
    let window = cx
        .open_window(options, |window, cx| {
            let workspace = cx.new(|cx| {
                let mut documents = documents.into_iter();
                let first = documents.next();
                let untitled = first.is_none().then(|| untitled_number([], None, cx));
                let first = first.unwrap_or_else(|| cx.new(|_| Document::new()));
                let mut workspace = Workspace::new(first, untitled, window, cx);
                for document in documents {
                    workspace.add_tab(document, None, window, cx);
                }
                workspace
            });
            window.focus(&workspace.focus_handle(cx), cx);
            workspace
        })
        .ok()?;
    window
        .update(cx, |_, window, cx| {
            // Closing the window with its button keeps its tabs among the closed ones.
            window.on_window_should_close(cx, |window, cx| {
                if let Some(Some(workspace)) = window.root::<Workspace>() {
                    workspace.update(cx, |workspace, cx| workspace.remember_tabs(cx));
                }
                true
            });
        })
        .ok();
    update_menus(None, cx);
    Some(window)
}

/// Brings forward the tab with the file at `path`, if a window has it open.
fn activate_open(path: &Path, cx: &mut App) -> bool {
    let found = workspaces(cx).find_map(|(window, workspace)| Some((window, workspace.find(path, cx)?)));
    let Some((window, index)) = found else { return false };
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
        .and_then(|window| window.update(cx, |_, window, _| window.bounds()).ok())
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
