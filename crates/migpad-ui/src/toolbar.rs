//! The toolbar under the menus: buttons with the names of commands, in groups apart.

use gpui::{AnyElement, App, IntoElement, RenderOnce, Role, Window, div, prelude::*, px, rgb};

use crate::button::Button;
use crate::theme::theme;

#[derive(IntoElement)]
pub struct Toolbar {
    groups: Vec<Vec<Button>>,
}

impl Toolbar {
    /// The buttons, by groups.
    pub fn new(groups: Vec<Vec<Button>>) -> Self {
        Toolbar { groups }
    }
}

impl RenderOnce for Toolbar {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = theme(cx);
        let mut children: Vec<AnyElement> = Vec::new();
        for (i, group) in self.groups.into_iter().filter(|group| !group.is_empty()).enumerate() {
            if i > 0 {
                children.push(div().flex_none().w(px(1.)).h(px(16.)).mx(px(4.)).bg(rgb(theme.border)).into_any_element());
            }
            children.extend(group.into_iter().map(IntoElement::into_any_element));
        }
        div()
            .id("toolbar")
            .role(Role::Toolbar)
            .flex()
            .flex_none()
            .items_center()
            .gap(px(2.))
            .h(px(32.))
            .px(px(4.))
            .bg(rgb(theme.bar))
            .border_b_1()
            .border_color(rgb(theme.border))
            .overflow_hidden()
            .children(children)
    }
}
