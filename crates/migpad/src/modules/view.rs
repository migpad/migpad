//! How the document shows: word wrap, invisible characters, indent guides. The toggles act on the
//! document of the window even while an input field has the focus.

use gpui::actions;

use crate::commands::{Command, MenuId, Module, Registry, update_menus};
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
            .checked(|workspace, cx| {
                workspace.editor().read(cx).wraps_lines() && !workspace.document().read(cx).is_large()
            })
            .enabled(|workspace, cx| !workspace.document().read(cx).is_large());
        registry.add(word_wrap, menu(0));
        let invisibles = Command::new("view.invisibles", Key::ViewInvisibles, ToggleInvisibles)
            .checked(|workspace, cx| workspace.editor().read(cx).shows_whitespace());
        registry.add(invisibles, menu(1));
        let indent_guides = Command::new("view.indent_guides", Key::ViewIndentGuides, ToggleIndentGuides)
            .checked(|workspace, cx| workspace.editor().read(cx).shows_indent_guides());
        registry.add(indent_guides, menu(1));
        // The menu bar of Windows and Linux in each window, besides that of the system.
        if cfg!(target_os = "macos") {
            let menu_bar = Command::new("view.menu_bar", Key::ViewMenuBar, ToggleMenuBarInWindow)
                .checked(|_, cx| view_options::menu_bar(cx));
            registry.add(menu_bar, menu(3));
            registry.on_app_action(|_: &ToggleMenuBarInWindow, cx| cx.defer(view_options::toggle_menu_bar_in_window));
        }

        registry.on_window_action(|workspace, _: &ToggleWordWrap, _, cx| {
            if workspace.document().read(cx).is_large() {
                return;
            }
            workspace.editor().update(cx, |editor, cx| editor.set_word_wrap(!editor.wraps_lines(), cx));
            update_menus(Some(workspace), cx);
        });
        registry.on_window_action(|workspace, _: &ToggleInvisibles, _, cx| {
            workspace.editor().update(cx, |editor, cx| editor.set_show_whitespace(!editor.shows_whitespace(), cx));
            update_menus(Some(workspace), cx);
        });
        registry.on_window_action(|workspace, _: &ToggleIndentGuides, _, cx| {
            workspace
                .editor()
                .update(cx, |editor, cx| editor.set_show_indent_guides(!editor.shows_indent_guides(), cx));
            update_menus(Some(workspace), cx);
        });
    }
}
