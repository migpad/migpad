//! How documents show: word wrap, invisible characters, indent guides. Each toggle is a setting,
//! the same in every tab and window, and acts even while an input field has the focus.

use gpui::actions;
use migpad_core::settings::Setting;

use crate::commands::{Command, MenuId, Module, Registry};
use crate::settings;
use crate::strings::Key;
use crate::view_options;

actions!(view, [ToggleWordWrap, ToggleInvisibles, ToggleIndentGuides, ToggleMenuBarInWindow]);

pub struct ViewModule;

impl Module for ViewModule {
    fn id(&self) -> &'static str {
        "view"
    }

    fn register(&self, registry: &mut Registry) {
        let menu = |group| Some((MenuId::View, group));
        // A large file is never wrapped: there the item is off and does nothing.
        let word_wrap = Command::new("view.word_wrap", Key::ViewWordWrap, ToggleWordWrap)
            .checked(|workspace, cx| settings::get(cx).word_wrap && !workspace.document().read(cx).is_large())
            .enabled(|workspace, cx| !workspace.document().read(cx).is_large());
        registry.add(word_wrap, menu(0));
        let invisibles = Command::new("view.invisibles", Key::ViewInvisibles, ToggleInvisibles)
            .checked(|_, cx| settings::get(cx).invisibles);
        registry.add(invisibles, menu(1));
        let indent_guides = Command::new("view.indent_guides", Key::ViewIndentGuides, ToggleIndentGuides)
            .checked(|_, cx| settings::get(cx).indent_guides);
        registry.add(indent_guides, menu(1));
        // The menu bar of Windows and Linux in each window, besides that of the system.
        if cfg!(target_os = "macos") {
            let menu_bar = Command::new("view.menu_bar", Key::ViewMenuBar, ToggleMenuBarInWindow)
                .checked(|_, cx| view_options::menu_bar(cx));
            registry.add(menu_bar, menu(3));
            registry.on_app_action(|_: &ToggleMenuBarInWindow, cx| cx.defer(view_options::toggle_menu_bar_in_window));
        }

        // The settings change once the window the command came from is done with it: every window
        // takes them.
        registry.on_window_action(|workspace, _: &ToggleWordWrap, _, cx| {
            if !workspace.document().read(cx).is_large() {
                let wrap = settings::get(cx).word_wrap;
                cx.defer(move |cx| settings::set(Setting::WordWrap(!wrap), cx));
            }
        });
        registry.on_window_action(|_, _: &ToggleInvisibles, _, cx| {
            let shown = settings::get(cx).invisibles;
            cx.defer(move |cx| settings::set(Setting::Invisibles(!shown), cx));
        });
        registry.on_window_action(|_, _: &ToggleIndentGuides, _, cx| {
            let shown = settings::get(cx).indent_guides;
            cx.defer(move |cx| settings::set(Setting::IndentGuides(!shown), cx));
        });
    }
}
