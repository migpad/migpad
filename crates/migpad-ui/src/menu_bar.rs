//! The menu bar of Windows and Linux, which GPUI does not make: the titles of the menus over the
//! window and the menu that drops down from one, as the menus of those systems behave. The mouse
//! opens a menu and chooses an item; Alt or F10 brings the keyboard to the bar, the arrows move
//! through it, Enter chooses, Esc goes back, and the underlined letters — mnemonics — open a menu
//! with Alt or choose an item of an open one. An item may open a submenu to its right: the pointer
//! over it or the right arrow opens it, the left arrow closes it.

use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;

use gpui::{
    Action, AnyElement, App, Bounds, BoxShadow, Context, FocusHandle, HighlightStyle, KeyDownEvent, Keystroke,
    Modifiers, MouseButton, MouseDownEvent, Pixels, Render, Role, SharedString, StyledText, Subscription, Toggled,
    UnderlineStyle, Window, anchored, canvas, deferred, div, point, prelude::*, px, rgb, rgba,
};

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

/// An item of a menu.
pub enum ItemSpec {
    Action {
        label: SharedString,
        mnemonic: Option<usize>,
        /// The keys of the command, shown on the right.
        keys: Option<SharedString>,
        /// Whether a toggle is on; `None` for an item that is not a toggle.
        checked: Option<bool>,
        enabled: bool,
        action: Box<dyn Action>,
    },
    /// An item that opens a menu of its own.
    Submenu {
        label: SharedString,
        mnemonic: Option<usize>,
        enabled: bool,
        items: Vec<ItemSpec>,
    },
    Separator,
}

impl ItemSpec {
    fn kind(&self) -> Item {
        match self {
            ItemSpec::Action { label, mnemonic, enabled, .. } => {
                Item::Choice { enabled: *enabled, mnemonic: mnemonic_char(label, *mnemonic) }
            }
            ItemSpec::Submenu { label, mnemonic, enabled, items } => Item::Submenu {
                enabled: *enabled,
                mnemonic: mnemonic_char(label, *mnemonic),
                items: items.iter().map(ItemSpec::kind).collect(),
            },
            ItemSpec::Separator => Item::Separator,
        }
    }
}

/// The letter at `mnemonic` of `text`, in lower case.
fn mnemonic_char(text: &str, mnemonic: Option<usize>) -> Option<char> {
    text.get(mnemonic?..)?.chars().next().map(|c| c.to_lowercase().next().unwrap_or(c))
}

/// The bytes of the letter at `mnemonic` of `text`.
fn mnemonic_range(text: &str, mnemonic: Option<usize>) -> Option<Range<usize>> {
    let at = mnemonic?;
    let c = text.get(at..)?.chars().next()?;
    Some(at..at + c.len_utf8())
}

/// What the items of a menu are, as the keyboard moves over them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    Choice {
        enabled: bool,
        mnemonic: Option<char>,
    },
    /// An item with a menu of its own, of these items.
    Submenu {
        enabled: bool,
        mnemonic: Option<char>,
        items: Vec<Item>,
    },
    Separator,
}

impl Item {
    fn mnemonic(&self) -> Option<char> {
        match self {
            Item::Choice { mnemonic, .. } | Item::Submenu { mnemonic, .. } => *mnemonic,
            Item::Separator => None,
        }
    }

    fn is_enabled(&self) -> bool {
        matches!(self, Item::Choice { enabled: true, .. } | Item::Submenu { enabled: true, .. })
    }

    /// The items of its menu, for an item that opens one.
    fn submenu(&self) -> Option<&[Item]> {
        match self {
            Item::Submenu { enabled: true, items, .. } => Some(items),
            _ => None,
        }
    }
}

/// What the keyboard and the mouse did to the menus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing outside the menus.
    Stay,
    /// The menus are done with: the focus goes back.
    Leave,
    /// The item `item` of the menu `menu` is chosen, or the item `sub` of its submenu.
    Choose(usize, usize, Option<usize>),
}

/// Where the keyboard is in the menus, apart from drawing them: which title is selected, whether
/// its menu is open, which item of it is highlighted, and the submenu of that item.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Nav {
    pub menu: Option<usize>,
    pub open: bool,
    pub item: Option<usize>,
    /// The submenu of the highlighted item, if it is open.
    pub sub: Option<Sub>,
}

/// An open submenu: which of its items is highlighted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sub {
    pub item: Option<usize>,
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
        let item = if keyboard { next_choice(items, None, true) } else { None };
        Nav { menu: Some(menu), open: true, item, sub: None }
    }

    /// A key while the menus are active, `menus` being the items of each menu and `titles` the
    /// mnemonics of the titles.
    pub fn key(&mut self, key: &str, menus: &[Vec<Item>], titles: &[Option<char>]) -> Outcome {
        let Some(menu) = self.menu else { return Outcome::Stay };
        let count = menus.len();
        let items = &menus[menu];
        if let Some(sub) = self.sub
            && let Some(sub_items) = self.item.and_then(|item| items.get(item)).and_then(Item::submenu)
        {
            return self.sub_key(key, sub, sub_items, menus);
        }
        let highlighted = self.item.and_then(|item| items.get(item));
        match key {
            // An item with a submenu opens it, with its first item highlighted.
            "right" | "enter" | "space" if self.open && highlighted.and_then(Item::submenu).is_some() => {
                let sub_items = highlighted.and_then(Item::submenu).unwrap_or_default();
                self.sub = Some(Sub { item: next_choice(sub_items, None, true) });
            }
            "left" | "right" => {
                let next = if key == "right" { (menu + 1) % count } else { (menu + count - 1) % count };
                *self = if self.open { Nav::open(next, &menus[next], true) } else { Nav { menu: Some(next), ..*self } };
            }
            "down" if !self.open => *self = Nav::open(menu, items, true),
            "up" if !self.open => *self = Nav { open: true, item: next_choice(items, None, false), ..*self },
            "down" | "up" => self.item = next_choice(items, self.item, key == "down"),
            "enter" | "space" if !self.open => *self = Nav::open(menu, items, true),
            "enter" | "space" => return self.choose(items),
            "escape" if self.open => *self = Nav { open: false, item: None, sub: None, ..*self },
            "escape" => {
                *self = Nav::default();
                return Outcome::Leave;
            }
            _ => {
                let Some(letter) = key.chars().next().filter(|_| key.chars().count() == 1) else {
                    return Outcome::Stay;
                };
                let letter = letter.to_lowercase().next().unwrap_or(letter);
                if !self.open {
                    if let Some(found) = titles.iter().position(|title| *title == Some(letter)) {
                        *self = Nav::open(found, &menus[found], true);
                    }
                } else if let Some(found) =
                    items.iter().position(|item| item.is_enabled() && item.mnemonic() == Some(letter))
                {
                    self.item = Some(found);
                    if let Some(sub_items) = items[found].submenu() {
                        self.sub = Some(Sub { item: next_choice(sub_items, None, true) });
                        return Outcome::Stay;
                    }
                    return self.choose(items);
                }
            }
        }
        Outcome::Stay
    }

    /// A key while the submenu `sub` of the highlighted item is open, of `items`.
    fn sub_key(&mut self, key: &str, sub: Sub, items: &[Item], menus: &[Vec<Item>]) -> Outcome {
        let (Some(menu), Some(item)) = (self.menu, self.item) else { return Outcome::Stay };
        match key {
            "down" | "up" => self.sub = Some(Sub { item: next_choice(items, sub.item, key == "down") }),
            "left" | "escape" => self.sub = None,
            // On to the next menu, as Windows goes from a submenu.
            "right" => {
                let next = (menu + 1) % menus.len();
                *self = Nav::open(next, &menus[next], true);
            }
            "enter" | "space" => return self.choose_sub(menu, item, sub.item, items),
            _ => {
                let Some(letter) = key.chars().next().filter(|_| key.chars().count() == 1) else {
                    return Outcome::Stay;
                };
                let letter = letter.to_lowercase().next().unwrap_or(letter);
                let found = items.iter().position(|item| item.is_enabled() && item.mnemonic() == Some(letter));
                return self.choose_sub(menu, item, found, items);
            }
        }
        Outcome::Stay
    }

    fn choose_sub(&mut self, menu: usize, item: usize, sub: Option<usize>, items: &[Item]) -> Outcome {
        match sub {
            Some(sub) if matches!(items.get(sub), Some(Item::Choice { enabled: true, .. })) => {
                *self = Nav::default();
                Outcome::Choose(menu, item, Some(sub))
            }
            _ => Outcome::Stay,
        }
    }

    fn choose(&mut self, items: &[Item]) -> Outcome {
        match (self.menu, self.item) {
            (Some(menu), Some(item)) if matches!(items.get(item), Some(Item::Choice { enabled: true, .. })) => {
                *self = Nav::default();
                Outcome::Choose(menu, item, None)
            }
            _ => Outcome::Stay,
        }
    }
}

/// The next item that can be highlighted after `from`, or before it, round the end; the first or
/// the last for none.
fn next_choice(items: &[Item], from: Option<usize>, forward: bool) -> Option<usize> {
    let count = items.len();
    let choices = |i: &usize| matches!(items[*i], Item::Choice { .. } | Item::Submenu { .. });
    if count == 0 {
        return None;
    }
    let order: Vec<usize> = match (from, forward) {
        (None, true) => (0..count).collect(),
        (None, false) => (0..count).rev().collect(),
        (Some(from), true) => (1..=count).map(|step| (from + step) % count).collect(),
        (Some(from), false) => (1..=count).map(|step| (from + count - step) % count).collect(),
    };
    order.into_iter().find(choices)
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
    /// Where the bar is in the window, for clicks on its titles while a menu is open.
    bounds: Rc<Cell<Bounds<Pixels>>>,
    /// Where an open submenu is: a press there is not outside the menus.
    sub_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    _activation: Subscription,
}

/// The menu a panel of items shows: a menu of the bar, or the submenu of its item.
#[derive(Clone, Copy)]
enum Level {
    Menu(usize),
    Sub(usize, usize),
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
            sub_bounds: Rc::default(),
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

    /// Chooses the item `item` of the menu `menu`, or the item `sub` of its submenu: the menus
    /// close, the focus goes back, and the action goes to where the focus is.
    fn choose(&mut self, menu: usize, item: usize, sub: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let menus = (self.menus)(self.previous_focus.as_ref(), window, cx);
        let item = menus.get(menu).and_then(|menu| menu.items.get(item));
        let item = match (item, sub) {
            (Some(ItemSpec::Submenu { items, enabled: true, .. }), Some(sub)) => items.get(sub),
            (item, None) => item,
            _ => None,
        };
        let action = match item {
            Some(ItemSpec::Action { action, enabled: true, .. }) => Some(action.boxed_clone()),
            _ => None,
        };
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
        let named = ["left", "right", "up", "down", "enter", "space", "escape"].contains(&keystroke.key.as_str());
        let key = if named {
            keystroke.key.clone()
        } else {
            // A letter of the layout typed, or the key under it.
            let typed = typed_letters(keystroke);
            let open = self.nav.menu.filter(|_| self.nav.open).map(|menu| items[menu].as_slice());
            let sub = open.zip(self.nav.item).and_then(|(open, item)| open.get(item)).and_then(Item::submenu);
            let wanted: Vec<Option<char>> = match (open, sub.filter(|_| self.nav.sub.is_some())) {
                (_, Some(sub)) => sub.iter().map(Item::mnemonic).collect(),
                (Some(open), None) => open.iter().map(Item::mnemonic).collect(),
                (None, None) => titles.clone(),
            };
            match typed.into_iter().find(|letter| wanted.contains(&Some(*letter))) {
                Some(letter) => letter.to_string(),
                None => {
                    cx.stop_propagation();
                    return;
                }
            }
        };
        self.keyboard = true;
        match self.nav.key(&key, &items, &titles) {
            Outcome::Stay => cx.notify(),
            Outcome::Leave => self.leave(window, cx),
            Outcome::Choose(menu, item, sub) => self.choose(menu, item, sub, window, cx),
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

    fn show_mnemonics(&self) -> bool {
        self.keyboard || self.alt_held
    }

    /// A label with its mnemonic underlined while they show.
    fn label(&self, text: &SharedString, mnemonic: Option<usize>) -> StyledText {
        let underline = mnemonic_range(text, mnemonic).filter(|_| self.show_mnemonics());
        let style = HighlightStyle {
            underline: Some(UnderlineStyle { thickness: px(1.), color: None, wavy: false }),
            ..Default::default()
        };
        StyledText::new(text.clone()).with_highlights(underline.map(|range| (range, style)))
    }

    fn render_menu(&self, menu: usize, spec: MenuSpec, cx: &mut Context<Self>) -> AnyElement {
        let panel = self.render_panel(spec.title, spec.items, Level::Menu(menu), cx).on_mouse_down_out(cx.listener(
            // A press outside the menus closes them; one on a title is for the title to handle.
            |bar, event: &MouseDownEvent, window, cx| {
                let on_sub = bar.sub_bounds.get().is_some_and(|sub| sub.contains(&event.position));
                if !bar.bounds.get().contains(&event.position) && !on_sub {
                    bar.leave(window, cx);
                }
            },
        ));
        // Under its title, over everything else.
        let menu = deferred(anchored().snap_to_window().child(panel)).with_priority(1);
        div().absolute().top(px(HEIGHT)).left_0().child(menu).into_any_element()
    }

    /// The panel of the items of a menu, or of a submenu.
    fn render_panel(
        &self,
        title: SharedString,
        items: Vec<ItemSpec>,
        level: Level,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let theme = theme(cx);
        let shadow = BoxShadow {
            color: rgba(theme.shadow).into(),
            offset: point(px(0.), px(3.)),
            blur_radius: px(10.),
            spread_radius: px(0.),
            inset: false,
        };
        let highlighted = match level {
            Level::Menu(_) => self.nav.item,
            Level::Sub(..) => self.nav.sub.and_then(|sub| sub.item),
        };
        let items = items.into_iter().enumerate().map(|(i, item)| {
            let (label, mnemonic, enabled, keys, checked, submenu) = match item {
                ItemSpec::Separator => {
                    return div().h(px(1.)).mx(px(8.)).my(px(4.)).bg(rgb(theme.border)).into_any_element();
                }
                ItemSpec::Action { label, mnemonic, keys, checked, enabled, .. } => {
                    (label, mnemonic, enabled, keys, checked, None)
                }
                ItemSpec::Submenu { label, mnemonic, enabled, items } => {
                    (label, mnemonic, enabled, None, None, Some(items))
                }
            };
            let lit = highlighted == Some(i);
            let expanded = lit && submenu.is_some() && self.nav.sub.is_some();
            let text = if enabled { theme.text } else { theme.text_disabled };
            let keys_color = if lit && enabled { theme.text } else { theme.text_muted };
            let has_submenu = submenu.is_some();
            let role = if checked.is_some() { Role::MenuItemCheckBox } else { Role::MenuItem };
            let sub_panel = submenu.filter(|_| expanded).map(|sub_items| {
                let Level::Menu(menu) = level else { unreachable!("submenus have no submenus") };
                let sub_bounds = self.sub_bounds.clone();
                let panel = self.render_panel(label.clone(), sub_items, Level::Sub(menu, i), cx).child(
                    canvas(move |area, _, _| sub_bounds.set(Some(area)), |_, _, _, _| {}).absolute().size_full(),
                );
                // To the right of its item, over its menu.
                let panel = deferred(anchored().snap_to_window().child(panel)).with_priority(2);
                div().absolute().top(px(-5.)).left_full().child(panel)
            });
            div()
                .id(("item", i))
                .role(role)
                .aria_label(label.clone())
                .when_some(checked, |item, on| item.aria_toggled(if on { Toggled::True } else { Toggled::False }))
                .when(has_submenu, |item| item.aria_expanded(expanded))
                // Screen readers follow the highlighted item: the focus stays on the menus.
                .when(lit, |item| item.aria_active_descendant())
                .relative()
                .flex()
                .items_center()
                .h(px(24.))
                .mx(px(4.))
                .pr(px(10.))
                .rounded(px(4.))
                .text_color(rgb(text))
                .when(lit, |item| item.bg(rgb(theme.menu_selected)))
                .on_mouse_move(cx.listener(move |bar, _, _, cx| {
                    let nav = match level {
                        // The pointer over an item with a submenu opens it, and keeps it open as it
                        // moves over the item.
                        Level::Menu(_) if has_submenu && enabled => {
                            let sub = bar.nav.sub.filter(|_| bar.nav.item == Some(i)).unwrap_or_default();
                            Nav { item: Some(i), sub: Some(sub), ..bar.nav }
                        }
                        Level::Menu(_) => Nav { item: Some(i), sub: None, ..bar.nav },
                        Level::Sub(_, item) => Nav { item: Some(item), sub: Some(Sub { item: Some(i) }), ..bar.nav },
                    };
                    if bar.nav != nav {
                        bar.nav = nav;
                        cx.notify();
                    }
                }))
                .on_click(cx.listener(move |bar, _, window, cx| match level {
                    Level::Menu(menu) if !has_submenu && enabled => bar.choose(menu, i, None, window, cx),
                    Level::Sub(menu, item) if enabled => bar.choose(menu, item, Some(i), window, cx),
                    _ => {}
                }))
                .child(div().flex_none().w(px(24.)).flex().justify_center().child(if checked == Some(true) {
                    "✓"
                } else {
                    ""
                }))
                .child(div().flex_1().whitespace_nowrap().child(self.label(&label, mnemonic)))
                .children(keys.map(|keys| div().flex_none().pl(px(28.)).text_color(rgb(keys_color)).child(keys)))
                .when(has_submenu, |item| {
                    item.child(div().flex_none().pl(px(28.)).text_color(rgb(keys_color)).child("▸"))
                })
                .children(sub_panel)
                .into_any_element()
        });
        let id = match level {
            Level::Menu(_) => SharedString::from("menu"),
            Level::Sub(..) => SharedString::from("submenu"),
        };
        div()
            .id(id)
            .role(Role::Menu)
            .aria_label(title)
            .min_w(px(220.))
            .py(px(4.))
            .bg(rgb(theme.menu))
            .border_1()
            .border_color(rgb(theme.border))
            .rounded(px(6.))
            .shadow(vec![shadow])
            .occlude()
            .text_size(crate::text_size())
            .text_color(rgb(theme.text))
            .children(items)
    }
}

/// The letters a keystroke could mean, in lower case: the one typed in the layout, and the key
/// under it, for a mnemonic in another script than the layout.
fn typed_letters(keystroke: &Keystroke) -> Vec<char> {
    let mut letters: Vec<char> = Vec::new();
    for text in [keystroke.key_char.as_deref(), Some(keystroke.key.as_str())].into_iter().flatten() {
        let mut chars = text.chars();
        if let (Some(c), None) = (chars.next(), chars.next()) {
            letters.push(c.to_lowercase().next().unwrap_or(c));
        }
    }
    letters
}

impl Render for MenuBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme(cx);
        let menus = (self.menus)(self.previous_focus.as_ref(), window, cx);
        // An open submenu tells where it is as it is drawn.
        self.sub_bounds.set(None);
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
            let menu = (open == Some(index)).then(|| self.render_menu(index, spec, cx));
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
                    .child(self.label(&title, mnemonic))
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
    fn arrows_move_over_titles_and_items_skipping_separators() {
        let (menus, titles) = menus();
        let mut nav = Nav::select_bar();
        assert_eq!(nav.key("left", &menus, &titles), Outcome::Stay);
        assert_eq!(nav, Nav { menu: Some(2), open: false, item: None, sub: None });
        nav.key("right", &menus, &titles);
        nav.key("down", &menus, &titles);
        assert_eq!(nav, Nav { menu: Some(0), open: true, item: Some(0), sub: None });
        nav.key("down", &menus, &titles);
        assert_eq!(nav.item, Some(2), "past the separator");
        nav.key("down", &menus, &titles);
        assert_eq!(nav.item, Some(0), "round the end");
        nav.key("up", &menus, &titles);
        assert_eq!(nav.item, Some(2));
        // With a menu open, the next one opens.
        nav.key("right", &menus, &titles);
        assert_eq!(nav, Nav { menu: Some(1), open: true, item: Some(0), sub: None });
    }

    #[test]
    fn enter_chooses_an_enabled_item_and_escape_goes_back() {
        let (menus, titles) = menus();
        let mut nav = Nav::open(1, &menus[1], true);
        assert_eq!(nav.key("enter", &menus, &titles), Outcome::Stay, "a disabled item");
        nav.key("down", &menus, &titles);
        assert_eq!(nav.key("enter", &menus, &titles), Outcome::Choose(1, 1, None));
        assert!(!nav.is_active());

        let mut nav = Nav::open(0, &menus[0], true);
        assert_eq!(nav.key("escape", &menus, &titles), Outcome::Stay);
        assert_eq!(nav, Nav { menu: Some(0), open: false, item: None, sub: None });
        assert_eq!(nav.key("escape", &menus, &titles), Outcome::Leave);
        assert!(!nav.is_active());
    }

    #[test]
    fn mnemonics_open_menus_and_choose_items() {
        let (menus, titles) = menus();
        let mut nav = Nav::select_bar();
        assert_eq!(nav.key("ф", &menus, &titles), Outcome::Stay);
        assert_eq!(nav, Nav { menu: Some(2), open: true, item: Some(0), sub: None });
        let mut nav = Nav::select_bar();
        nav.key("E", &menus, &titles);
        assert_eq!(nav.menu, Some(1));
        assert_eq!(nav.key("o", &menus, &titles), Outcome::Stay, "a disabled item is not chosen");
        assert_eq!(nav.key("a", &menus, &titles), Outcome::Choose(1, 1, None));
    }

    #[test]
    fn submenus_open_to_the_right_and_close_to_the_left() {
        let recent = Item::Submenu { enabled: true, mnemonic: Some('r'), items: vec![A, SEP, B] };
        let empty = Item::Submenu { enabled: false, mnemonic: Some('e'), items: Vec::new() };
        let menus = vec![vec![A, recent, empty], vec![B]];
        let titles = vec![Some('f'), Some('v')];
        let mut nav = Nav::open(0, &menus[0], true);
        nav.key("down", &menus, &titles);
        assert_eq!((nav.item, nav.sub), (Some(1), None));
        // Right opens the submenu with its first item highlighted; down skips its separator.
        nav.key("right", &menus, &titles);
        assert_eq!(nav.sub, Some(Sub { item: Some(0) }));
        nav.key("down", &menus, &titles);
        assert_eq!(nav.sub, Some(Sub { item: Some(2) }));
        assert_eq!(nav.key("enter", &menus, &titles), Outcome::Choose(0, 1, Some(2)));
        assert!(!nav.is_active());
        // Left and escape close it; right goes on to the next menu.
        let mut nav = Nav::open(0, &menus[0], true);
        nav.key("r", &menus, &titles);
        assert_eq!((nav.item, nav.sub), (Some(1), Some(Sub { item: Some(0) })), "its mnemonic opens it");
        nav.key("left", &menus, &titles);
        assert_eq!((nav.menu, nav.item, nav.sub), (Some(0), Some(1), None));
        nav.key("enter", &menus, &titles);
        assert!(nav.sub.is_some(), "enter opens it too");
        assert_eq!(nav.key("b", &menus, &titles), Outcome::Choose(0, 1, Some(2)), "a mnemonic in it chooses");
        let mut nav = Nav { sub: Some(Sub::default()), item: Some(1), ..Nav::open(0, &menus[0], true) };
        nav.key("right", &menus, &titles);
        assert_eq!((nav.menu, nav.sub), (Some(1), None));
        // A disabled submenu does not open.
        let mut nav = Nav { item: Some(2), ..Nav::open(0, &menus[0], true) };
        nav.key("right", &menus, &titles);
        assert_eq!((nav.menu, nav.sub), (Some(1), None), "on to the next menu");
    }

    #[test]
    fn mnemonics_are_the_letters_marked() {
        assert_eq!(mnemonic_char("Вы&резать".replace('&', "").as_str(), Some(4)), Some('р'));
        assert_eq!(mnemonic_char("File", Some(0)), Some('f'));
        assert_eq!(mnemonic_char("File", None), None);
        assert_eq!(mnemonic_range("Правка", Some(0)), Some(0..2));
    }
}
