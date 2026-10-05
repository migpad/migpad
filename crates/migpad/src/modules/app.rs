//! The application: hiding it and quitting.

use gpui::actions;

use crate::commands::{Command, MenuId, Module, Registry, by_os};
use crate::strings::Key;

actions!(app, [Quit, Hide, HideOthers, ShowAll]);

pub struct AppModule;

impl Module for AppModule {
    fn id(&self) -> &'static str {
        "app"
    }

    fn register(&self, registry: &mut Registry) {
        // The menu of the application is macOS's; Windows and Linux will quit from the File menu.
        let menu = |group| cfg!(target_os = "macos").then_some((MenuId::App, group));
        if cfg!(target_os = "macos") {
            registry.add_services(MenuId::App, 0);
            registry.add(Command::new("app.hide", Key::AppHide, Hide).keys(&["cmd-h"]), menu(1));
            registry.add(Command::new("app.hide_others", Key::AppHideOthers, HideOthers).keys(&["cmd-alt-h"]), menu(1));
            registry.add(Command::new("app.show_all", Key::AppShowAll, ShowAll), menu(1));
        }
        registry.add(Command::new("app.quit", Key::AppQuit, Quit).keys(by_os(&["cmd-q"], &[], &["ctrl-q"])), menu(2));
        registry.on_app_action(|_: &Quit, cx| cx.quit());
        registry.on_app_action(|_: &Hide, cx| cx.hide());
        registry.on_app_action(|_: &HideOthers, cx| cx.hide_other_apps());
        registry.on_app_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    }
}
