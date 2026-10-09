//! A check box with its label: a click, or Space released while it has the keyboard, turns it on
//! or off, as on Windows; Enter does not.

use std::rc::Rc;

use gpui::{
    App, ClickEvent, ElementId, FocusHandle, IntoElement, KeyboardButton, KeyboardClickEvent, RenderOnce, Role,
    SharedString, Toggled, Window, div, prelude::*, px, rgb,
};

use crate::theme::theme;

type ToggleHandler = Rc<dyn Fn(bool, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct Checkbox {
    id: ElementId,
    label: SharedString,
    checked: bool,
    focus: FocusHandle,
    on_toggle: Option<ToggleHandler>,
}

impl Checkbox {
    /// A check box that has the keyboard with `focus`, a tab stop.
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>, checked: bool, focus: &FocusHandle) -> Self {
        Checkbox { id: id.into(), label: label.into(), checked, focus: focus.clone(), on_toggle: None }
    }

    /// What is done when it is turned on or off: it is told which.
    pub fn on_toggle(self, handler: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        Checkbox { on_toggle: Some(Rc::new(handler)), ..self }
    }
}

impl RenderOnce for Checkbox {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = theme(cx);
        let focused = self.focus.is_focused(window);
        let checked = self.checked;
        let toggled = self.on_toggle;
        div()
            .id(self.id)
            .role(Role::CheckBox)
            .aria_label(self.label.clone())
            .aria_toggled(if checked { Toggled::True } else { Toggled::False })
            .track_focus(&self.focus)
            .flex()
            .items_center()
            .gap(px(6.))
            .text_color(rgb(theme.text))
            // GPUI makes Space and Enter released on an element with the keyboard a click.
            .on_click(move |event, window, cx| {
                let enter =
                    matches!(event, ClickEvent::Keyboard(KeyboardClickEvent { button: KeyboardButton::Enter, .. }));
                if let Some(toggle) = toggled.as_ref().filter(|_| !enter) {
                    toggle(!checked, window, cx);
                }
            })
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(14.))
                    .rounded(px(3.))
                    .border_1()
                    .border_color(rgb(if focused || checked { theme.accent } else { theme.border }))
                    .bg(rgb(if checked { theme.accent } else { theme.editor.background }))
                    .when(focused, |box_| box_.shadow_xs())
                    .when(checked, |box_| box_.text_color(rgb(theme.tab_active)).text_size(px(11.)).child("✓")),
            )
            .child(self.label)
    }
}
