//! MigPad, a fast cross-platform text editor.

// Release builds on Windows are GUI applications: no console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod debug_input;
mod modules;
mod strings;
mod workspace;

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use gpui::{App, AppContext, Bounds, Entity, Focusable, TitlebarOptions, WindowBounds, WindowOptions, px, size};
use migpad_core::document::{Document, OpenAs, Opened, open};
use migpad_editor::EditorView;
use migpad_ui::ThemeMode;

use crate::strings::Language;
use crate::workspace::Workspace;

fn main() {
    let path = file_argument(std::env::args_os().skip(1));
    gpui_platform::application().run(move |cx: &mut App| {
        strings::set_language(Language::of_system());
        migpad_ui::theme::set_mode(theme_mode(), cx);
        migpad_editor::init(cx);
        commands::init(&modules::all(), cx);
        commands::update_menus(None, cx);
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
                let editor = cx.new(|cx| EditorView::new(document.clone(), window, cx));
                let workspace = cx.new(|cx| Workspace::new(editor, window, cx));
                window.focus(&workspace.focus_handle(cx), cx);
                workspace
            })
            .expect("failed to open the main window");
        debug_input::play(window, cx);
        cx.activate(true);
    });
}

/// Light or dark as the system is; a debug build takes `MIGPAD_THEME=light` or `dark` for
/// screenshots, until the settings can choose.
fn theme_mode() -> ThemeMode {
    match std::env::var("MIGPAD_THEME") {
        Ok(theme) if cfg!(debug_assertions) && theme == "light" => ThemeMode::Light,
        Ok(theme) if cfg!(debug_assertions) && theme == "dark" => ThemeMode::Dark,
        _ => ThemeMode::System,
    }
}

/// The file to open: the first argument that is not an option. An option such as
/// `-AppleLanguages '(en)'`, which sets a user default of macOS, is passed over with its value;
/// after `--` the argument is a file even if it starts with `-`.
fn file_argument(args: impl IntoIterator<Item = OsString>) -> Option<PathBuf> {
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg == "--" {
            return args.next().map(PathBuf::from);
        }
        if !arg.as_encoded_bytes().starts_with(b"-") {
            return Some(PathBuf::from(arg));
        }
        args.next();
    }
    None
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

#[cfg(test)]
mod tests {
    use super::*;

    fn file(args: &[&str]) -> Option<PathBuf> {
        file_argument(args.iter().map(OsString::from))
    }

    #[test]
    fn the_file_is_the_first_argument_that_is_not_an_option() {
        assert_eq!(file(&["notes.txt"]), Some("notes.txt".into()));
        assert_eq!(file(&["-AppleLanguages", "(en)", "заметки.txt", "more.txt"]), Some("заметки.txt".into()));
        assert_eq!(file(&["--", "-dash.txt"]), Some("-dash.txt".into()));
        assert_eq!(file(&["-AppleLanguages", "(en)"]), None);
        assert_eq!(file(&[]), None);
    }
}
