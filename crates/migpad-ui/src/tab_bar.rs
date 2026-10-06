//! The bar of tabs over the text: a tab for each document of the window, with its name, the mark
//! of changes to save and the button that closes it. A press switches to a tab, the middle button
//! closes it, dragging moves it; when the tabs do not fit, the bar scrolls, the wheel too.

use std::rc::Rc;

use gpui::{
    App, AppContext, Context, EntityId, IntoElement, MouseButton, Render, RenderOnce, Role, ScrollHandle,
    SharedString, Window, div, prelude::*, px, rgb,
};

use crate::theme::theme;
use crate::tooltip::Tooltip;

/// What a tab shows.
pub struct TabInfo {
    pub title: SharedString,
    /// The path of the file, shown when the pointer rests on the tab.
    pub tooltip: Option<SharedString>,
    /// Whether the document has changes to save: ● instead of × until the pointer is over the tab.
    pub modified: bool,
}

type Handler<T> = Rc<dyn Fn(T, &mut Window, &mut App)>;

/// The bar of the tabs of a window.
#[derive(IntoElement)]
pub struct TabBar {
    /// The view the tabs are of: a tab dragged from another window does not drop here.
    owner: EntityId,
    tabs: Vec<TabInfo>,
    active: usize,
    scroll: ScrollHandle,
    close_label: SharedString,
    on_select: Handler<usize>,
    on_close: Handler<usize>,
    on_move: Handler<(usize, usize)>,
    on_new: Handler<()>,
}

/// A tab being dragged to another place in the bar.
#[derive(Clone)]
pub struct DraggedTab {
    owner: EntityId,
    index: usize,
    title: SharedString,
}

impl TabBar {
    /// The tabs of `owner`, `active` shown; `scroll` keeps where the bar is scrolled to.
    pub fn new(owner: EntityId, tabs: Vec<TabInfo>, active: usize, scroll: ScrollHandle) -> Self {
        TabBar {
            owner,
            tabs,
            active,
            scroll,
            close_label: SharedString::default(),
            on_select: Rc::new(|_, _, _| {}),
            on_close: Rc::new(|_, _, _| {}),
            on_move: Rc::new(|_, _, _| {}),
            on_new: Rc::new(|_, _, _| {}),
        }
    }

    /// The tooltip of the buttons that close tabs.
    pub fn close_label(self, label: impl Into<SharedString>) -> Self {
        TabBar { close_label: label.into(), ..self }
    }

    pub fn on_select(self, handler: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        TabBar { on_select: Rc::new(handler), ..self }
    }

    pub fn on_close(self, handler: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        TabBar { on_close: Rc::new(handler), ..self }
    }

    /// A tab dragged from one place to another: `(from, to)`.
    pub fn on_move(self, handler: impl Fn((usize, usize), &mut Window, &mut App) + 'static) -> Self {
        TabBar { on_move: Rc::new(handler), ..self }
    }

    /// A double click on the free part of the bar.
    pub fn on_new(self, handler: impl Fn((), &mut Window, &mut App) + 'static) -> Self {
        TabBar { on_new: Rc::new(handler), ..self }
    }
}

impl RenderOnce for TabBar {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = theme(cx);
        let owner = self.owner;
        let count = self.tabs.len();
        let tabs = self.tabs.into_iter().enumerate().map(|(i, tab)| {
            let active = i == self.active;
            let group = SharedString::from(format!("tab-{i}"));
            let (on_select, on_close, on_close_middle, on_move) =
                (self.on_select.clone(), self.on_close.clone(), self.on_close.clone(), self.on_move.clone());
            let dragged = DraggedTab { owner, index: i, title: tab.title.clone() };
            // ● for changes to save; × on the active tab and under the pointer.
            let close = div()
                .id(("close", i))
                .role(Role::Button)
                .aria_label(self.close_label.clone())
                .relative()
                .flex_none()
                .size(px(18.))
                .rounded(px(4.))
                .text_color(rgb(theme.text_muted))
                .hover(|style| style.bg(rgb(theme.hover)).text_color(rgb(theme.text)))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(move |_, window, cx| on_close(i, window, cx))
                .tooltip(Tooltip::builder(self.close_label.clone(), None))
                .child(
                    div()
                        .absolute()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(10.))
                        .child("●")
                        .when(!tab.modified, |dot| dot.invisible())
                        .group_hover(group.clone(), |style| style.invisible()),
                )
                .child(
                    div()
                        .absolute()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child("×")
                        .when(tab.modified || !active, |cross| cross.invisible())
                        .group_hover(group.clone(), |style| style.visible()),
                );
            div()
                .id(("tab", i))
                .group(group)
                .role(Role::Tab)
                .aria_selected(active)
                .aria_label(tab.title.clone())
                .relative()
                .flex_none()
                .flex()
                .items_center()
                .gap(px(4.))
                .h_full()
                .min_w(px(64.))
                .max_w(px(220.))
                .pl(px(12.))
                .pr(px(5.))
                .border_r_1()
                .border_color(rgb(theme.border))
                .when(active, |tab| tab.bg(rgb(theme.tab_active)).text_color(rgb(theme.text)))
                .when(!active, |tab| tab.text_color(rgb(theme.text_muted)).hover(|style| style.bg(rgb(theme.hover))))
                .on_mouse_down(MouseButton::Left, move |_, window, cx| on_select(i, window, cx))
                .on_mouse_up(MouseButton::Middle, move |_, window, cx| on_close_middle(i, window, cx))
                .on_drag(dragged, |dragged, _, _, cx| cx.new(|_| dragged.clone()))
                .drag_over::<DraggedTab>(move |style, dragged, _, _| {
                    if dragged.owner == owner { style.bg(rgb(theme.pressed)) } else { style }
                })
                .on_drop(move |dragged: &DraggedTab, window, cx| {
                    if dragged.owner == owner {
                        on_move((dragged.index, i), window, cx);
                    }
                })
                .when_some(tab.tooltip, |tab, path| tab.tooltip(Tooltip::builder(path, None)))
                // The active tab is marked by a line over it.
                .when(active, |tab| tab.child(div().absolute().top_0().left_0().right_0().h(px(2.)).bg(rgb(theme.accent))))
                .child(div().flex_shrink(1.).min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(tab.title))
                .child(close)
        });
        let (on_move, on_new) = (self.on_move.clone(), self.on_new.clone());
        div()
            .id("tab-bar")
            .role(Role::TabList)
            .flex()
            .flex_none()
            .h(px(32.))
            .bg(rgb(theme.bar))
            .border_b_1()
            .border_color(rgb(theme.border))
            .child(div().id("tabs").flex().h_full().overflow_x_scroll().track_scroll(&self.scroll).children(tabs))
            // The free part: a tab dropped here goes to the end, a double click opens a new one.
            .child(
                div()
                    .id("tab-bar-rest")
                    .flex_1()
                    .h_full()
                    .drag_over::<DraggedTab>(move |style, dragged, _, _| {
                        if dragged.owner == owner { style.bg(rgb(theme.pressed)) } else { style }
                    })
                    .on_drop(move |dragged: &DraggedTab, window, cx| {
                        if dragged.owner == owner {
                            on_move((dragged.index, count - 1), window, cx);
                        }
                    })
                    .on_click(move |event, window, cx| {
                        if event.click_count() == 2 {
                            on_new((), window, cx);
                        }
                    }),
            )
    }
}

impl Render for DraggedTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme(cx);
        div()
            .px(px(12.))
            .py(px(6.))
            .rounded(px(4.))
            .border_1()
            .border_color(rgb(theme.border))
            .bg(rgb(theme.tab_active))
            .text_color(rgb(theme.text))
            .text_size(crate::text_size())
            .opacity(0.9)
            .child(self.title.clone())
    }
}
