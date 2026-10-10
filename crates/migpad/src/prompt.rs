//! The questions MigPad asks over a window where the system has none GPUI can show: on Linux. The
//! message, its detail wrapped to the width of the box, and the buttons in a row on the right, in
//! the colors of the theme. The keyboard answers as in the dialogs of the systems: Tab, Shift+Tab
//! and the arrows go from button to button, Enter and Space press the one with the focus, Escape
//! the Cancel button.

use gpui::{
    App, BoxShadow, Context, EventEmitter, FocusHandle, Focusable, FontWeight, KeyDownEvent, PromptButton,
    PromptHandle, PromptLevel, PromptResponse, Render, RenderablePromptHandle, Role, SharedString, Window, div, point,
    prelude::*, px, rgb, rgba,
};
use migpad_ui::{text_size, theme};

/// The width of the box, where the window is wide enough.
const WIDTH: f32 = 420.;

/// Puts the questions of MigPad in place of the plain ones GPUI draws itself; macOS and Windows ask
/// with dialogs of the system.
pub fn init(cx: &mut App) {
    if cfg!(not(any(target_os = "macos", target_os = "windows"))) {
        cx.set_prompt_builder(build);
    }
}

fn build(
    _: PromptLevel,
    message: &str,
    detail: Option<&str>,
    buttons: &[PromptButton],
    handle: PromptHandle,
    window: &mut Window,
    cx: &mut App,
) -> RenderablePromptHandle {
    let view = cx.new(|cx| Prompt::new(message, detail, buttons, cx));
    handle.with_view(view, window, cx)
}

/// A question: its buttons, and the one with the keyboard focus.
struct Prompt {
    message: SharedString,
    detail: Option<SharedString>,
    buttons: Vec<PromptButton>,
    focused: usize,
    focus: FocusHandle,
}

impl EventEmitter<PromptResponse> for Prompt {}

impl Focusable for Prompt {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Prompt {
    fn new(message: &str, detail: Option<&str>, buttons: &[PromptButton], cx: &mut Context<Self>) -> Self {
        Prompt {
            message: message.to_owned().into(),
            detail: detail.filter(|detail| !detail.is_empty()).map(|detail| detail.to_owned().into()),
            buttons: buttons.to_vec(),
            focused: default_button(buttons),
            focus: cx.focus_handle(),
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let count = self.buttons.len();
        let modifiers = event.keystroke.modifiers;
        if count == 0 || modifiers.control || modifiers.alt || modifiers.platform || modifiers.function {
            return;
        }
        match event.keystroke.key.as_str() {
            "tab" if modifiers.shift => self.focused = (self.focused + count - 1) % count,
            "tab" => self.focused = (self.focused + 1) % count,
            "left" => self.focused = self.focused.saturating_sub(1),
            "right" => self.focused = (self.focused + 1).min(count - 1),
            "enter" | "space" => cx.emit(PromptResponse(self.focused)),
            "escape" => {
                if let Some(index) = escape_button(&self.buttons) {
                    cx.emit(PromptResponse(index));
                }
            }
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }
}

/// The button Enter presses before the focus moves: the first that does not cancel.
fn default_button(buttons: &[PromptButton]) -> usize {
    buttons.iter().position(|button| !button.is_cancel()).unwrap_or(0)
}

/// The button Escape presses: Cancel; OK where there is no Cancel, or the only button.
fn escape_button(buttons: &[PromptButton]) -> Option<usize> {
    buttons
        .iter()
        .position(PromptButton::is_cancel)
        .or_else(|| buttons.iter().position(|button| matches!(button, PromptButton::Ok(_))))
        .or_else(|| (buttons.len() == 1).then_some(0))
}

impl Render for Prompt {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme(cx);
        let width = WIDTH.min(f32::from(window.viewport_size().width) - 32.).max(200.);
        let shadow = BoxShadow {
            color: rgba(theme.shadow).into(),
            offset: point(px(0.), px(4.)),
            blur_radius: px(16.),
            spread_radius: px(0.),
            inset: false,
        };
        let buttons = self.buttons.iter().enumerate().map(|(index, button)| {
            div()
                .id(index)
                .role(Role::Button)
                .aria_label(button.label().clone())
                .flex()
                .items_center()
                .justify_center()
                .h(px(28.))
                .min_w(px(88.))
                .px(px(12.))
                .rounded(px(5.))
                .border_1()
                .border_color(rgb(if index == self.focused { theme.accent } else { theme.border }))
                .whitespace_nowrap()
                .hover(|style| style.bg(rgb(theme.hover)))
                .active(|style| style.bg(rgb(theme.pressed)))
                .on_click(cx.listener(move |_, _, _, cx| cx.emit(PromptResponse(index))))
                .child(button.label().clone())
        });
        // Over the window, dimmed: the box takes the keys and the mouse until it is answered.
        div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgba(theme.shadow))
            .occlude()
            .child(
                div()
                    .id("prompt")
                    .role(Role::AlertDialog)
                    .aria_label(self.message.clone())
                    .w(px(width))
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .p(px(20.))
                    .bg(rgb(theme.menu))
                    .border_1()
                    .border_color(rgb(theme.border))
                    .rounded(px(8.))
                    .shadow(vec![shadow])
                    .text_size(text_size())
                    .text_color(rgb(theme.text))
                    .child(div().text_size(text_size() * 1.1).font_weight(FontWeight::BOLD).child(self.message.clone()))
                    .children(self.detail.clone().map(|detail| div().child(detail)))
                    .child(div().mt(px(14.)).flex().flex_wrap().justify_end().gap(px(8.)).children(buttons)),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enter_saves_and_escape_cancels() {
        let buttons = [PromptButton::new("Save"), PromptButton::new("Don't Save"), PromptButton::cancel("Cancel")];
        assert_eq!(default_button(&buttons), 0);
        assert_eq!(escape_button(&buttons), Some(2));
    }

    #[test]
    fn escape_takes_ok_without_cancel() {
        let buttons = [PromptButton::ok("OK"), PromptButton::new("Third-Party Licenses")];
        assert_eq!(default_button(&buttons), 0);
        assert_eq!(escape_button(&buttons), Some(0));
        assert_eq!(escape_button(&[PromptButton::new("Close")]), Some(0));
        assert_eq!(escape_button(&[PromptButton::new("Yes"), PromptButton::new("No")]), None);
    }

    #[test]
    fn a_cancel_first_is_not_the_default() {
        assert_eq!(default_button(&[PromptButton::cancel("Cancel"), PromptButton::new("Delete")]), 1);
    }
}
