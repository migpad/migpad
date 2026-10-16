//! The bar of tabs over the text, always shown, one tab or many: a tab for each document of the
//! window, with its name, the mark of changes to save and the button that closes it, and after the
//! last tab a button that opens a new one. A press switches to a tab, the middle button closes it,
//! dragging moves it; when the tabs do not fit, they scroll, the wheel too, and the button of a new
//! tab stays at the right edge.

use std::rc::Rc;

use gpui::{
    AnyElement, App, AppContext, Bounds, Context, EntityId, Global, IntoElement, MouseButton, Pixels, Point, Render,
    RenderOnce, Role, ScrollHandle, SharedString, Window, WindowId, div, prelude::*, px, rgb,
};

use crate::theme::theme;
use crate::tooltip::Tooltip;

/// What a tab shows.
pub struct TabInfo {
    /// The document of the tab, to find it after a drag.
    pub document: EntityId,
    pub title: SharedString,
    /// The path of the file, shown when the pointer rests on the tab.
    pub tooltip: Option<SharedString>,
    /// Whether the document has changes to save: ● instead of × until the pointer is over the tab.
    pub modified: bool,
    /// Whether its file is not there any more: the title is struck through.
    pub missing: bool,
    /// Whether the tab is pinned: it shows a pin icon instead of a close button.
    pub pinned: bool,
}

type Handler<T> = Rc<dyn Fn(T, &mut Window, &mut App)>;

/// The tab being dragged, from the start of the drag: GPUI keeps it to itself. And the window over
/// which it goes outside its own, and where among the tabs there it would go: that bar shows it,
/// since GPUI draws the dragged tab only in its own window.
#[derive(Default)]
struct Dragging {
    tab: Option<DraggedTab>,
    over: Option<(WindowId, usize)>,
}

impl Global for Dragging {}

/// What the application does as a drag of a tab ends, wherever the tab was let go of.
struct DragEnd(Rc<dyn Fn(&mut App)>);

impl Global for DragEnd {}

/// The tab being dragged, if a drag of a tab started and was not taken yet: for a window that gets
/// the button back outside of itself, where no other window sees the drop.
pub fn take_dragged_tab(cx: &mut App) -> Option<DraggedTab> {
    cx.try_global::<Dragging>()?;
    cx.global_mut::<Dragging>().tab.take()
}

/// Shows in the bar of the window `over` — before its tab of that index — where the tab being
/// dragged outside its window would go, or in no bar.
pub fn show_tab_drop(over: Option<(WindowId, usize)>, cx: &mut App) {
    let was = std::mem::replace(&mut cx.default_global::<Dragging>().over, over);
    if was == over {
        return;
    }
    for id in [was, over].into_iter().flatten().map(|(id, _)| id) {
        if let Some(window) = cx.windows().into_iter().find(|window| window.window_id() == id) {
            let _ = window.update(cx, |_, window, _| window.refresh());
        }
    }
}

/// Calls `handler` as each drag of a tab ends.
pub fn on_tab_drag_end(handler: impl Fn(&mut App) + 'static, cx: &mut App) {
    cx.set_global(DragEnd(Rc::new(handler)));
}

/// Where a tab dropped at `point` of a window goes among the tabs drawn at `tabs` — the bounds a
/// [`ScrollHandle`] of the bar keeps, moved by its offset: before the first tab whose middle is
/// right of it. `None` off the height of the bar.
pub fn drop_position(tabs: &[Bounds<Pixels>], point: Point<Pixels>) -> Option<usize> {
    let first = tabs.first()?;
    if point.y < first.top() || point.y >= first.bottom() {
        return None;
    }
    Some(tabs.iter().filter(|tab| tab.center().x < point.x).count())
}

/// The bar of the tabs of a window.
#[derive(IntoElement)]
pub struct TabBar {
    owner: EntityId,
    window: WindowId,
    tabs: Vec<TabInfo>,
    active: usize,
    scroll: ScrollHandle,
    close_label: SharedString,
    /// The tooltip of the button of a new tab, and its keys.
    new_label: (SharedString, Option<SharedString>),
    on_select: Handler<usize>,
    on_close: Handler<usize>,
    on_move: Handler<(usize, usize)>,
    on_new: Handler<()>,
    on_context_menu: Handler<(usize, Point<Pixels>)>,
    /// A tab dragged from another window, dropped at a position.
    on_receive: Handler<(DraggedTab, usize)>,
    /// What shows at the right end of the bar: how the active tab shows its document.
    trailing: Option<AnyElement>,
}

/// A tab being dragged to another place in the bar, or to another window.
#[derive(Clone)]
pub struct DraggedTab {
    owner: EntityId,
    window: WindowId,
    document: EntityId,
    index: usize,
    title: SharedString,
}

impl DraggedTab {
    pub fn source_window(&self) -> WindowId {
        self.window
    }

    pub fn document_id(&self) -> EntityId {
        self.document
    }

    pub fn source_index(&self) -> usize {
        self.index
    }

    pub fn title(&self) -> &str {
        &self.title
    }
}

impl TabBar {
    /// The tabs of `owner` in `window`, `active` shown; `scroll` keeps where the bar is scrolled to.
    pub fn new(owner: EntityId, window: WindowId, tabs: Vec<TabInfo>, active: usize, scroll: ScrollHandle) -> Self {
        TabBar {
            owner,
            window,
            tabs,
            active,
            scroll,
            close_label: SharedString::default(),
            new_label: (SharedString::default(), None),
            on_select: Rc::new(|_, _, _| {}),
            on_close: Rc::new(|_, _, _| {}),
            on_move: Rc::new(|_, _, _| {}),
            on_new: Rc::new(|_, _, _| {}),
            on_context_menu: Rc::new(|_, _, _| {}),
            on_receive: Rc::new(|_, _, _| {}),
            trailing: None,
        }
    }

    /// What shows at the right end of the bar.
    pub fn trailing(self, element: impl IntoElement) -> Self {
        TabBar { trailing: Some(element.into_any_element()), ..self }
    }

    /// The tooltip of the buttons that close tabs.
    pub fn close_label(self, label: impl Into<SharedString>) -> Self {
        TabBar { close_label: label.into(), ..self }
    }

    /// The tooltip of the button of a new tab, and the keys of the command.
    pub fn new_label(self, label: impl Into<SharedString>, keys: Option<SharedString>) -> Self {
        TabBar { new_label: (label.into(), keys), ..self }
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

    /// The button of a new tab, or a double click on the free part of the bar.
    pub fn on_new(self, handler: impl Fn((), &mut Window, &mut App) + 'static) -> Self {
        TabBar { on_new: Rc::new(handler), ..self }
    }

    /// A right click on a tab: its index and the position of the pointer.
    pub fn on_context_menu(self, handler: impl Fn((usize, Point<Pixels>), &mut Window, &mut App) + 'static) -> Self {
        TabBar { on_context_menu: Rc::new(handler), ..self }
    }

    /// A tab dragged from another window, dropped at a position among the tabs.
    pub fn on_receive(self, handler: impl Fn((DraggedTab, usize), &mut Window, &mut App) + 'static) -> Self {
        TabBar { on_receive: Rc::new(handler), ..self }
    }
}

impl RenderOnce for TabBar {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = theme(cx);
        let owner = self.owner;
        let window = self.window;
        let count = self.tabs.len();
        // A tab dragged from another window over this one: where it would go, marked with a line.
        let over = cx.try_global::<Dragging>().and_then(|dragging| dragging.over);
        let drop_at = over.filter(|(over, _)| *over == window).map(|(_, index)| index);
        let marker = move || div().absolute().top_0().bottom_0().w(px(2.)).bg(rgb(theme.accent));
        let tabs = self.tabs.into_iter().enumerate().map(|(i, tab)| {
            let active = i == self.active;
            let pinned = tab.pinned;
            let tab_title = tab.title.clone();
            let tab_missing = tab.missing;
            let group = SharedString::from(format!("tab-{i}"));
            let (on_select, on_close, on_close_middle, on_move) =
                (self.on_select.clone(), self.on_close.clone(), self.on_close.clone(), self.on_move.clone());
            let (on_context_menu, on_receive) = (self.on_context_menu.clone(), self.on_receive.clone());
            let dragged = DraggedTab { owner, window, document: tab.document, index: i, title: tab.title.clone() };
            // Pinned tabs show a pin, or ● for changes to save; unpinned show ● or × to close.
            let trailing = if pinned {
                let icon =
                    div().id(("close", i)).relative().flex_none().size(px(18.)).text_color(rgb(theme.text_muted));
                if tab.modified {
                    icon.child(
                        div()
                            .absolute()
                            .size_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(10.))
                            .child("●"),
                    )
                } else {
                    icon.child(
                        div()
                            .absolute()
                            .left(px(6.5))
                            .top(px(3.))
                            .size(px(5.))
                            .rounded(px(2.5))
                            .border_1()
                            .border_color(rgb(theme.text_muted)),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(px(8.25))
                            .top(px(8.))
                            .w(px(1.5))
                            .h(px(7.))
                            .rounded(px(0.75))
                            .bg(rgb(theme.text_muted)),
                    )
                }
            } else {
                let close_label = self.close_label.clone();
                div()
                    .id(("close", i))
                    .role(Role::Button)
                    .aria_label(close_label.clone())
                    .relative()
                    .flex_none()
                    .size(px(18.))
                    .rounded(px(4.))
                    .text_color(rgb(theme.text_muted))
                    .hover(|style| style.bg(rgb(theme.hover)).text_color(rgb(theme.text)))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(move |_, window, cx| on_close(i, window, cx))
                    .tooltip(Tooltip::builder(close_label, None))
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
                    )
            };
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
                .on_mouse_down(MouseButton::Right, move |event, window, cx| {
                    on_context_menu((i, event.position), window, cx);
                    cx.stop_propagation();
                })
                .when(!pinned, |tab| {
                    tab.on_mouse_up(MouseButton::Middle, move |_, window, cx| on_close_middle(i, window, cx))
                })
                .on_drag(dragged, |dragged, _, _, cx| {
                    cx.set_global(Dragging { tab: Some(dragged.clone()), over: None });
                    let view = cx.new(|_| dragged.clone());
                    // The drag ends with its view, wherever it is dropped: the tab is forgotten.
                    cx.observe_release(&view, |_, cx| {
                        show_tab_drop(None, cx);
                        cx.global_mut::<Dragging>().tab = None;
                        if let Some(DragEnd(handler)) = cx.try_global::<DragEnd>() {
                            handler.clone()(cx);
                        }
                    })
                    .detach();
                    view
                })
                .drag_over::<DraggedTab>(move |style, _, _, _| style.bg(rgb(theme.pressed)))
                .on_drop(move |dragged: &DraggedTab, window, cx| {
                    if dragged.owner == owner {
                        on_move((dragged.index, i), window, cx);
                    } else {
                        on_receive((dragged.clone(), i), window, cx);
                    }
                })
                .when_some(tab.tooltip, |tab, path| tab.tooltip(Tooltip::builder(path, None)))
                // The active tab is marked by a line over it.
                .when(active, |tab| {
                    tab.child(div().absolute().top_0().left_0().right_0().h(px(2.)).bg(rgb(theme.accent)))
                })
                .when(drop_at == Some(i), |tab| tab.child(marker().left_0()))
                .when(drop_at == Some(count) && i + 1 == count, |tab| tab.child(marker().right_0()))
                .child(
                    div()
                        .flex_shrink(1.)
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .when(tab_missing, |title| title.line_through())
                        .child(tab_title.clone()),
                )
                .child(trailing)
        });
        let (on_move, on_new, on_plus, on_receive_end) =
            (self.on_move.clone(), self.on_new.clone(), self.on_new.clone(), self.on_receive.clone());
        // A plus of two thin lines, crisp at any scale.
        let line = || div().absolute().rounded(px(0.75)).bg(rgb(theme.text_muted));
        let glyph = div()
            .relative()
            .size(px(11.))
            .child(line().left_0().top(px(4.75)).w(px(11.)).h(px(1.5)))
            .child(line().top_0().left(px(4.75)).w(px(1.5)).h(px(11.)));
        let plus = div()
            .id("new-tab")
            .role(Role::Button)
            .aria_label(self.new_label.0.clone())
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(24.))
            .mx(px(4.))
            .my_auto()
            .rounded(px(4.))
            .hover(|style| style.bg(rgb(theme.hover)))
            .active(|style| style.bg(rgb(theme.pressed)))
            .on_click(move |_, window, cx| on_plus((), window, cx))
            .tooltip(Tooltip::builder(self.new_label.0, self.new_label.1))
            .child(glyph);
        div()
            .id("tab-bar")
            .role(Role::TabList)
            .flex()
            .flex_none()
            .h(px(32.))
            .bg(rgb(if drop_at.is_some() { theme.pressed } else { theme.bar }))
            .border_b_1()
            .border_color(rgb(theme.border))
            // The tabs give way to the button when they do not fit: it stays in sight.
            .child(
                div()
                    .id("tabs")
                    .flex()
                    .flex_shrink(1.)
                    .min_w_0()
                    .h_full()
                    .overflow_x_scroll()
                    .track_scroll(&self.scroll)
                    .children(tabs),
            )
            .child(plus)
            // The free part: a tab dropped here goes to the end, a double click opens a new one.
            .child(
                div()
                    .id("tab-bar-rest")
                    .flex_1()
                    .h_full()
                    .drag_over::<DraggedTab>(move |style, _, _, _| style.bg(rgb(theme.pressed)))
                    .on_drop(move |dragged: &DraggedTab, window, cx| {
                        if dragged.owner == owner {
                            on_move((dragged.index, count - 1), window, cx);
                        } else {
                            on_receive_end((dragged.clone(), count), window, cx);
                        }
                    })
                    .on_click(move |event, window, cx| {
                        if event.click_count() == 2 {
                            on_new((), window, cx);
                        }
                    }),
            )
            .children(self.trailing)
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

#[cfg(test)]
mod tests {
    use gpui::{point, size};

    use super::*;

    #[test]
    fn a_tab_goes_before_the_first_tab_whose_middle_is_right_of_it() {
        let tabs: Vec<_> =
            [0., 100., 200.].iter().map(|&x| Bounds::new(point(px(x), px(25.)), size(px(100.), px(32.)))).collect();
        assert_eq!(drop_position(&tabs, point(px(10.), px(40.))), Some(0));
        assert_eq!(drop_position(&tabs, point(px(160.), px(40.))), Some(2));
        assert_eq!(drop_position(&tabs, point(px(900.), px(30.))), Some(3));
        assert_eq!(drop_position(&tabs, point(px(160.), px(10.))), None);
        assert_eq!(drop_position(&tabs, point(px(160.), px(57.))), None);
        assert_eq!(drop_position(&[], point(px(0.), px(0.))), None);
    }
}
