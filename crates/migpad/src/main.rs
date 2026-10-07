//! MigPad, a fast cross-platform text editor.

// Release builds on Windows are GUI applications: no console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod debug_input;
mod find;
mod journals;
mod keys;
mod modules;
mod notices;
mod recent;
mod session;
mod status;
mod strings;
mod tabs;
mod windows;
mod workspace;

use std::ffi::OsString;
use std::path::PathBuf;

use gpui::App;
use migpad_ui::ThemeMode;

use crate::strings::Language;

fn main() {
    let paths = file_arguments(std::env::args_os().skip(1));
    let application = gpui_platform::application();
    // A click on MigPad in the Dock while it has no windows opens one, as on macOS it stays open
    // without them.
    application.on_reopen(|cx| {
        if cx.windows().is_empty() {
            windows::open_window(&[], cx);
        }
    });
    application.run(move |cx: &mut App| {
        strings::set_language(Language::of_system());
        journals::init(cx);
        recent::init(cx);
        migpad_ui::theme::set_mode(theme_mode(), cx);
        migpad_editor::init(cx);
        commands::init(&modules::all(), cx);
        commands::update_menus(None, cx);
        cx.on_window_closed(|cx, closed| {
            session::window_closed(closed, cx);
            // On macOS MigPad stays open without windows, as applications there do.
            if cx.windows().is_empty() && !cfg!(target_os = "macos") {
                cx.quit();
            }
            // The menus show the check marks of the active window, which may not have changed.
            let active = cx.active_window().and_then(|window| window.downcast::<workspace::Workspace>());
            let updated = active.is_some_and(|active| {
                active.update(cx, |workspace, _, cx| commands::update_menus(Some(workspace), cx)).is_ok()
            });
            if !updated {
                commands::update_menus(None, cx);
            }
        })
        .detach();
        // However the program ends — from the Dock, on logging out — edits of the last moment reach
        // the disk, and the session is written for the next start to bring everything back.
        cx.on_app_quit(|cx| {
            session::prepare_quit(cx);
            async {}
        })
        .detach();
        // The windows of the last time come back, with the files of the command line; or a window
        // opens for these files, or an untitled document.
        let window = session::restore(&paths, cx)
            .or_else(|| windows::open_window(&paths, cx))
            .or_else(|| windows::open_window(&[], cx));
        match window {
            Some(window) => {
                // A second copy keeps nothing in the folder of data: it says so.
                if journals::another_copy(cx) {
                    let _ = window.update(cx, |workspace, _, cx| workspace.notify(notices::another_copy(), cx));
                }
                debug_input::play(window, cx)
            }
            // The reason is told above; without a window there is nothing to do.
            None => cx.quit(),
        }
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

/// The files to open: the arguments that are not options. An option such as
/// `-AppleLanguages '(en)'`, which sets a user default of macOS, is passed over with its value;
/// after `--` every argument is a file, even if it starts with `-`.
fn file_arguments(args: impl IntoIterator<Item = OsString>) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg == "--" {
            files.extend(args.map(PathBuf::from));
            break;
        }
        if arg.as_encoded_bytes().starts_with(b"-") {
            args.next();
        } else {
            files.push(PathBuf::from(arg));
        }
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(args: &[&str]) -> Vec<PathBuf> {
        file_arguments(args.iter().map(OsString::from))
    }

    #[test]
    fn files_are_the_arguments_that_are_not_options() {
        assert_eq!(files(&["notes.txt"]), [PathBuf::from("notes.txt")]);
        assert_eq!(
            files(&["-AppleLanguages", "(en)", "заметки.txt", "more.txt"]),
            [PathBuf::from("заметки.txt"), PathBuf::from("more.txt")]
        );
        assert_eq!(files(&["a.txt", "--", "-dash.txt", "b.txt"]), ["a.txt", "-dash.txt", "b.txt"].map(PathBuf::from));
        assert!(files(&["-AppleLanguages", "(en)"]).is_empty());
        assert!(files(&[]).is_empty());
    }
}
