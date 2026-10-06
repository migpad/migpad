//! Tooltips: a line of text, with the keys of a command after it.

use gpui::{AnyView, App, AppContext, BoxShadow, Context, Render, SharedString, Window, div, point, prelude::*, px, rgb, rgba};

use crate::theme::theme;

pub struct Tooltip {
    text: SharedString,
    keys: Option<SharedString>,
}

impl Tooltip {
    /// What `.tooltip()` of an element takes: a tooltip with `text`, and `keys` after it.
    pub fn builder(
        text: impl Into<SharedString>,
        keys: Option<SharedString>,
    ) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
        let text = text.into();
        move |_, cx| cx.new(|_| Tooltip { text: text.clone(), keys: keys.clone() }).into()
    }
}

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme(cx);
        let shadow = BoxShadow {
            color: rgba(theme.shadow).into(),
            offset: point(px(0.), px(2.)),
            blur_radius: px(6.),
            spread_radius: px(0.),
            inset: false,
        };
        // Below and to the right of the pointer, clear of it.
        div().pl(px(6.)).pt(px(12.)).child(
            div()
                .flex()
                .gap(px(10.))
                .px(px(7.))
                .py(px(3.))
                .rounded(px(4.))
                .border_1()
                .border_color(rgb(theme.border))
                .bg(rgb(theme.tooltip))
                .shadow(vec![shadow])
                .text_size(crate::text_size())
                .text_color(rgb(theme.text))
                .child(self.text.clone())
                .when_some(self.keys.clone(), |tooltip, keys| {
                    tooltip.child(div().text_color(rgb(theme.text_muted)).child(keys))
                }),
        )
    }
}
