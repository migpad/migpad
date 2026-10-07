//! MigPad, a fast cross-platform text editor.

// Release builds on Windows are GUI applications: no console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod cli;
mod commands;
mod debug_input;
mod find;
mod go_to;
mod instance;
mod journals;
mod keys;
mod modules;
#[cfg(target_os = "macos")]
mod native_menu;
mod notices;
mod recent;
mod session;
mod status;
mod strings;
mod tabs;
mod view_options;
mod windows;
mod workspace;

use std::path::PathBuf;
use std::time::Duration;

use gpui::App;
use migpad_ui::ThemeMode;

use crate::instance::{Inbox, Request};
use crate::strings::Language;

fn main() {
    let cwd = std::env::current_dir().unwrap_or_default();
    let files = cli::files(std::env::args_os().skip(1), &cwd, |path| path.exists());
    let request = Request { files };
    let found = journals::find();
    let root = found.as_ref().map(|dir| dir.root().to_path_buf());
    // Another copy runs with this folder of data: it takes the files, and this one is done.
    if let Some(root) = &root
        && instance::send(root, &request).is_ok()
    {
        return;
    }
    // From a terminal, the program goes on in a process of its own: the terminal is free at once.
    if instance::leave_terminal() {
        return;
    }
    let data = journals::take(found);
    let inbox = Inbox::new();
    match &root {
        Some(root) if data.holds_folder() => {
            if let Err(error) = instance::listen(root, inbox.clone()) {
                eprintln!("MigPad could not listen for the files of its next starts: {error}");
            }
        }
        // Another copy holds the folder and may be starting: its channel opens in a moment. If it
        // does not answer, this one runs on its own, keeping nothing in the folder.
        Some(root) if data.another_copy() => {
            for _ in 0..20 {
                std::thread::sleep(Duration::from_millis(100));
                if instance::send(root, &request).is_ok() {
                    return;
                }
            }
        }
        _ => {}
    }

    let application = gpui_platform::application();
    // A click on MigPad in the Dock while it has no windows opens one, as on macOS it stays open
    // without them.
    application.on_reopen(|cx| {
        if cx.windows().is_empty() {
            windows::open_window(&[], cx);
        }
    });
    // Files from Finder and the Dock, as MigPad starts and while it runs.
    let urls = inbox.clone();
    application.on_open_urls(move |list| urls.push(Request { files: instance::files_of_urls(&list) }));
    application.run(move |cx: &mut App| {
        strings::set_language(Language::of_system());
        journals::init(data, cx);
        recent::init(cx);
        view_options::init(cx);
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
        // The files of the command line, and those Finder gave as MigPad started.
        let mut files = request.files;
        files.extend(inbox.take_all().into_iter().flat_map(|request| request.files));
        let paths: Vec<PathBuf> = files.iter().map(|file| file.path.clone()).collect();
        // The windows of the last time come back, with these files; or a window opens for them, or
        // an untitled document.
        let window = session::restore(&paths, cx)
            .or_else(|| windows::open_window(&paths, cx))
            .or_else(|| windows::open_window(&[], cx));
        match window {
            Some(window) => {
                // A second copy keeps nothing in the folder of data: it says so.
                if journals::another_copy(cx) {
                    let _ = window.update(cx, |workspace, _, cx| workspace.notify(notices::another_copy(), cx));
                }
                instance::go_to_places(&files, cx);
                debug_input::play(window, cx)
            }
            // The reason is told above; without a window there is nothing to do.
            None => cx.quit(),
        }
        instance::serve(inbox, cx);
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
