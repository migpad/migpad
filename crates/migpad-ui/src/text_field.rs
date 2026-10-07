//! A text field: the one line of an input field — the single-line view of the editor — in a frame
//! that shows when the field has the keyboard.

use gpui::{App, Entity, Focusable, IntoElement, RenderOnce, Window, div, prelude::*, px, rgb};
use migpad_editor::EditorView;

use crate::theme::theme;

#[derive(IntoElement)]
pub struct TextField {
    view: Entity<EditorView>,
}

impl TextField {
    /// The field of `view`, a single-line view; it is as wide as the element around it lets it be.
    pub fn new(view: Entity<EditorView>) -> Self {
        TextField { view }
    }
}

impl RenderOnce for TextField {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = theme(cx);
        let focused = self.view.focus_handle(cx).contains_focused(window, cx);
        div()
            .flex()
            .items_center()
            .w_full()
            .h(px(24.))
            .px(px(2.))
            .rounded(px(4.))
            .border_1()
            .border_color(rgb(if focused { theme.accent } else { theme.border }))
            .bg(rgb(theme.editor.background))
            .overflow_hidden()
            .child(self.view)
    }
}
