//! Notification bars over the text: a message that does not stop the work — information, a
//! warning or an error — with buttons for what can be done about it and one that closes it.

use std::rc::Rc;

use gpui::{App, ElementId, IntoElement, RenderOnce, Role, SharedString, Window, div, prelude::*, px, rgb};

use crate::button::Button;
use crate::theme::theme;
use crate::tooltip::Tooltip;

/// What a notification is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

type CloseHandler = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct NotificationBar {
    id: ElementId,
    severity: Severity,
    message: SharedString,
    actions: Vec<Button>,
    close_label: SharedString,
    on_close: Option<CloseHandler>,
}

impl NotificationBar {
    pub fn new(id: impl Into<ElementId>, severity: Severity, message: impl Into<SharedString>) -> Self {
        NotificationBar {
            id: id.into(),
            severity,
            message: message.into(),
            actions: Vec::new(),
            close_label: SharedString::default(),
            on_close: None,
        }
    }

    /// A button for what can be done about the message.
    pub fn action(mut self, button: Button) -> Self {
        self.actions.push(button);
        self
    }

    /// The button that closes the bar, with its tooltip.
    pub fn on_close(self, label: impl Into<SharedString>, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        NotificationBar { close_label: label.into(), on_close: Some(Rc::new(handler)), ..self }
    }
}

impl RenderOnce for NotificationBar {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = theme(cx);
        let (background, edge) = match self.severity {
            Severity::Info => (theme.info, theme.info_edge),
            Severity::Warning => (theme.warning, theme.warning_edge),
            Severity::Error => (theme.error, theme.error_edge),
        };
        let close = self.on_close.map(|on_close| {
            div()
                .id("close")
                .role(Role::Button)
                .aria_label(self.close_label.clone())
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(22.))
                .rounded(px(4.))
                .text_color(rgb(theme.text_muted))
                .hover(|style| style.bg(rgb(theme.hover)).text_color(rgb(theme.text)))
                .on_click(move |_, window, cx| on_close(window, cx))
                .tooltip(Tooltip::builder(self.close_label.clone(), None))
                .child("×")
        });
        div()
            .id(self.id)
            .role(Role::Alert)
            .aria_label(self.message.clone())
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .min_h(px(34.))
            .pl(px(8.))
            .pr(px(6.))
            .py(px(4.))
            .bg(rgb(background))
            .border_b_1()
            .border_color(rgb(theme.border))
            .text_color(rgb(theme.text))
            .child(div().flex_none().w(px(3.)).h(px(18.)).rounded(px(2.)).bg(rgb(edge)))
            .child(div().flex_1().min_w_0().child(self.message))
            .children(self.actions)
            .children(close)
    }
}
