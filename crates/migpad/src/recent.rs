//! The recent files: the menu File ▸ Recent Files, with the encoding of each file and where its
//! selection was when it closed. A file goes first when it is opened or saved; one that is not
//! there any more leaves the list as the menus are built.

use std::path::{Path, PathBuf};

use gpui::{App, Global};
use migpad_core::encoding::Encoding;
use migpad_core::history::Selection;
use migpad_core::state::recent::{RecentFile, RecentFiles};

use crate::commands::refresh_menus;
use crate::journals;
use crate::session;
use crate::strings::{Key, fill};

#[derive(Default)]
struct Recent(RecentFiles);

impl Global for Recent {}

/// Reads the list the program kept.
pub fn init(cx: &mut App) {
    let path = journals::data(cx).map(|data| data.state().join("recent.toml"));
    let mut recent = path.map_or_else(
        || Ok(RecentFiles::default()),
        |path| {
            RecentFiles::read(&path).inspect_err(|error| eprintln!("MigPad could not read the recent files: {error}"))
        },
    );
    if let Ok(recent) = &mut recent {
        recent.forget_missing();
    }
    cx.set_global(Recent(recent.unwrap_or_default()));
}

/// The recent files, the last first.
pub fn files(cx: &App) -> Vec<RecentFile> {
    cx.try_global::<Recent>().map(|recent| recent.0.files().to_vec()).unwrap_or_default()
}

/// The recent file at `path`, if the list has it.
pub fn find(path: &Path, cx: &App) -> Option<RecentFile> {
    cx.try_global::<Recent>()?.0.get(path).cloned()
}

/// The file at `path` was opened in `encoding`: it goes first, where its selection was.
pub fn opened(path: &Path, encoding: Encoding, cx: &mut App) {
    let selection = find(path, cx).map_or_else(|| Selection::caret(0), |file| file.selection);
    change(cx, |recent| recent.touch(RecentFile { path: absolute(path), encoding: name(encoding), selection }));
}

/// The file at `path` was saved in `encoding`, with `selection`: it goes first.
pub fn saved(path: &Path, encoding: Encoding, selection: Selection, cx: &mut App) {
    change(cx, |recent| recent.touch(RecentFile { path: absolute(path), encoding: name(encoding), selection }));
}

/// The tab of the file at `path` closed, in `encoding`, with `selection`: they are remembered for
/// the next time it opens from the list.
pub fn closed(path: &Path, encoding: Encoding, selection: Selection, cx: &mut App) {
    change(cx, |recent| recent.update(path, name(encoding), selection));
}

/// Takes the file at `path` off the list: it is not there any more.
pub fn forget(path: &Path, cx: &mut App) {
    change(cx, |recent| {
        recent.remove(path);
    });
}

/// Empties the list.
pub fn clear(cx: &mut App) {
    change(cx, RecentFiles::clear);
}

/// Takes the files that are not there any more off the list, as the menus are built.
pub fn forget_missing(cx: &mut App) {
    if cx.default_global::<Recent>().0.forget_missing() {
        session::changed(cx);
    }
}

/// The list as the file of the state keeps it.
pub fn to_toml(cx: &App) -> String {
    cx.try_global::<Recent>().map(|recent| recent.0.to_toml()).unwrap_or_else(|| RecentFiles::default().to_toml())
}

/// How the menu shows a file: its name, then its folder, the home folder as `~`.
pub fn label(path: &Path) -> String {
    let name = path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned());
    let Some(folder) = path.parent() else { return name };
    let folder = match std::env::home_dir().and_then(|home| folder.strip_prefix(&home).ok().map(Path::to_path_buf)) {
        Some(inside) if inside.as_os_str().is_empty() => "~".to_owned(),
        Some(inside) => format!("~{}{}", std::path::MAIN_SEPARATOR, inside.display()),
        None => folder.display().to_string(),
    };
    fill(Key::FileRecentLabel, &[("name", &name), ("folder", &folder)])
}

fn change(cx: &mut App, change: impl FnOnce(&mut RecentFiles)) {
    change(&mut cx.default_global::<Recent>().0);
    session::changed(cx);
    refresh_menus(cx);
}

fn name(encoding: Encoding) -> Option<String> {
    Some(encoding.name().to_owned())
}

fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_owned())
}
