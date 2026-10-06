//! The root of a window: the view of its document, and the commands of the window, which act on
//! the window or on that document whatever has the focus.

use gpui::{App, Context, Entity, FocusHandle, Focusable, Render, Subscription, Window, div, prelude::*};
use migpad_editor::EditorView;

use crate::commands::{Registry, update_menus};

pub struct Workspace {
    editor: Entity<EditorView>,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    pub fn new(editor: Entity<EditorView>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // The check marks of the menus are those of the document of the active window.
        let activation = cx.observe_window_activation(window, |workspace, window, cx| {
            if window.is_window_active() {
                update_menus(Some(workspace), cx);
            }
        });
        // Colors follow the appearance of the system, unless a theme is chosen.
        let appearance = migpad_ui::theme::follow_system(window, cx);
        Workspace { editor, _subscriptions: vec![activation, appearance] }
    }

    /// The view of the document that the commands act on.
    pub fn editor(&self) -> &Entity<EditorView> {
        &self.editor
    }
}

impl Focusable for Workspace {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.editor.focus_handle(cx)
    }
}

impl Render for Workspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let handlers = cx.global::<Registry>().window_handlers();
        let root = div().key_context("Workspace").size_full();
        handlers.iter().fold(root, |root, handler| handler(root, cx)).child(self.editor.clone())
    }
}
