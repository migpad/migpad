//! The session: the windows of the program and their tabs, kept in `state/session.toml` a moment
//! after they change and on quitting, and the tabs closed lately, in `state/closed.toml`. At the
//! next start they open again, each document with its changes and undo history from its journal
//! ([ADR 0020]). A journal with changes that the session does not know — left by a crash right
//! after its tab opened — opens too: nothing is lost.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui::{
    App, AppContext, Bounds, DisplayId, Entity, EntityId, Global, Task, WindowBounds, WindowHandle, WindowId, point,
    px, size,
};
use migpad_core::document::{Document, DocumentId, OpenAs, OpenError, Opened, Recovered, open};
use migpad_core::encoding::Encoding;
use migpad_core::state::closed::{self, ClosedState};
use migpad_core::state::session::{Rect, Session, WindowMode, WindowState};
use migpad_core::state::{TabState, write_atomically};

use crate::journals;
use crate::notices::{self, Notice};
use crate::recent;
use crate::settings;
use crate::strings::{Key, tr};
use crate::windows::{self, ForTab, Opening};
use crate::workspace::Workspace;

/// How long after a change the session is written.
const WRITE_DELAY: Duration = Duration::from_millis(1000);

/// What the session is made of, as the windows tell it, and its writing.
#[derive(Default)]
struct Writer {
    /// The windows in the order they opened, each as it was when it last changed.
    windows: Vec<(WindowId, WindowState)>,
    active: Option<WindowId>,
    /// Writes the files a moment after the last change.
    pending: Option<Task<()>>,
    /// What the files had when they were last written: the session, the closed tabs and the recent
    /// files.
    written: Option<[String; 3]>,
    /// The program is quitting and the session is written for the last time: the windows that
    /// close now stay in it.
    done: bool,
}

impl Global for Writer {}

/// The window `window` is as `state` tells now.
pub fn record(window: WindowId, state: WindowState, cx: &mut App) {
    let writer = cx.default_global::<Writer>();
    if writer.done {
        return;
    }
    match writer.windows.iter_mut().find(|(id, _)| *id == window) {
        Some((_, old)) if *old == state => return,
        Some((_, old)) => *old = state,
        None => writer.windows.push((window, state)),
    }
    schedule(cx);
}

/// The window `window` became active.
pub fn activated(window: WindowId, cx: &mut App) {
    let writer = cx.default_global::<Writer>();
    if writer.done || writer.active == Some(window) {
        return;
    }
    writer.active = Some(window);
    schedule(cx);
}

/// The window `window` closed: it is no longer in the session.
pub fn window_closed(window: WindowId, cx: &mut App) {
    let writer = cx.default_global::<Writer>();
    if writer.done {
        return;
    }
    writer.windows.retain(|(id, _)| *id != window);
    schedule(cx);
}

/// Something else of the state changed: the tabs closed lately, the recent files.
pub fn changed(cx: &mut App) {
    if !cx.default_global::<Writer>().done {
        schedule(cx);
    }
}

fn schedule(cx: &mut App) {
    let task = cx.spawn(async move |cx| {
        cx.background_executor().timer(WRITE_DELAY).await;
        cx.update(write);
    });
    cx.default_global::<Writer>().pending = Some(task);
}

/// Writes the session, the closed tabs and the recent files now, if they changed since they were
/// last written.
fn write(cx: &mut App) {
    let Some(state) = journals::data(cx).map(|data| data.state()) else { return };
    let closed = closed::to_toml(&windows::closed_state(cx));
    let recent = recent::to_toml(cx);
    let writer = cx.default_global::<Writer>();
    if writer.done {
        return;
    }
    let active = writer.active.and_then(|active| writer.windows.iter().position(|(id, _)| *id == active));
    let windows = writer.windows.iter().map(|(_, window)| window.clone()).collect();
    let session = Session { windows, active: active.unwrap_or(0) }.to_toml();
    let files = [session, closed, recent];
    if writer.written.as_ref() == Some(&files) {
        return;
    }
    let result = ["session.toml", "closed.toml", "recent.toml"]
        .iter()
        .zip(&files)
        .try_for_each(|(name, text)| write_atomically(&state.join(name), text));
    match result {
        Ok(()) => writer.written = Some(files),
        Err(error) => eprintln!("MigPad could not write its session: {error}"),
    }
}

/// Quits as the user asks — File ▸ Exit, Cmd+Q, the last window on Windows and Linux — asking
/// nothing, since the journals bring everything back at the next start ([ADR 0020]). Documents
/// whose changes no journal can keep — an error stopped it, or there is no folder of data — are
/// asked about first, in one question in the window of the first of them. Call it once what is
/// being updated now is done, as with `cx.defer`: the documents of every window are needed.
pub fn quit(cx: &mut App) {
    let unkept = unkept_changes(cx);
    match unkept.first().map(|(window, ..)| *window) {
        Some(window) => {
            let _ = window.update(cx, |workspace, window, cx| workspace.ask_before_quitting(unkept, window, cx));
        }
        None => quit_now(cx),
    }
}

/// Quits now, everything left as [`prepare_quit`] leaves it.
pub fn quit_now(cx: &mut App) {
    prepare_quit(cx);
    cx.quit();
}

/// Before the program goes, however it ends: the changes of each document go to a journal if none
/// keeps them yet, the journals reach the disk, and the session is written for the last time.
pub fn prepare_quit(cx: &mut App) {
    let documents = windows::open_documents(cx);
    for document in &documents {
        document.update(cx, |doc, _| doc.keep_changes());
    }
    journals::sync_now(&documents, cx);
    finish(cx);
}

/// The documents with changes that no journal keeps, even after trying to write one now: their
/// windows, the documents, and their names.
fn unkept_changes(cx: &mut App) -> Vec<(WindowHandle<Workspace>, EntityId, String)> {
    let changed: Vec<(WindowHandle<Workspace>, Entity<Document>, String)> = windows::workspaces(cx)
        .flat_map(|(window, workspace)| {
            workspace.changed_documents(cx).into_iter().map(move |(document, name)| (window, document, name))
        })
        .collect();
    changed
        .into_iter()
        .filter(|(_, document, _)| !document.update(cx, |doc, _| doc.keep_changes()))
        .map(|(window, document, name)| (window, document.entity_id(), name))
        .collect()
}

/// The program is quitting: the session is written for the last time, as it is now.
pub fn finish(cx: &mut App) {
    write(cx);
    let writer = cx.default_global::<Writer>();
    writer.done = true;
    writer.pending = None;
}

/// What a document comes back as from its journal.
pub enum Recovery {
    /// The document as it was, and what to tell about it.
    Document(Entity<Document>, Option<Notice>),
    /// The journal has nothing to give: the file is opened as usual.
    FromFile,
    /// The journal could not be read and is set aside: the file is opened as usual, if there is
    /// one, and the notice tells why its changes did not come back.
    Failed(Notice),
}

/// Recovers the document `id` from its journal, if it has one; `path` is its file, as the
/// session knows it.
pub fn recover(id: DocumentId, path: Option<&Path>, cx: &mut App) -> Recovery {
    let Some(journal) = journals::dir(cx).map(|dir| dir.join(format!("{id}.mpj"))) else { return Recovery::FromFile };
    if !journal.exists() {
        return Recovery::FromFile;
    }
    let name = |path: Option<&Path>| match path.and_then(Path::file_name) {
        Some(name) => name.to_string_lossy().into_owned(),
        None => tr(Key::FileUntitled).to_owned(),
    };
    match Document::recover(&journal, open_file) {
        // Nothing was left unsaved, and the file has changed since: it opens as it is now.
        Ok(Recovered::Restored { mut doc, file_changed: true }) if !doc.is_modified() => {
            doc.remove_journal();
            Recovery::FromFile
        }
        Ok(Recovered::Restored { doc, file_changed }) => {
            let notice = file_changed.then(|| notices::changed_while_closed(&name(doc.path.as_deref())));
            Recovery::Document(cx.new(|_| doc), notice)
        }
        // A large file changed, without a copy for the changes to go on: they are set aside.
        Ok(Recovered::Conflict { file }) => {
            set_aside(&journal, cx);
            let notice = notices::changes_set_aside(&name(file.path.as_deref()));
            Recovery::Document(journals::document(file, cx), Some(notice))
        }
        Err(error) => {
            set_aside(&journal, cx);
            Recovery::Failed(notices::recover_failed(&name(path), &error.to_string()))
        }
    }
}

/// Reads the file a journal starts from, in the encoding the journal tells.
fn open_file(path: &Path, encoding: Encoding) -> Result<Document, OpenError> {
    match open(path, OpenAs::Encoding(encoding))? {
        Opened::Complete(doc) => Ok(doc),
        Opened::Partial { loader, .. } => loader.load(),
    }
}

/// Moves a journal that cannot go on to `state/closed`, so that nothing in it is lost.
fn set_aside(journal: &Path, cx: &App) {
    let Some(folder) = journals::data(cx).map(|data| data.state().join("closed")) else { return };
    let moved = std::fs::create_dir_all(&folder)
        .and_then(|()| std::fs::rename(journal, folder.join(journal.file_name().unwrap_or_default())));
    if let Err(error) = moved {
        eprintln!("MigPad could not set aside the journal {}: {error}", journal.display());
    }
}

/// Opens the windows of the session again, with their tabs, the tabs closed lately, and the
/// documents with changes whose journals the session does not know; `paths` open in the active
/// window. Returns that window — `None` if there was nothing to open. Without the setting to bring
/// the session back, only documents with unsaved changes open again, in their windows.
pub fn restore(paths: &[PathBuf], cx: &mut App) -> Option<WindowHandle<Workspace>> {
    let data = journals::data(cx)?;
    let state = data.state();
    let session = Session::read(&state.join("session.toml")).unwrap_or_else(|error| {
        eprintln!("MigPad could not read its session: {error}");
        None
    });
    let closed = closed::read_closed(&state.join("closed.toml")).unwrap_or_else(|error| {
        eprintln!("MigPad could not read the tabs closed lately: {error}");
        Vec::new()
    });
    let session = session.unwrap_or_default();
    let tabs_of = |entry: &ClosedState| match entry {
        ClosedState::Tab(tab) => vec![tab.clone()],
        ClosedState::Window { tabs, .. } => tabs.clone(),
    };
    let mut known: HashSet<DocumentId> =
        session.windows.iter().flat_map(|window| &window.tabs).filter_map(|tab| tab.document).collect();
    known.extend(closed.iter().flat_map(tabs_of).filter_map(|tab| tab.document));
    windows::restore_closed(closed, cx);

    let everything = settings::get(cx).restore_session;
    let mut opened: Vec<(usize, WindowHandle<Workspace>)> = Vec::new();
    for (i, window) in session.windows.iter().enumerate() {
        let (mut tabs, mut notices, mut active) = (Vec::new(), Vec::new(), None);
        for (j, tab) in window.tabs.iter().enumerate() {
            let restored = if everything { restore_tab(tab, cx) } else { restore_changes(tab, cx) };
            match restored {
                Restored::Tab(tab) => {
                    if j == window.active {
                        active = Some(tabs.len());
                    }
                    tabs.push(tab);
                }
                Restored::Notice(notice) => notices.push(notice),
                Restored::Nothing => {}
            }
        }
        if tabs.is_empty() && notices.is_empty() {
            continue;
        }
        let bounds = window_bounds(window, cx);
        if let Some(handle) = windows::open_window_with(tabs, notices, Some(bounds), cx) {
            if let Some(index) = active {
                let _ = handle.update(cx, |workspace, window, cx| workspace.activate(index, window, cx));
            }
            opened.push((i, handle));
        }
    }
    let active = opened.iter().find(|(i, _)| *i == session.active).or(opened.last()).map(|(_, handle)| *handle);

    let orphans = orphans(&data.journals(), &known, cx);
    let active = match active {
        Some(window) => {
            let _ = window.update(cx, |workspace, window, cx| {
                for tab in orphans {
                    workspace.add_tab(tab, window, cx);
                }
                workspace.open_paths(paths, window, cx);
            });
            Some(window)
        }
        None if orphans.is_empty() => return None,
        None => {
            let window = windows::open_window_with(orphans, Vec::new(), None, cx)?;
            let _ = window.update(cx, |workspace, window, cx| workspace.open_paths(paths, window, cx));
            Some(window)
        }
    };
    if let Some(window) = active {
        let _ = window.update(cx, |_, window, _| window.activate_window());
    }
    active
}

/// What a tab of the session opens as.
enum Restored {
    Tab(ForTab),
    /// Nothing to open, and why: told in its window.
    Notice(Notice),
    Nothing,
}

fn restore_tab(tab: &TabState, cx: &mut App) -> Restored {
    let selection = Some(tab.selection);
    let mut failure = None;
    if let Some(id) = tab.document {
        match recover(id, tab.path.as_deref(), cx) {
            Recovery::Document(document, notice) => {
                let encoding_chosen = tab.encoding.is_some();
                return Restored::Tab(ForTab { document, loading: None, selection, notice, encoding_chosen });
            }
            Recovery::Failed(notice) => failure = Some(notice),
            Recovery::FromFile => {}
        }
    }
    // A file without changes that is not there any more has nothing to open.
    let Some(path) = tab.path.as_deref().filter(|path| path.exists()) else {
        return failure.map_or(Restored::Nothing, Restored::Notice);
    };
    // In the encoding chosen for it, not guessed anew.
    let open_as = tab.encoding.map_or(OpenAs::Detect { tld: None }, OpenAs::Encoding);
    match windows::open_document_as(path, open_as, cx) {
        Opening::Document(document, loading) => {
            let encoding_chosen = tab.encoding.is_some();
            Restored::Tab(ForTab { document, loading, selection, notice: failure, encoding_chosen })
        }
        Opening::Failed(notice) => Restored::Notice(notice),
    }
}

/// A tab of the session, if its document has unsaved changes: the session itself is not brought
/// back. The journal of a document without them goes, as the tab does.
fn restore_changes(tab: &TabState, cx: &mut App) -> Restored {
    let Some(id) = tab.document else { return Restored::Nothing };
    match recover(id, tab.path.as_deref(), cx) {
        Recovery::Document(document, notice) if document.read(cx).is_modified() => {
            let encoding_chosen = tab.encoding.is_some();
            Restored::Tab(ForTab { document, loading: None, selection: Some(tab.selection), notice, encoding_chosen })
        }
        // What happened to changes set aside is still told.
        Recovery::Document(document, notice) => {
            document.update(cx, |doc, _| doc.remove_journal());
            notice.map_or(Restored::Nothing, Restored::Notice)
        }
        Recovery::Failed(notice) => Restored::Notice(notice),
        Recovery::FromFile => Restored::Nothing,
    }
}

/// The documents with changes whose journals are in `dir` but not in the session — `known` —
/// recovered; the journals of those without changes go, and the leftovers of rewriting them.
fn orphans(dir: &Path, known: &HashSet<DocumentId>, cx: &mut App) -> Vec<ForTab> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut names: Vec<PathBuf> = entries.filter_map(|entry| Some(entry.ok()?.path())).collect();
    names.sort();
    let mut tabs = Vec::new();
    for path in names {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if name.ends_with(".mpj.tmp") {
            let _ = std::fs::remove_file(&path);
            continue;
        }
        let Some(id) = name.strip_suffix(".mpj").and_then(|id| id.parse::<DocumentId>().ok()) else { continue };
        if known.contains(&id) {
            continue;
        }
        match recover(id, None, cx) {
            Recovery::Document(document, notice) => {
                if document.read(cx).is_modified() {
                    tabs.push(ForTab { document, loading: None, selection: None, notice, encoding_chosen: false });
                } else {
                    document.update(cx, |doc, _| doc.remove_journal());
                }
            }
            Recovery::Failed(notice) => eprintln!("{}", notice.message),
            Recovery::FromFile => {}
        }
    }
    tabs
}

/// Where a window of the session opens: where it was, on its screen if that is still there, or
/// in the middle of the main screen if that place is off every screen now.
fn window_bounds(window: &WindowState, cx: &App) -> (WindowBounds, Option<DisplayId>) {
    let rect = window.bounds;
    let bounds =
        Bounds::new(point(px(rect.x as f32), px(rect.y as f32)), size(px(rect.width as f32), px(rect.height as f32)));
    let display = window.display.as_ref().and_then(|uuid| {
        cx.displays().into_iter().find(|display| display.uuid().is_ok_and(|id| id.to_string() == *uuid))
    });
    let screen = display.clone().or_else(|| cx.primary_display());
    // The title bar on the screen, the place counted from the corner of the screen or not.
    let visible = screen.is_some_and(|screen| {
        let screen = screen.bounds();
        let title = Bounds::new(bounds.origin, size(bounds.size.width.min(px(200.)), px(30.)));
        title.intersects(&Bounds::new(point(px(0.), px(0.)), screen.size)) || title.intersects(&screen)
    });
    let display_id = display.map(|display| display.id());
    let bounds = if visible { bounds } else { Bounds::centered(display_id, bounds.size, cx) };
    let bounds = match window.mode {
        WindowMode::Normal => WindowBounds::Windowed(bounds),
        WindowMode::Maximized => WindowBounds::Maximized(bounds),
        WindowMode::FullScreen => WindowBounds::Fullscreen(bounds),
    };
    (bounds, display_id)
}

/// The place of a window as the session keeps it: the corner of its frame and the size of its
/// content — what a window opens with — and for a maximized or full-screen one, where it goes
/// back to.
pub fn place(window: &gpui::Window, cx: &App) -> (Rect, WindowMode, Option<String>) {
    let (bounds, mode) = match window.window_bounds() {
        WindowBounds::Windowed(_) => (Bounds::new(window.bounds().origin, window.viewport_size()), WindowMode::Normal),
        WindowBounds::Maximized(bounds) => (bounds, WindowMode::Maximized),
        WindowBounds::Fullscreen(bounds) => (bounds, WindowMode::FullScreen),
    };
    let rect = Rect {
        x: f64::from(f32::from(bounds.origin.x)),
        y: f64::from(f32::from(bounds.origin.y)),
        width: f64::from(f32::from(bounds.size.width)),
        height: f64::from(f32::from(bounds.size.height)),
    };
    let display = window.display(cx).and_then(|display| display.uuid().ok()).map(|uuid| uuid.to_string());
    (rect, mode, display)
}
