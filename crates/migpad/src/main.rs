//! MigPad, a fast cross-platform text editor.

// Release builds on Windows are GUI applications: no console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use gpui::{App, Bounds, Context, TitlebarOptions, Window, WindowBounds, WindowOptions, div, prelude::*, px, size};

/// Placeholder content of the main window.
struct MainWindow;

impl Render for MainWindow {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().bg(gpui::white())
    }
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(1000.), px(700.)), cx))),
            titlebar: Some(TitlebarOptions { title: Some("MigPad".into()), ..Default::default() }),
            ..Default::default()
        };
        cx.open_window(options, |_, cx| cx.new(|_| MainWindow)).expect("failed to open the main window");
        cx.activate(true);
    });
}
