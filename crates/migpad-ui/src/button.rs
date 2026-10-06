//! A button with a text: flat until the pointer is over it, gray when it does nothing now.

use std::rc::Rc;

use gpui::{App, ClickEvent, ElementId, IntoElement, RenderOnce, Role, SharedString, Window, div, prelude::*, px, rgb};

use crate::theme::theme;
use crate::tooltip::Tooltip;

type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    label: SharedString,
    disabled: bool,
    tooltip: Option<(SharedString, Option<SharedString>)>,
    on_click: Option<ClickHandler>,
}

impl Button {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Button { id: id.into(), label: label.into(), disabled: false, tooltip: None, on_click: None }
    }

    pub fn disabled(self, disabled: bool) -> Self {
        Button { disabled, ..self }
    }

    /// Shown when the pointer rests on the button: a text, and keys after it.
    pub fn tooltip(self, text: impl Into<SharedString>, keys: Option<SharedString>) -> Self {
        Button { tooltip: Some((text.into(), keys)), ..self }
    }

    pub fn on_click(self, handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        Button { on_click: Some(Rc::new(handler)), ..self }
    }
}

impl RenderOnce for Button {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = theme(cx);
        div()
            .id(self.id)
            .role(Role::Button)
            .aria_label(self.label.clone())
            .flex()
            .flex_none()
            .items_center()
            .h(px(24.))
            .px(px(8.))
            .rounded(px(4.))
            .whitespace_nowrap()
            .when(self.disabled, |button| button.text_color(rgb(theme.text_disabled)))
            .when(!self.disabled, |button| {
                button
                    .text_color(rgb(theme.text))
                    .hover(|style| style.bg(rgb(theme.hover)))
                    .active(|style| style.bg(rgb(theme.pressed)))
            })
            .when_some(self.on_click.filter(|_| !self.disabled), |button, handler| {
                button.on_click(move |event, window, cx| handler(event, window, cx))
            })
            .when_some(self.tooltip, |button, (text, keys)| button.tooltip(Tooltip::builder(text, keys)))
            .child(self.label)
    }
}
