//! The menu bar of Windows and Linux, which GPUI does not make: the titles of the menus over the
//! window and the menu that drops down from one, as the menus of those systems behave. The mouse
//! opens a menu and chooses an item; Alt or F10 brings the keyboard to the bar, the arrows move
//! through it, Enter chooses, Esc goes back, and the underlined letters — mnemonics — open a menu
//! with Alt or choose an item of an open one. An item may open a submenu to its right: the pointer
//! over it or the right arrow opens it, the left arrow closes it.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AnyElement, App, Bounds, Context, FocusHandle, KeyDownEvent, Keystroke, Modifiers, MouseButton, Pixels, Render,
    Role, SharedString, Subscription, Window, anchored, canvas, deferred, div, point, prelude::*, px, rgb,
};

pub use crate::menu::ItemSpec;
use crate::menu::{self, Item, Levels, MenuHost, Step, menu_key, mnemonic_char, typed_letters};
use crate::theme::theme;

/// The height of the bar.
const HEIGHT: f32 = 24.;

/// A menu of the bar, as the application describes it each time it is drawn.
pub struct MenuSpec {
    pub title: SharedString,
    /// Where the mnemonic of the title is, in bytes.
    pub mnemonic: Option<usize>,
    pub items: Vec<ItemSpec>,
}

/// What the keyboard and the mouse did to the menus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing outside the menus.
    Stay,
    /// The menus are done with: the focus goes back.
    Leave,
    /// The item at the path of the menu `menu` is chosen: the item of the menu, or of a submenu
    /// down the path.
    Choose(usize, Vec<usize>),
}

/// Where the keyboard is in the menus, apart from drawing them: which title is selected, whether
/// its menu is open, and where in it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Nav {
    pub menu: Option<usize>,
    pub open: bool,
    pub levels: Levels,
}

impl Nav {
    pub fn is_active(&self) -> bool {
        self.menu.is_some()
    }

    /// Alt or F10: the first title is selected, no menu open.
    pub fn select_bar() -> Nav {
        Nav { menu: Some(0), ..Nav::default() }
    }

    /// The menu `menu` open, its first item highlighted when the keyboard opened it.
    pub fn open(menu: usize, items: &[Item], keyboard: bool) -> Nav {
        Nav { menu: Some(menu), open: true, levels: Levels::open(items, keyboard) }
    }

    /// A key while the menus are active, `menus` being the items of each menu and `titles` the
    /// mnemonics of the titles.
    pub fn key(&mut self, key: &str, menus: &[Vec<Item>], titles: &[Option<char>]) -> Outcome {
        let Some(menu) = self.menu else { return Outcome::Stay };
        let count = menus.len();
        let (previous, next) = ((menu + count - 1) % count, (menu + 1) % count);
        if self.open {
            match self.levels.key(key, &menus[menu]) {
                Step::Stay => {}
                Step::Choose(path) => {
                    *self = Nav::default();
                    return Outcome::Choose(menu, path);
                }
                Step::Left => *self = Nav::open(previous, &menus[previous], true),
                // On to the next menu, from a submenu too, as Windows goes.
                Step::Right => *self = Nav::open(next, &menus[next], true),
                Step::Escape => *self = Nav { menu: Some(menu), ..Nav::default() },
            }
            return Outcome::Stay;
        }
        match key {
            "left" => self.menu = Some(previous),
            "right" => self.menu = Some(next),
            "down" | "enter" | "space" => *self = Nav::open(menu, &menus[menu], true),
            "up" => {
                let items = &menus[menu];
                let mut levels = Levels::open(items, false);
                levels.key("up", items);
                *self = Nav { menu: Some(menu), open: true, levels };
            }
            "escape" => {
                *self = Nav::default();
                return Outcome::Leave;
            }
            letter => {
                let letter = letter.chars().next().map(|c| c.to_lowercase().next().unwrap_or(c));
                if let Some(found) = titles.iter().position(|title| letter.is_some() && *title == letter) {
                    *self = Nav::open(found, &menus[found], true);
                }
            }
        }
        Outcome::Stay
    }
}

/// Describes the menus; it gets the focus the chosen item goes to, for which items are enabled.
type MenusSource = Rc<dyn Fn(Option<&FocusHandle>, &mut Window, &mut App) -> Vec<MenuSpec>>;

/// The menu bar of a window.
pub struct MenuBar {
    /// The focus of the menus while the keyboard is in them: on an element no press reaches, so
    /// that presses on the bar do not take the focus from the text.
    focus: FocusHandle,
    nav: Nav,
    /// Whether the keyboard drives the menus now: then the mnemonics are underlined.
    keyboard: bool,
    /// Whether Alt is held: the mnemonics are underlined too.
    alt_held: bool,
    /// Whether Alt is pressed alone, nothing else since: released so, it brings the keyboard to
    /// the menus.
    alt_tap: bool,
    /// The focus before the menus took it, to go back to.
    previous_focus: Option<FocusHandle>,
    menus: MenusSource,
    /// Where the bar is in the window: the menus open under it, over the rest of the window.
    bounds: Rc<Cell<Bounds<Pixels>>>,
    _activation: Subscription,
}

impl MenuBar {
    /// A bar of the menus that `menus` describes each time they are drawn, in `window`.
    pub fn new(
        menus: impl Fn(Option<&FocusHandle>, &mut Window, &mut App) -> Vec<MenuSpec> + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // The menus close when the window is no longer active, as the menus of the system do.
        let activation = cx.observe_window_activation(window, |bar, window, cx| {
            if !window.is_window_active() {
                bar.alt_tap = false;
                if bar.nav.is_active() {
                    bar.leave(window, cx);
                }
            }
        });
        MenuBar {
            focus: cx.focus_handle(),
            nav: Nav::default(),
            keyboard: false,
            alt_held: false,
            alt_tap: false,
            previous_focus: None,
            menus: Rc::new(menus),
            bounds: Rc::default(),
            _activation: activation,
        }
    }

    pub fn is_active(&self) -> bool {
        self.nav.is_active()
    }

    /// The modifiers changed: Alt held shows the mnemonics, and Alt pressed and released alone
    /// brings the keyboard to the menus or back — not when the window is not active, as after
    /// switching windows with Alt+Tab.
    pub fn modifiers_changed(&mut self, modifiers: &Modifiers, window: &mut Window, cx: &mut Context<Self>) {
        let alone = modifiers.alt && modifiers.number_of_modifiers() == 1 && !modifiers.function;
        if alone && !self.alt_held {
            self.alt_tap = true;
        } else if modifiers.number_of_modifiers() > 0 && !alone {
            self.alt_tap = false;
        }
        let released = self.alt_held && !modifiers.alt;
        self.set_alt_held(modifiers.alt && !modifiers.control && !modifiers.platform, cx);
        if released && std::mem::take(&mut self.alt_tap) && window.is_window_active() {
            self.toggle(window, cx);
        }
    }

    /// A key or a press of the mouse while Alt is held: its release is not for the menus.
    pub fn other_input(&mut self) {
        self.alt_tap = false;
    }

    /// Alt or F10: brings the keyboard to the bar, or takes it back to the window.
    pub fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.nav.is_active() {
            self.leave(window, cx);
        } else {
            self.take_focus(window, cx);
            self.nav = Nav::select_bar();
            self.keyboard = true;
            cx.notify();
        }
    }

    /// Alt with a letter: opens the menu with that mnemonic, if there is one; returns whether it did.
    pub fn open_by_mnemonic(&mut self, keystroke: &Keystroke, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let menus = (self.menus)(self.previous_focus.as_ref(), window, cx);
        let typed = typed_letters(keystroke);
        let Some(index) =
            menus.iter().position(|menu| mnemonic_char(&menu.title, menu.mnemonic).is_some_and(|m| typed.contains(&m)))
        else {
            return false;
        };
        let items: Vec<Item> = menus[index].items.iter().map(ItemSpec::kind).collect();
        self.take_focus(window, cx);
        self.nav = Nav::open(index, &items, true);
        self.keyboard = true;
        cx.notify();
        true
    }

    /// Whether Alt is held now: the mnemonics show while it is.
    fn set_alt_held(&mut self, held: bool, cx: &mut Context<Self>) {
        if self.alt_held != held {
            self.alt_held = held;
            cx.notify();
        }
    }

    fn take_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus.is_focused(window) {
            self.previous_focus = window.focused(cx);
            window.focus(&self.focus, cx);
        }
    }

    /// Closes the menus and gives the focus back.
    fn leave(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.nav = Nav::default();
        self.keyboard = false;
        if let Some(previous) = self.previous_focus.take() {
            window.focus(&previous, cx);
        }
        cx.notify();
    }

    /// Chooses the item at `path` of the menu `menu`: the menus close, the focus goes back, and
    /// the action goes to where the focus is.
    fn choose(&mut self, menu: usize, path: &[usize], window: &mut Window, cx: &mut Context<Self>) {
        let menus = (self.menus)(self.previous_focus.as_ref(), window, cx);
        let action = menus.get(menu).and_then(|menu| menu::action_at(&menu.items, path));
        self.leave(window, cx);
        if let Some(action) = action {
            window.defer(cx, move |window, cx| window.dispatch_action(action, cx));
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let modifiers = &keystroke.modifiers;
        if !self.nav.is_active() || modifiers.control || modifiers.platform {
            return;
        }
        let menus = (self.menus)(self.previous_focus.as_ref(), window, cx);
        let items: Vec<Vec<Item>> = menus.iter().map(|menu| menu.items.iter().map(ItemSpec::kind).collect()).collect();
        let titles: Vec<Option<char>> = menus.iter().map(|menu| mnemonic_char(&menu.title, menu.mnemonic)).collect();
        // A letter is the mnemonic of an item of the menu open, or of a title.
        let key = match self.nav.menu.filter(|_| self.nav.open) {
            Some(menu) => menu_key(keystroke, self.nav.levels.current(&items[menu])),
            None => {
                let titles: Vec<Item> =
                    titles.iter().map(|&mnemonic| Item::Choice { enabled: true, mnemonic }).collect();
                menu_key(keystroke, &titles)
            }
        };
        let Some(key) = key else {
            cx.stop_propagation();
            return;
        };
        self.keyboard = true;
        match self.nav.key(&key, &items, &titles) {
            Outcome::Stay => cx.notify(),
            Outcome::Leave => self.leave(window, cx),
            Outcome::Choose(menu, path) => self.choose(menu, &path, window, cx),
        }
        cx.stop_propagation();
    }

    /// A press on a title: opens its menu, or closes it if it is open.
    fn press_title(&mut self, index: usize, items: &[Item], window: &mut Window, cx: &mut Context<Self>) {
        if self.nav.open && self.nav.menu == Some(index) {
            self.leave(window, cx);
        } else {
            self.take_focus(window, cx);
            self.nav = Nav::open(index, items, false);
            self.keyboard = false;
            cx.notify();
        }
    }

    fn render_menu(&self, spec: MenuSpec, cx: &mut Context<Self>) -> AnyElement {
        let panel = menu::panel(self, spec.title, spec.items, 0, 1, cx);
        // Under its title, over everything else.
        let menu = deferred(anchored().snap_to_window().child(panel)).with_priority(1);
        div().absolute().top(px(HEIGHT)).left_0().child(menu).into_any_element()
    }
}

impl MenuHost for MenuBar {
    fn levels(&self) -> &Levels {
        &self.nav.levels
    }

    fn levels_mut(&mut self) -> &mut Levels {
        &mut self.nav.levels
    }

    fn shows_mnemonics(&self) -> bool {
        self.keyboard || self.alt_held
    }

    fn choose(&mut self, path: Vec<usize>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(menu) = self.nav.menu {
            MenuBar::choose(self, menu, &path, window, cx);
        }
    }
}

impl Render for MenuBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme(cx);
        let menus = (self.menus)(self.previous_focus.as_ref(), window, cx);
        // The menus are left when the focus leaves them, as a press on a tab takes it.
        if self.nav.is_active() && !self.focus.contains_focused(window, cx) {
            self.nav = Nav::default();
            self.keyboard = false;
            self.previous_focus = None;
        }
        let open = self.nav.menu.filter(|_| self.nav.open);
        let mut titles: Vec<AnyElement> = Vec::new();
        for (index, spec) in menus.into_iter().enumerate() {
            let (title, mnemonic) = (spec.title.clone(), spec.mnemonic);
            let items: Vec<Item> = spec.items.iter().map(ItemSpec::kind).collect();
            let menu = (open == Some(index)).then(|| self.render_menu(spec, cx));
            let selected = self.nav.menu == Some(index);
            let items_for_hover = items.clone();
            titles.push(
                div()
                    .id(("menu-title", index))
                    .role(Role::MenuItem)
                    .aria_label(title.clone())
                    .aria_expanded(selected && self.nav.open)
                    .relative()
                    .flex()
                    .items_center()
                    .h_full()
                    .px(px(8.))
                    .rounded(px(4.))
                    .when(selected, |title| title.bg(rgb(theme.menu_selected)))
                    .when(selected && !self.nav.open, |title| title.aria_active_descendant())
                    .when(!selected, |title| title.hover(|style| style.bg(rgb(theme.hover))))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |bar, _, window, cx| bar.press_title(index, &items, window, cx)),
                    )
                    // With a menu open, the pointer over another title opens its menu.
                    .on_mouse_move(cx.listener(move |bar, _, window, cx| {
                        if bar.nav.open && bar.nav.menu != Some(index) {
                            bar.take_focus(window, cx);
                            bar.nav = Nav::open(index, &items_for_hover, false);
                            cx.notify();
                        }
                    }))
                    .child(menu::label(&title, mnemonic, self.shows_mnemonics()))
                    .children(menu)
                    .into_any_element(),
            );
        }
        let bounds = self.bounds.clone();
        // Under an open menu, the window takes no presses: one closes the menu, as on Windows.
        let overlay = open.is_some().then(|| {
            let top = self.bounds.get().bottom();
            let size = window.viewport_size();
            let leave = |bar: &mut Self, _: &gpui::MouseDownEvent, window: &mut Window, cx: &mut Context<Self>| {
                bar.leave(window, cx)
            };
            let cover = div()
                .id("menu-cover")
                .w(size.width)
                .h((size.height - top).max(px(0.)))
                .occlude()
                .on_mouse_down(MouseButton::Left, cx.listener(leave))
                .on_mouse_down(MouseButton::Right, cx.listener(leave))
                .on_mouse_down(MouseButton::Middle, cx.listener(leave));
            deferred(anchored().position(point(px(0.), top)).child(cover)).with_priority(0)
        });
        div()
            .id("menu-bar")
            .role(Role::MenuBar)
            .on_key_down(cx.listener(Self::key_down))
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .h(px(HEIGHT))
            .px(px(2.))
            .py(px(1.))
            .bg(rgb(theme.bar))
            .text_color(rgb(theme.text))
            .child(div().absolute().size_0().track_focus(&self.focus))
            .children(titles)
            .children(overlay)
            .child(canvas(move |area, _, _| bounds.set(area), |_, _, _, _| {}).absolute().top_0().left_0().size_full())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: Item = Item::Choice { enabled: true, mnemonic: Some('a') };
    const B: Item = Item::Choice { enabled: true, mnemonic: Some('b') };
    const OFF: Item = Item::Choice { enabled: false, mnemonic: Some('o') };
    const SEP: Item = Item::Separator;

    fn menus() -> (Vec<Vec<Item>>, Vec<Option<char>>) {
        (vec![vec![A, SEP, B], vec![OFF, A], vec![B]], vec![Some('f'), Some('e'), Some('ф')])
    }

    #[test]
    fn arrows_move_over_titles_and_open_their_menus() {
        let (menus, titles) = menus();
        let mut nav = Nav::select_bar();
        assert_eq!(nav.key("left", &menus, &titles), Outcome::Stay);
        assert_eq!((nav.menu, nav.open), (Some(2), false));
        nav.key("right", &menus, &titles);
        nav.key("down", &menus, &titles);
        assert_eq!((nav.menu, nav.open, nav.levels.highlighted(0)), (Some(0), true, Some(0)));
        nav.key("down", &menus, &titles);
        assert_eq!(nav.levels.highlighted(0), Some(2), "past the separator");
        // With a menu open, the next one opens, and the one before.
        nav.key("right", &menus, &titles);
        assert_eq!((nav.menu, nav.levels.highlighted(0)), (Some(1), Some(0)));
        nav.key("left", &menus, &titles);
        assert_eq!(nav.menu, Some(0));
        // Up on a title opens its menu at the last item.
        let mut nav = Nav::select_bar();
        nav.key("up", &menus, &titles);
        assert_eq!((nav.open, nav.levels.highlighted(0)), (true, Some(2)));
    }

    #[test]
    fn enter_chooses_an_enabled_item_and_escape_goes_back() {
        let (menus, titles) = menus();
        let mut nav = Nav::open(1, &menus[1], true);
        assert_eq!(nav.key("enter", &menus, &titles), Outcome::Stay, "a disabled item");
        nav.key("down", &menus, &titles);
        assert_eq!(nav.key("enter", &menus, &titles), Outcome::Choose(1, vec![1]));
        assert!(!nav.is_active());

        let mut nav = Nav::open(0, &menus[0], true);
        assert_eq!(nav.key("escape", &menus, &titles), Outcome::Stay);
        assert_eq!((nav.menu, nav.open), (Some(0), false));
        assert_eq!(nav.key("escape", &menus, &titles), Outcome::Leave);
        assert!(!nav.is_active());
    }

    #[test]
    fn mnemonics_open_menus_and_choose_items() {
        let (menus, titles) = menus();
        let mut nav = Nav::select_bar();
        assert_eq!(nav.key("ф", &menus, &titles), Outcome::Stay);
        assert_eq!((nav.menu, nav.open, nav.levels.highlighted(0)), (Some(2), true, Some(0)));
        let mut nav = Nav::select_bar();
        nav.key("E", &menus, &titles);
        assert_eq!(nav.menu, Some(1));
        assert_eq!(nav.key("o", &menus, &titles), Outcome::Stay, "a disabled item is not chosen");
        assert_eq!(nav.key("a", &menus, &titles), Outcome::Choose(1, vec![1]));
    }

    #[test]
    fn submenus_open_to_the_right_and_the_next_menu_after_them() {
        let recent = Item::Submenu { enabled: true, mnemonic: Some('r'), items: vec![A, SEP, B] };
        let menus = vec![vec![A, recent], vec![B]];
        let titles = vec![Some('f'), Some('v')];
        let mut nav = Nav::open(0, &menus[0], true);
        nav.key("down", &menus, &titles);
        nav.key("right", &menus, &titles);
        assert!(nav.levels.is_open(0, 1));
        nav.key("down", &menus, &titles);
        assert_eq!(nav.key("enter", &menus, &titles), Outcome::Choose(0, vec![1, 2]));
        // Right on an item of a submenu goes on to the next menu.
        let mut nav = Nav::open(0, &menus[0], true);
        nav.key("r", &menus, &titles);
        nav.key("right", &menus, &titles);
        assert_eq!((nav.menu, nav.open), (Some(1), true));
    }
}
