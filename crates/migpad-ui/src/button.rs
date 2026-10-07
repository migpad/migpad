//! A button with a text: flat until the pointer is over it, gray when it does nothing now. A toggle
//! shows whether it is on.

use std::rc::Rc;

use gpui::{
    AnyElement, App, ClickEvent, ElementId, IntoElement, RenderOnce, Role, SharedString, Toggled, Window, div,
    prelude::*, px, rgb,
};

use crate::theme::theme;
use crate::tooltip::Tooltip;

type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    label: SharedString,
    /// What a screen reader says, if not the label: a symbol tells it nothing.
    name: Option<SharedString>,
    /// What the button shows instead of its label: styled text.
    content: Option<AnyElement>,
    disabled: bool,
    /// Whether a toggle is on; `None` for a button that is not a toggle.
    toggled: Option<bool>,
    tooltip: Option<(SharedString, Option<SharedString>)>,
    on_click: Option<ClickHandler>,
}

impl Button {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Button {
            id: id.into(),
            label: label.into(),
            name: None,
            content: None,
            disabled: false,
            toggled: None,
            tooltip: None,
            on_click: None,
        }
    }

    /// Shows `content` instead of the label, which a screen reader still says.
    pub fn content(self, content: impl IntoElement) -> Self {
        Button { content: Some(content.into_any_element()), ..self }
    }

    /// What a screen reader says of the button, when its label is a symbol.
    pub fn name(self, name: impl Into<SharedString>) -> Self {
        Button { name: Some(name.into()), ..self }
    }

    /// The button is a toggle, on or off.
    pub fn toggled(self, on: bool) -> Self {
        Button { toggled: Some(on), ..self }
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
            .aria_label(self.name.unwrap_or_else(|| self.label.clone()))
            .when_some(self.toggled, |button, on| button.aria_toggled(if on { Toggled::True } else { Toggled::False }))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .h(px(24.))
            .min_w(px(24.))
            .px(px(if self.toggled.is_some() { 5. } else { 8. }))
            .rounded(px(4.))
            .whitespace_nowrap()
            .when(self.toggled == Some(true), |button| {
                button.bg(rgb(theme.menu_selected)).border_1().border_color(rgb(theme.accent))
            })
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
            .child(match self.content {
                Some(content) => content,
                None => self.label.into_any_element(),
            })
    }
}
