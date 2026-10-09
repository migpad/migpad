//! The journals of the documents: each document keeps one in the folder of the data from its
//! first edit, so that its changes survive a crash. The journal is flushed to the disk a moment
//! after the last edit, when the window goes to the background, and on quitting.

use std::fs::File;
use std::path::PathBuf;
use std::time::Duration;

use gpui::{App, AppContext, Entity, Global};
use migpad_core::data::DataDir;
use migpad_core::document::Document;

/// How long after the last edit the journal is flushed to the disk.
pub const SYNC_DELAY: Duration = Duration::from_millis(1500);

/// The folder of the data, if this copy of the program has one.
pub struct Data {
    dir: Option<DataDir>,
    /// The folder found for the program, even if another copy holds it: the settings are read from
    /// there.
    root: Option<PathBuf>,
    /// Holds the folder for this copy of the program while it runs.
    lock: Option<File>,
    /// Another running copy holds the folder: this one keeps nothing there.
    another_copy: bool,
}

impl Global for Data {}

impl Data {
    /// Whether this copy holds the folder: it is the first one.
    pub fn holds_folder(&self) -> bool {
        self.lock.is_some()
    }

    /// Whether another running copy holds the folder.
    pub fn another_copy(&self) -> bool {
        self.another_copy
    }
}

/// The folder of the data — next to the program in portable mode, at home otherwise. A debug
/// build takes another folder from `MIGPAD_DATA_DIR`, for checks, until the settings can tell.
pub fn find() -> Option<DataDir> {
    match std::env::var_os("MIGPAD_DATA_DIR") {
        Some(dir) if cfg!(debug_assertions) => Some(DataDir::at(PathBuf::from(dir))),
        _ => DataDir::find(),
    }
}

/// Takes the folder of the data found for this copy of the program. A second copy, which another
/// one runs before it, keeps nothing there: their journals and sessions would mix.
pub fn take(found: Option<DataDir>) -> Data {
    let root = found.as_ref().map(|dir| dir.root().to_path_buf());
    match found {
        Some(dir) => match dir.lock() {
            Ok(Some(lock)) => Data { dir: Some(dir), root, lock: Some(lock), another_copy: false },
            Ok(None) => Data { dir: None, root, lock: None, another_copy: true },
            // The journals will tell what is wrong with the folder.
            Err(error) => {
                eprintln!("MigPad could not take its folder of data {}: {error}", dir.root().display());
                Data { dir: Some(dir), root, lock: None, another_copy: false }
            }
        },
        None => {
            eprintln!("MigPad has no home folder for its data: changes are not kept safe from a crash.");
            Data { dir: None, root, lock: None, another_copy: false }
        }
    }
}

/// Keeps the folder of the data this copy took for the program.
pub fn init(data: Data, cx: &mut App) {
    cx.set_global(data);
}

/// Whether another running copy of the program holds the folder of the data.
pub fn another_copy(cx: &App) -> bool {
    cx.try_global::<Data>().is_some_and(|data| data.another_copy)
}

/// The folder of the data, if this copy has one.
pub fn data(cx: &App) -> Option<DataDir> {
    cx.try_global::<Data>()?.dir.clone()
}

/// The folder of the data found for the program, even if another running copy holds it.
pub fn root(cx: &App) -> Option<PathBuf> {
    cx.try_global::<Data>()?.root.clone()
}

/// The folder of the journals.
pub fn dir(cx: &App) -> Option<PathBuf> {
    cx.try_global::<Data>()?.dir.as_ref().map(DataDir::journals)
}

/// A new document of the application, which keeps its journal from the first edit.
pub fn document(mut doc: Document, cx: &mut App) -> Entity<Document> {
    if let Some(dir) = dir(cx) {
        doc.journal_in(dir);
    }
    cx.new(|_| doc)
}

/// Flushes the journal of `document` to the disk in the background, if anything was written to
/// it since the last time. A flush that fails stops the journal, which the document tells.
pub fn sync(document: &Entity<Document>, cx: &mut App) {
    let Some(file) = document.update(cx, |doc, _| doc.journal_to_sync()) else { return };
    let flush = cx.background_spawn(async move { file.sync_data() });
    let document = document.downgrade();
    cx.spawn(async move |cx| {
        if let Err(error) = flush.await {
            let _ = document.update(cx, |doc, cx| {
                doc.journal_sync_failed(error);
                cx.notify();
            });
        }
    })
    .detach();
}

/// Flushes the journals of `documents` now: the program is quitting.
pub fn sync_now(documents: &[Entity<Document>], cx: &mut App) {
    for document in documents {
        document.update(cx, |doc, _| doc.sync_journal());
    }
}
