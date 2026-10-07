//! The status bar at the bottom of a window: fields of text on its left and on its right; a field
//! may open a menu, as the encoding does.

use std::rc::Rc;

use gpui::{App, IntoElement, Pixels, Point, RenderOnce, Role, SharedString, Window, div, prelude::*, px, rgb};

use crate::theme::theme;
use crate::tooltip::Tooltip;

type MenuHandler = Rc<dyn Fn(Point<Pixels>, &mut Window, &mut App)>;

/// A field of the bar: its text, and for one that opens a menu, its tooltip and what opens the
/// menu where it was clicked.
struct Field {
    text: SharedString,
    menu: Option<(SharedString, MenuHandler)>,
}

#[derive(IntoElement)]
pub struct StatusBar {
    left: Vec<Field>,
    right: Vec<Field>,
}

impl StatusBar {
    pub fn new() -> Self {
        StatusBar { left: Vec::new(), right: Vec::new() }
    }

    /// A field after those on the left.
    pub fn left(mut self, field: impl Into<SharedString>) -> Self {
        self.left.push(Field { text: field.into(), menu: None });
        self
    }

    /// A field after those on the right.
    pub fn right(mut self, field: impl Into<SharedString>) -> Self {
        self.right.push(Field { text: field.into(), menu: None });
        self
    }

    /// A field after those on the right that opens a menu: a click calls `open` with where it was.
    pub fn right_menu(
        mut self,
        field: impl Into<SharedString>,
        tooltip: impl Into<SharedString>,
        open: impl Fn(Point<Pixels>, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.right.push(Field { text: field.into(), menu: Some((tooltip.into(), Rc::new(open))) });
        self
    }

    /// The fields, left ones first.
    pub fn fields(&self) -> impl Iterator<Item = &str> {
        self.left.iter().chain(&self.right).map(|field| field.text.as_ref())
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
        let fields = |side: &'static str, fields: Vec<Field>| {
            div().flex().items_center().gap(px(10.)).children(fields.into_iter().enumerate().map(move |(i, field)| {
                let Some((tooltip, open)) = field.menu else {
                    return div().px(px(4.)).child(field.text).into_any_element();
                };
                div()
                    .id((side, i))
                    .role(Role::Button)
                    .aria_label(tooltip.clone())
                    .px(px(4.))
                    .rounded(px(3.))
                    .hover(|style| style.bg(rgb(theme.hover)).text_color(rgb(theme.text)))
                    .on_click(move |event, window, cx| open(event.position(), window, cx))
                    .tooltip(Tooltip::builder(tooltip, None))
                    .child(field.text)
                    .into_any_element()
            }))
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
            .px(px(6.))
            .bg(rgb(theme.bar))
            .border_t_1()
            .border_color(rgb(theme.border))
            .text_color(rgb(theme.text_muted))
            .whitespace_nowrap()
            .overflow_hidden()
            .child(fields("left", self.left))
            .child(fields("right", self.right))
    }
}
