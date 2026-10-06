//! The status bar at the bottom of a window: fields of text on its left and on its right.

use gpui::{App, IntoElement, RenderOnce, Role, SharedString, Window, div, prelude::*, px, rgb};

use crate::theme::theme;

#[derive(Debug, IntoElement)]
pub struct StatusBar {
    left: Vec<SharedString>,
    right: Vec<SharedString>,
}

impl StatusBar {
    pub fn new() -> Self {
        StatusBar { left: Vec::new(), right: Vec::new() }
    }

    /// A field after those on the left.
    pub fn left(mut self, field: impl Into<SharedString>) -> Self {
        self.left.push(field.into());
        self
    }

    /// A field after those on the right.
    pub fn right(mut self, field: impl Into<SharedString>) -> Self {
        self.right.push(field.into());
        self
    }

    /// The fields, left ones first.
    pub fn fields(&self) -> impl Iterator<Item = &str> {
        self.left.iter().chain(&self.right).map(SharedString::as_ref)
    }
}

impl Default for StatusBar {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for StatusBar {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = theme(cx);
        let fields = |fields: Vec<SharedString>| {
            div().flex().items_center().gap(px(18.)).children(fields.into_iter().map(|field| div().child(field)))
        };
        div()
            .id("status-bar")
            .role(Role::Status)
            .flex()
            .flex_none()
            .items_center()
            .justify_between()
            .gap(px(18.))
            .h(px(24.))
            .px(px(10.))
            .bg(rgb(theme.bar))
            .border_t_1()
            .border_color(rgb(theme.border))
            .text_color(rgb(theme.text_muted))
            .whitespace_nowrap()
            .overflow_hidden()
            .child(fields(self.left))
            .child(fields(self.right))
    }
}
