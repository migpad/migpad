//! MigPad, a fast cross-platform text editor.

// Release builds on Windows are GUI applications: no console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod about;
mod appearance;
mod cli;
mod command_link;
mod commands;
mod debug_input;
#[cfg(target_os = "macos")]
mod default_app;
mod find;
mod go_to;
mod instance;
mod journals;
mod keys;
mod modules;
#[cfg(target_os = "macos")]
mod native_menu;
#[cfg(target_os = "macos")]
mod native_settings;
mod notices;
mod own_settings;
mod recent;
mod session;
mod settings;
mod settings_window;
mod status;
mod strings;
mod table;
mod tabs;
mod translations;
mod view_options;
mod windows;
mod workspace;

use std::io::Write as _;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpui::App;

use crate::instance::{Inbox, Request};

fn main() {
    // The version, for scripts and the checks of the packages: nothing starts.
    if std::env::args_os().nth(1).is_some_and(|arg| arg == "--version") {
        let _ = writeln!(std::io::stdout(), "MigPad {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let cwd = std::env::current_dir().unwrap_or_default();
    // In place of Notepad, the command line is Notepad's.
    let notepad = command_line().and_then(|line| cli::notepad(&line, &cwd));
    let files = notepad.unwrap_or_else(|| cli::files(std::env::args_os().skip(1), &cwd, |path| path.exists()));
    let request = Request { files };
    let found = journals::find();
    let root = found.as_ref().map(|dir| dir.root().to_path_buf());
    // Another copy runs with this folder of data: it takes the files, and this one is done.
    let sent = root.as_ref().map(|root| instance::send(root, &request));
    if let Some(Ok(())) = sent {
        return;
    }
    // A copy that took the request and did not answer may still open the files: it is not asked
    // again.
    let silent = matches!(&sent, Some(Err(error)) if error.kind() == std::io::ErrorKind::TimedOut);
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
        Some(root) if data.another_copy() && !silent => {
            let deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(100));
                match instance::send(root, &request) {
                    Ok(()) => return,
                    Err(error) if error.kind() == std::io::ErrorKind::TimedOut => break,
                    Err(_) => {}
                }
            }
        }
        _ => {}
    }

    let application = gpui_platform::application();
    // A click on MigPad in the Dock while it has no windows opens one, as on macOS it stays open
    // without them.
    application.on_reopen(|cx| {
        if windows::document_windows(cx) == 0 {
            windows::open_window(&[], cx);
        }
    });
    // Files from Finder and the Dock, as MigPad starts and while it runs.
    let urls = inbox.clone();
    application.on_open_urls(move |list| urls.push(Request { files: instance::files_of_urls(&list) }));
    application.run(move |cx: &mut App| {
        journals::init(data, cx);
        // The language, the theme, the font, the strings of the user: before anything shows. What
        // is wrong with the strings is told in the language of the settings.
        settings::init(cx);
        if let Some(root) = journals::root(cx) {
            translations::load(&root, cx);
        }
        recent::init(cx);
        migpad_editor::init(cx);
        own_settings::init(cx);
        commands::init(&modules::all(), cx);
        commands::update_menus(None, cx);
        cx.on_window_closed(|cx, closed| {
            session::window_closed(closed, cx);
            // On macOS MigPad stays open without windows, as applications there do; elsewhere it
            // quits with its last window of documents, the settings window with it.
            if windows::document_windows(cx) == 0 && !cfg!(target_os = "macos") {
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
                // A second copy keeps nothing in the folder of data: it says so. What is wrong with the
                // settings is told too.
                if journals::another_copy(cx) {
                    let _ = window.update(cx, |workspace, _, cx| workspace.notify(notices::another_copy(), cx));
                }
                let told = translations::take_notices(cx).into_iter().chain(settings::notice(cx));
                for notice in told.collect::<Vec<_>>() {
                    let _ = window.update(cx, |workspace, _, cx| workspace.notify(notice, cx));
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

/// The command line as Windows gave it, before it is split into arguments: Notepad takes the rest
/// of it as one file, spaces and all.
#[cfg(windows)]
fn command_line() -> Option<String> {
    use windows_sys::Win32::System::Environment::GetCommandLineW;
    // SAFETY: the command line of the process, a string that ends with a zero and lives as long.
    let line = unsafe { GetCommandLineW() };
    if line.is_null() {
        return None;
    }
    // SAFETY: the characters up to the zero.
    let length = (0..).take_while(|&i| unsafe { *line.add(i) } != 0).count();
    Some(String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(line, length) }))
}

#[cfg(not(windows))]
fn command_line() -> Option<String> {
    None
}
