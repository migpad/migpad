//! The settings: `settings.toml` in the folder of the data ([ADR 0011]), read as MigPad starts. A
//! change takes effect at once, whatever made it — the settings window, the View menu, the file
//! edited by hand in MigPad or another program, seen when a window of MigPad comes back: in every
//! window and menu. What is wrong with the file is told over the text of the active window.

use std::path::{Path, PathBuf};

use gpui::{App, Global, SharedString};
use migpad_core::document::Fingerprint;
use migpad_core::settings::{self as file, Setting, Settings, SettingsFile};
use migpad_editor::{EditorSettings, font_installed, system_font};
use migpad_ui::ThemeMode;

use crate::appearance;
use crate::commands::refresh_menus;
use crate::journals;
use crate::notices::{self, Notice, Topic};
use crate::settings_window;
use crate::strings::{self, Language};
use crate::view_options;
use crate::windows;
use crate::workspace::Workspace;

/// The settings and their file.
struct State {
    file: SettingsFile,
    /// The file, if there is a folder of data.
    path: Option<PathBuf>,
    /// Whether changes go to the file: not in a second copy of MigPad, which keeps nothing in the
    /// folder of the data.
    writable: bool,
    /// The file as MigPad last read or wrote it: another fingerprint is a change by someone else.
    fingerprint: Option<Fingerprint>,
}

impl Global for State {}

/// Reads the settings and puts them in effect, before the first window opens.
pub fn init(cx: &mut App) {
    let path = journals::root(cx).map(|root| root.join("settings.toml"));
    let file =
        path.as_deref().map_or_else(SettingsFile::default, |path| SettingsFile::read(path, &Settings::default()));
    let fingerprint = path.as_deref().and_then(|path| Fingerprint::of_path(path).ok());
    let settings = file.settings().clone();
    let writable = !journals::another_copy(cx);
    cx.set_global(State { file, path, writable, fingerprint });
    apply(None, &settings, cx);
}

/// The settings in effect.
pub fn get(cx: &App) -> &Settings {
    match cx.try_global::<State>() {
        Some(state) => state.file.settings(),
        None => default(),
    }
}

fn default() -> &'static Settings {
    static DEFAULT: std::sync::OnceLock<Settings> = std::sync::OnceLock::new();
    DEFAULT.get_or_init(Settings::default)
}

/// Changes a setting: it takes effect, and goes to the file. A value it has already changes
/// nothing.
pub fn set(setting: Setting, cx: &mut App) {
    // Another program may have changed the file since MigPad read it: the change goes on top.
    check_file(cx);
    let Some(state) = cx.try_global::<State>() else { return };
    let old = state.file.settings().clone();
    let state = cx.global_mut::<State>();
    if !state.file.set(setting) {
        return;
    }
    let new = state.file.settings().clone();
    let failure = match &state.path {
        // A file that is not TOML is not written: it stays as MigPad read it.
        Some(path) if state.writable && state.file.is_writable() => match state.file.write(path) {
            Ok(()) => {
                state.fingerprint = Fingerprint::of_path(path).ok();
                None
            }
            Err(error) => Some(notices::settings_write_failed(path, &error)),
        },
        _ => None,
    };
    apply(Some(&old), &new, cx);
    if let Some(notice) = failure {
        tell(notice, cx);
    }
}

/// Reads the file again if it changed since MigPad last read or wrote it: another program, or
/// MigPad itself, saved it. What is wrong with it is told; a notification of an old problem goes.
pub fn check_file(cx: &mut App) {
    let Some(state) = cx.try_global::<State>() else { return };
    let Some(path) = state.path.clone() else { return };
    let fingerprint = Fingerprint::of_path(&path).ok();
    if fingerprint == state.fingerprint {
        return;
    }
    let old = state.file.settings().clone();
    let file = SettingsFile::read(&path, &old);
    let new = file.settings().clone();
    let notice = notices::settings_problems(file.problems());
    let state = cx.global_mut::<State>();
    state.file = file;
    state.fingerprint = fingerprint;
    apply(Some(&old), &new, cx);
    match notice.or_else(|| font_notice(&new, cx)) {
        Some(notice) => tell(notice, cx),
        None => clear_notices(cx),
    }
}

/// What to tell about the settings as MigPad starts: what is wrong with the file, or a font that
/// is not in the system.
pub fn notice(cx: &App) -> Option<Notice> {
    let state = cx.try_global::<State>()?;
    notices::settings_problems(state.file.problems()).or_else(|| font_notice(state.file.settings(), cx))
}

/// The font of the settings, if the system does not have it.
fn font_notice(settings: &Settings, cx: &App) -> Option<Notice> {
    let font = settings.font.as_deref()?;
    (!font_installed(font, cx)).then(|| notices::font_missing(font, system_font()))
}

/// Opens the file of the settings in a tab of the active window, a new one with every setting and
/// what it is if there is none yet.
pub fn open_file(cx: &mut App) {
    let Some(state) = cx.try_global::<State>() else { return };
    let Some(path) = state.path.clone() else { return };
    if !path.exists() && state.writable {
        let written = SettingsFile::default().write(&path);
        let state = cx.global_mut::<State>();
        // The new file has the defaults, which the settings take where the user changed nothing.
        match written {
            Ok(()) => state.fingerprint = Fingerprint::of_path(&path).ok(),
            Err(error) => return tell(notices::settings_write_failed(&path, &error), cx),
        }
    }
    let Some(window) = windows::last_active(cx).or_else(|| windows::open_window(&[], cx)) else { return };
    let _ = window.update(cx, |workspace, window, cx| {
        workspace.open_paths(&[path.as_path()], window, cx);
        window.activate_window();
    });
}

/// Whether `path` is the file of the settings.
pub fn is_settings_file(path: &Path, cx: &App) -> bool {
    let settings = cx.try_global::<State>().and_then(|state| state.path.as_deref());
    // Through a link, or the file a link points to.
    settings.is_some_and(|settings| windows::same_file(settings, path))
}

/// Puts the settings `new` in effect: all of them, or those that differ from `old`.
fn apply(old: Option<&Settings>, new: &Settings, cx: &mut App) {
    let changed = |differs: fn(&Settings, &Settings) -> bool| old.is_none_or(|old| differs(old, new));
    if changed(|old, new| old.language != new.language) {
        strings::set_language(language(new.language));
        if old.is_some() {
            windows::language_changed(cx);
        }
    }
    if changed(|old, new| old.theme != new.theme) {
        let mode = match new.theme {
            file::Theme::System => ThemeMode::System,
            file::Theme::Light => ThemeMode::Light,
            file::Theme::Dark => ThemeMode::Dark,
        };
        // The parts the system draws — title bars, menus — first: a theme that follows the system
        // takes its appearance from them.
        appearance::set(mode);
        migpad_ui::theme::set_mode(mode, cx);
    }
    let editor = EditorSettings {
        font: new.font.clone().map(SharedString::from),
        font_size: new.font_size as f32,
        tab_width: new.tab_width as usize,
        word_wrap: new.word_wrap,
        show_whitespace: new.invisibles,
        show_indent_guides: new.indent_guides,
    };
    if cx.try_global::<EditorSettings>() != Some(&editor) {
        cx.set_global(editor);
        cx.refresh_windows();
    }
    if old.is_some_and(|old| old.menu_bar != new.menu_bar) {
        view_options::menu_bar_changed(cx);
    }
    if old.is_some() {
        refresh_menus(cx);
        settings_window::refresh(cx);
    }
}

/// The language of the interface for the setting.
fn language(setting: file::Language) -> Language {
    match setting {
        file::Language::System => Language::of_system(),
        file::Language::English => Language::English,
        file::Language::Russian => Language::Russian,
    }
}

/// Tells `notice` over the text of the active window.
fn tell(notice: Notice, cx: &mut App) {
    if let Some(window) = windows::last_active(cx) {
        let _ = window.update(cx, |workspace, _, cx| workspace.notify(notice, cx));
    }
}

/// Closes the notifications about the settings in every window: what they told is over.
fn clear_notices(cx: &mut App) {
    let windows: Vec<_> = windows::workspaces(cx).map(|(window, _)| window).collect();
    for window in windows {
        let _ = window.update(cx, |workspace: &mut Workspace, _, cx| workspace.clear_topic(Topic::Settings, cx));
    }
}
