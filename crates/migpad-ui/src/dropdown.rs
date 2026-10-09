//! A list to choose from that shows what is chosen, as the lists of Windows and Linux do: a click,
//! or Enter, Space, Alt+Down or F4 while it has the keyboard, opens its items — a menu, which the
//! owner of the list shows under it — and Up and Down choose the item before or after without
//! opening it.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    App, Bounds, ElementId, FocusHandle, IntoElement, KeyDownEvent, Pixels, RenderOnce, Role, SharedString, Window,
    canvas, div, prelude::*, px, rgb,
};

use crate::theme::theme;

/// Opens the items under the list, whose bounds in the window it is given; and whether a key, not
/// a click, opened them.
type OpenHandler = Rc<dyn Fn(Bounds<Pixels>, bool, &mut Window, &mut App)>;
/// Chooses the item before, -1, or after, 1, the one chosen.
type StepHandler = Rc<dyn Fn(isize, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct Dropdown {
    id: ElementId,
    /// What is chosen.
    label: SharedString,
    /// What a screen reader says the list is for.
    name: SharedString,
    focus: FocusHandle,
    width: Pixels,
    on_open: Option<OpenHandler>,
    on_step: Option<StepHandler>,
}

impl Dropdown {
    /// A list `width` wide, showing `label`, what is chosen; it has the keyboard with `focus`, a
    /// tab stop.
    pub fn new(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        name: impl Into<SharedString>,
        focus: &FocusHandle,
        width: Pixels,
    ) -> Self {
        Dropdown {
            id: id.into(),
            label: label.into(),
            name: name.into(),
            focus: focus.clone(),
            width,
            on_open: None,
            on_step: None,
        }
    }

    pub fn on_open(self, handler: impl Fn(Bounds<Pixels>, bool, &mut Window, &mut App) + 'static) -> Self {
        Dropdown { on_open: Some(Rc::new(handler)), ..self }
    }

    pub fn on_step(self, handler: impl Fn(isize, &mut Window, &mut App) + 'static) -> Self {
        Dropdown { on_step: Some(Rc::new(handler)), ..self }
    }
}

impl RenderOnce for Dropdown {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = theme(cx);
        let focused = self.focus.is_focused(window);
        // Where the list was laid out last, for its items to open under it.
        let bounds = Rc::new(Cell::new(Bounds::default()));
        let measured = bounds.clone();
        let (clicked, keyed) = (self.on_open.clone(), self.on_open);
        let step = self.on_step;
        let at = bounds.clone();
        div()
            .id(self.id)
            .role(Role::ComboBox)
            .aria_label(self.name)
            .track_focus(&self.focus)
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .justify_between()
            .gap(px(8.))
            .w(self.width)
            .h(px(24.))
            .px(px(8.))
            .rounded(px(4.))
            .border_1()
            .border_color(rgb(if focused { theme.accent } else { theme.border }))
            .bg(rgb(theme.editor.background))
            .text_color(rgb(theme.text))
            .hover(|style| style.bg(rgb(theme.hover)))
            // The keys are taken as they are pressed, below: not again as GPUI makes Space and Enter
            // released a click.
            .on_click(move |event, window, cx| {
                if let Some(open) = clicked.as_ref().filter(|_| !event.is_keyboard()) {
                    open(at.get(), false, window, cx);
                }
            })
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                let keystroke = &event.keystroke;
                let modifiers = &keystroke.modifiers;
                let opens = match keystroke.key.as_str() {
                    "enter" | "space" | "f4" => !modifiers.modified(),
                    "down" => modifiers.alt,
                    _ => false,
                };
                if opens {
                    if let Some(open) = &keyed {
                        open(bounds.get(), true, window, cx);
                    }
                    cx.stop_propagation();
                    return;
                }
                let delta = match keystroke.key.as_str() {
                    "up" if !modifiers.modified() => -1,
                    "down" if !modifiers.modified() => 1,
                    _ => return,
                };
                if let Some(step) = &step {
                    step(delta, window, cx);
                }
                cx.stop_propagation();
            })
            .child(div().flex_1().overflow_hidden().whitespace_nowrap().text_ellipsis().child(self.label))
            .child(div().flex_none().text_color(rgb(theme.text_muted)).child("▾"))
            .child(canvas(move |bounds, _, _| measured.set(bounds), |_, _, _, _| {}).absolute().size_full())
    }
}
