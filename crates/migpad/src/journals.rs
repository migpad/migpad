//! The journals of the documents: each document keeps one in the folder of the data from its
//! first edit, so that its changes survive a crash. The journal is flushed to the disk a moment
//! after the last edit, when the window goes to the background, and on quitting.

use std::path::PathBuf;
use std::time::Duration;

use gpui::{App, AppContext, Entity, Global};
use migpad_core::data::DataDir;
use migpad_core::document::Document;

/// How long after the last edit the journal is flushed to the disk.
pub const SYNC_DELAY: Duration = Duration::from_millis(1500);

/// The folder of the data, if there is one.
struct Data(Option<DataDir>);

impl Global for Data {}

/// Finds the folder of the data: next to the program in portable mode, at home otherwise. A debug
/// build takes another one from `MIGPAD_DATA_DIR`, for checks, until the settings can tell.
pub fn init(cx: &mut App) {
    let data = match std::env::var_os("MIGPAD_DATA_DIR") {
        Some(dir) if cfg!(debug_assertions) => Some(DataDir::at(PathBuf::from(dir))),
        _ => DataDir::find(),
    };
    if data.is_none() {
        eprintln!("MigPad has no home folder for its data: changes are not kept safe from a crash.");
    }
    cx.set_global(Data(data));
}

/// The folder of the journals.
pub fn dir(cx: &App) -> Option<PathBuf> {
    cx.try_global::<Data>()?.0.as_ref().map(DataDir::journals)
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
