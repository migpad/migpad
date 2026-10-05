//! MigPad, a fast cross-platform text editor.

// Release builds on Windows are GUI applications: no console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod debug_input;

use std::path::{Path, PathBuf};

use gpui::{App, AppContext, Bounds, Entity, Focusable, TitlebarOptions, WindowBounds, WindowOptions, px, size};
use migpad_core::document::{Document, OpenAs, Opened, open};
use migpad_editor::EditorView;

fn main() {
    let path = std::env::args_os().nth(1).map(PathBuf::from);
    gpui_platform::application().run(move |cx: &mut App| {
        migpad_editor::init(cx);
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let document = cx.new(|_| Document::new());
        if let Some(path) = &path {
            load(path, &document, cx);
        }
        let title = path
            .as_deref()
            .and_then(Path::file_name)
            .map_or("MigPad".into(), |name| name.to_string_lossy().into_owned());
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(1000.), px(700.)), cx))),
            titlebar: Some(TitlebarOptions { title: Some(title.into()), ..Default::default() }),
            ..Default::default()
        };
        let window = cx
            .open_window(options, |window, cx| {
                let view = cx.new(|cx| EditorView::new(document.clone(), window, cx));
                window.focus(&view.focus_handle(cx), cx);
                view
            })
            .expect("failed to open the main window");
        debug_input::play(window.into(), cx);
        cx.activate(true);
    });
}

/// Opens the file into `document`: a large one shows its beginning at once and the whole text
/// once the background load is done.
fn load(path: &Path, document: &Entity<Document>, cx: &mut App) {
    match open(path, OpenAs::Detect { tld: None }) {
        Ok(Opened::Complete(loaded)) => document.update(cx, |doc, cx| {
            *doc = loaded;
            cx.notify();
        }),
        Ok(Opened::Partial { preview, loader }) => {
            document.update(cx, |doc, cx| {
                *doc = preview;
                cx.notify();
            });
            let loading = cx.background_executor().spawn(async move { loader.load() });
            let document = document.clone();
            let path = path.to_owned();
            cx.spawn(async move |cx| match loading.await {
                Ok(loaded) => document.update(cx, |doc, cx| {
                    *doc = loaded;
                    cx.notify();
                }),
                Err(error) => eprintln!("{}: {error}", path.display()),
            })
            .detach();
        }
        Err(error) => eprintln!("{}: {error}", path.display()),
    }
}
