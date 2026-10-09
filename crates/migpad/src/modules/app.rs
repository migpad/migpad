//! The application: About MigPad, its settings, hiding it and quitting.

use gpui::actions;

use crate::about;
use crate::commands::{Command, MenuId, Module, Registry, by_os};
use crate::session;
use crate::settings_window;
use crate::strings::Key;

actions!(app, [About, OpenSettings, Quit, Hide, HideOthers, ShowAll]);

pub struct AppModule;

impl Module for AppModule {
    fn id(&self) -> &'static str {
        "app"
    }

    fn register(&self, registry: &mut Registry) {
        // The menu of the application is macOS's: About MigPad, the settings and quitting are there.
        // Windows and Linux have About MigPad in the Help menu, the settings at the end of the Edit
        // menu, and quit from the File menu.
        let menu = |group| cfg!(target_os = "macos").then_some((MenuId::App, group));
        let (label, place) = if cfg!(target_os = "macos") {
            (Key::AppAboutMacos, menu(0))
        } else {
            (Key::AppAbout, Some((MenuId::Help, 0)))
        };
        registry.add(Command::new("app.about", label, About), place);
        let (label, place) = if cfg!(target_os = "macos") {
            (Key::SettingsCommandMacos, menu(1))
        } else {
            (Key::SettingsCommand, Some((MenuId::Edit, 9)))
        };
        let settings =
            Command::new("app.settings", label, OpenSettings).keys(by_os(&["cmd-,"], &["ctrl-,"], &["ctrl-,"]));
        registry.add(settings, place);
        if cfg!(target_os = "macos") {
            registry.add_services(MenuId::App, 2);
            registry.add(Command::new("app.hide", Key::AppHide, Hide).keys(&["cmd-h"]), menu(3));
            registry.add(Command::new("app.hide_others", Key::AppHideOthers, HideOthers).keys(&["cmd-alt-h"]), menu(3));
            registry.add(Command::new("app.show_all", Key::AppShowAll, ShowAll), menu(3));
        }
        let (label, place) =
            if cfg!(target_os = "macos") { (Key::AppQuit, menu(4)) } else { (Key::AppExit, Some((MenuId::File, 9))) };
        registry.add(Command::new("app.quit", label, Quit).keys(by_os(&["cmd-q"], &[], &["ctrl-q"])), place);
        registry.on_app_action(|_: &About, cx| cx.defer(about::show));
        // Once the window the keys came from is done with them.
        registry.on_app_action(|_: &OpenSettings, cx| cx.defer(settings_window::open));
        // The documents of every window are asked whether their changes are kept.
        registry.on_app_action(|_: &Quit, cx| cx.defer(session::quit));
        registry.on_app_action(|_: &Hide, cx| cx.hide());
        registry.on_app_action(|_: &HideOthers, cx| cx.hide_other_apps());
        registry.on_app_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    }
}
