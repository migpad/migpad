//! Context menus: a menu that opens where the right button was pressed, or at the caret with the
//! keys of the menu, over everything in the window — the panel of the menus of the menu bar, with
//! its submenus and its keyboard. A press outside closes it, as Esc does, and so does choosing an
//! item, whose action goes to where the focus was.

use gpui::{
    Context, DismissEvent, EventEmitter, FocusHandle, Focusable, KeyDownEvent, MouseButton, MouseDownEvent, Pixels,
    Point, Render, SharedString, Subscription, Window, anchored, deferred, div, point, prelude::*, px,
};

use crate::menu::{self, Item, ItemSpec, Levels, MenuHost, Step, menu_key};

/// Over the menu bar and its menus, which never show with a context menu.
const PRIORITY: usize = 20;

pub struct ContextMenu {
    focus: FocusHandle,
    items: Vec<ItemSpec>,
    /// Where it opens, in the window: it goes the other way at the edges.
    position: Point<Pixels>,
    levels: Levels,
    /// Whether the keyboard drives the menu: then the mnemonics are underlined.
    keyboard: bool,
    /// The focus before the menu took it, to give back.
    previous_focus: Option<FocusHandle>,
    _activation: Subscription,
}

impl EventEmitter<DismissEvent> for ContextMenu {}

impl ContextMenu {
    /// The menu of `items` at `position` in the window; `keyboard` if a key opened it, which
    /// highlights its first item. It takes the focus, and gives it back when it closes.
    pub fn new(
        items: Vec<ItemSpec>,
        position: Point<Pixels>,
        keyboard: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // The menu closes when the window is no longer active, as the menus of the system do.
        let activation = cx.observe_window_activation(window, |menu, window, cx| {
            if !window.is_window_active() {
                menu.dismiss(window, cx);
            }
        });
        let kinds: Vec<Item> = items.iter().map(ItemSpec::kind).collect();
        let focus = cx.focus_handle();
        let previous_focus = window.focused(cx);
        window.focus(&focus, cx);
        ContextMenu {
            focus,
            items,
            position,
            levels: Levels::open(&kinds, keyboard),
            keyboard,
            previous_focus,
            _activation: activation,
        }
    }

    /// Closes the menu: the focus goes back.
    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(previous) = self.previous_focus.take() {
            window.focus(&previous, cx);
        }
        cx.emit(DismissEvent);
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        if keystroke.modifiers.control || keystroke.modifiers.platform {
            return;
        }
        let kinds: Vec<Item> = self.items.iter().map(ItemSpec::kind).collect();
        // The menus of macOS have no mnemonics.
        let mnemonics: &[Item] = if cfg!(target_os = "macos") { &[] } else { self.levels.current(&kinds) };
        let Some(key) = menu_key(keystroke, mnemonics) else {
            cx.stop_propagation();
            return;
        };
        self.keyboard = true;
        match self.levels.key(&key, &kinds) {
            Step::Stay | Step::Left | Step::Right => cx.notify(),
            Step::Escape => self.dismiss(window, cx),
            Step::Choose(path) => MenuHost::choose(self, path, window, cx),
        }
        cx.stop_propagation();
    }
}

impl MenuHost for ContextMenu {
    fn levels(&self) -> &Levels {
        &self.levels
    }

    fn levels_mut(&mut self) -> &mut Levels {
        &mut self.levels
    }

    fn shows_mnemonics(&self) -> bool {
        self.keyboard && !cfg!(target_os = "macos")
    }

    fn choose(&mut self, path: Vec<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let action = menu::action_at(&self.items, &path);
        self.dismiss(window, cx);
        if let Some(action) = action {
            window.defer(cx, move |window, cx| window.dispatch_action(action, cx));
        }
    }
}

impl Focusable for ContextMenu {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ContextMenu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let size = window.viewport_size();
        // Under the menu, the window takes no presses: one closes the menu.
        let dismiss =
            |menu: &mut Self, _: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>| menu.dismiss(window, cx);
        let cover = div()
            .id("context-menu-cover")
            .w(size.width)
            .h(size.height)
            .occlude()
            .on_mouse_down(MouseButton::Left, cx.listener(dismiss))
            .on_mouse_down(MouseButton::Right, cx.listener(dismiss))
            .on_mouse_down(MouseButton::Middle, cx.listener(dismiss));
        let panel = menu::panel(self, SharedString::default(), self.items.clone(), 0, PRIORITY + 1, cx);
        div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .absolute()
            .size_0()
            .child(deferred(anchored().position(point(px(0.), px(0.))).child(cover)).with_priority(PRIORITY))
            .child(deferred(anchored().position(self.position).child(panel)).with_priority(PRIORITY + 1))
    }
}
