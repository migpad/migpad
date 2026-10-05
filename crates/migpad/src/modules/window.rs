//! The window: minimizing and zooming it, from the Window menu of macOS.

use gpui::actions;

use crate::commands::{Command, MenuId, Module, Registry, by_os};
use crate::strings::Key;

actions!(window, [Minimize, Zoom]);

pub struct WindowModule;

impl Module for WindowModule {
    fn id(&self) -> &'static str {
        "window"
    }

    fn register(&self, registry: &mut Registry) {
        // On Windows and Linux the title bar of the window has these.
        let menu = cfg!(target_os = "macos").then_some((MenuId::Window, 0));
        registry.add(
            Command::new("window.minimize", Key::WindowMinimize, Minimize).keys(by_os(&["cmd-m"], &[], &[])),
            menu,
        );
        registry.add(Command::new("window.zoom", Key::WindowZoom, Zoom), menu);
        registry.on_window_action(|_, _: &Minimize, window, _| window.minimize_window());
        registry.on_window_action(|_, _: &Zoom, window, _| window.zoom_window());
    }
}
