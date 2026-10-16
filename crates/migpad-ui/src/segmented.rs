//! A switch of segments, one of them chosen — as a Markdown document shows as its page or as its
//! text. A click on another segment chooses it.

use std::rc::Rc;

use gpui::{App, ElementId, IntoElement, RenderOnce, Role, SharedString, Window, div, prelude::*, px, rgb};

use crate::theme::theme;
use crate::tooltip::Tooltip;

type Handler = Rc<dyn Fn(usize, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct Segmented {
    id: ElementId,
    labels: Vec<SharedString>,
    chosen: usize,
    /// The tooltip, and the keys of the command the switch is.
    tooltip: Option<(SharedString, Option<SharedString>)>,
    on_choose: Option<Handler>,
}

impl Segmented {
    /// Segments of `labels`, the one at `chosen` chosen.
    pub fn new(
        id: impl Into<ElementId>,
        labels: impl IntoIterator<Item = impl Into<SharedString>>,
        chosen: usize,
    ) -> Self {
        Segmented {
            id: id.into(),
            labels: labels.into_iter().map(Into::into).collect(),
            chosen,
            tooltip: None,
            on_choose: None,
        }
    }

    pub fn tooltip(self, label: impl Into<SharedString>, keys: Option<SharedString>) -> Self {
        Segmented { tooltip: Some((label.into(), keys)), ..self }
    }

    /// What a click on a segment not chosen does: it gets its number.
    pub fn on_choose(self, on_choose: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        Segmented { on_choose: Some(Rc::new(on_choose)), ..self }
    }
}

impl RenderOnce for Segmented {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = theme(cx);
        let segments = self.labels.into_iter().enumerate().map(|(i, label)| {
            let chosen = i == self.chosen;
            let on_choose = self.on_choose.clone();
            div()
                .id(i)
                .role(Role::RadioButton)
                .aria_label(label.clone())
                .flex()
                .items_center()
                .h_full()
                .px(px(9.))
                .rounded(px(4.))
                .when(chosen, |segment| {
                    segment
                        .bg(rgb(theme.tab_active))
                        .text_color(rgb(theme.text))
                        .border_1()
                        .border_color(rgb(theme.border))
                })
                .when(!chosen, |segment| {
                    segment.text_color(rgb(theme.text_muted)).hover(|style| style.text_color(rgb(theme.text))).on_click(
                        move |_, window, cx| {
                            if let Some(on_choose) = &on_choose {
                                on_choose(i, window, cx);
                            }
                        },
                    )
                })
                .child(label)
        });
        div()
            .id(self.id)
            .role(Role::RadioGroup)
            .flex()
            .flex_none()
            .items_center()
            .h(px(24.))
            .my_auto()
            .mx(px(6.))
            .p(px(2.))
            .gap(px(2.))
            .rounded(px(6.))
            .bg(rgb(theme.hover))
            .when_some(self.tooltip, |switch, (label, keys)| switch.tooltip(Tooltip::builder(label, keys)))
            .children(segments)
    }
}
