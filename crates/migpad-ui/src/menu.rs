//! What the menus MigPad draws have in common — the menu bar of Windows and Linux and the context
//! menus: the items, the keyboard over them, and the panels they show in. An item may open a
//! submenu to its right, and an item of that another one, however deep: the pointer over the item
//! opens it, so does a click, the right arrow and Enter; the left arrow and Esc close it.

use gpui::{
    Action, AnyElement, BoxShadow, Context, HighlightStyle, Keystroke, Role, SharedString, Stateful, StyledText,
    Toggled, UnderlineStyle, anchored, deferred, div, point, prelude::*, px, rgb, rgba,
};

use crate::theme::theme;

/// An item of a menu, as the application describes it each time the menu is drawn.
pub enum ItemSpec {
    Action {
        label: SharedString,
        /// Where the mnemonic of the label is, in bytes.
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

impl Clone for ItemSpec {
    fn clone(&self) -> Self {
        match self {
            ItemSpec::Action { label, mnemonic, keys, checked, enabled, action } => ItemSpec::Action {
                label: label.clone(),
                mnemonic: *mnemonic,
                keys: keys.clone(),
                checked: *checked,
                enabled: *enabled,
                action: action.boxed_clone(),
            },
            ItemSpec::Submenu { label, mnemonic, enabled, items } => {
                ItemSpec::Submenu { label: label.clone(), mnemonic: *mnemonic, enabled: *enabled, items: items.clone() }
            }
            ItemSpec::Separator => ItemSpec::Separator,
        }
    }
}

impl ItemSpec {
    pub(crate) fn kind(&self) -> Item {
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

/// The item at `path` of `items`: the item `path[0]` of the menu, or of a submenu down the path.
pub(crate) fn item_at<'a>(items: &'a [ItemSpec], path: &[usize]) -> Option<&'a ItemSpec> {
    let (&first, rest) = path.split_first()?;
    let item = items.get(first)?;
    match (item, rest.is_empty()) {
        (_, true) => Some(item),
        (ItemSpec::Submenu { items, enabled: true, .. }, false) => item_at(items, rest),
        _ => None,
    }
}

/// The action of the enabled item at `path` of `items`, if it is one that acts.
pub fn action_at(items: &[ItemSpec], path: &[usize]) -> Option<Box<dyn Action>> {
    match item_at(items, path)? {
        ItemSpec::Action { action, enabled: true, .. } => Some(action.boxed_clone()),
        _ => None,
    }
}

/// The letter at `mnemonic` of `text`, in lower case.
pub(crate) fn mnemonic_char(text: &str, mnemonic: Option<usize>) -> Option<char> {
    text.get(mnemonic?..)?.chars().next().map(|c| c.to_lowercase().next().unwrap_or(c))
}

/// The bytes of the letter at `mnemonic` of `text`.
fn mnemonic_range(text: &str, mnemonic: Option<usize>) -> Option<std::ops::Range<usize>> {
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
    pub(crate) fn mnemonic(&self) -> Option<char> {
        match self {
            Item::Choice { mnemonic, .. } | Item::Submenu { mnemonic, .. } => *mnemonic,
            Item::Separator => None,
        }
    }

    fn is_enabled(&self) -> bool {
        matches!(self, Item::Choice { enabled: true, .. } | Item::Submenu { enabled: true, .. })
    }

    /// The items of its menu, for an enabled item that opens one.
    pub(crate) fn submenu(&self) -> Option<&[Item]> {
        match self {
            Item::Submenu { enabled: true, items, .. } => Some(items),
            _ => None,
        }
    }
}

/// The next item that can be highlighted after `from`, or before it, round the end; the first or
/// the last for none.
pub(crate) fn next_choice(items: &[Item], from: Option<usize>, forward: bool) -> Option<usize> {
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

/// The letters a keystroke could mean, in lower case: the one typed in the layout, and the key
/// under it, for a mnemonic in another script than the layout.
pub(crate) fn typed_letters(keystroke: &Keystroke) -> Vec<char> {
    let mut letters: Vec<char> = Vec::new();
    for text in [keystroke.key_char.as_deref(), Some(keystroke.key.as_str())].into_iter().flatten() {
        let mut chars = text.chars();
        if let (Some(c), None) = (chars.next(), chars.next()) {
            letters.push(c.to_lowercase().next().unwrap_or(c));
        }
    }
    letters
}

/// The key a menu takes from `keystroke`, if any: an arrow, Enter, Space, Esc, or a letter that is
/// the mnemonic of an item of `items` — of the letters the keystroke could mean.
pub(crate) fn menu_key(keystroke: &Keystroke, items: &[Item]) -> Option<String> {
    const NAMED: [&str; 7] = ["left", "right", "up", "down", "enter", "space", "escape"];
    if NAMED.contains(&keystroke.key.as_str()) {
        return Some(keystroke.key.clone());
    }
    let wanted: Vec<Option<char>> = items.iter().map(Item::mnemonic).collect();
    typed_letters(keystroke).into_iter().find(|letter| wanted.contains(&Some(*letter))).map(String::from)
}

/// What a key did in an open menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Nothing outside the menu.
    Stay,
    /// The item at this path is chosen.
    Choose(Vec<usize>),
    /// The left arrow in the menu itself: the menu bar goes to the menu before.
    Left,
    /// The right arrow on an item without a submenu: the menu bar goes to the next menu.
    Right,
    /// Esc in the menu itself: it closes.
    Escape,
}

/// Where the keyboard and the pointer are in an open menu: the item highlighted in the menu and in
/// each submenu open from it, the menu first. A submenu is open from the highlighted item of every
/// level but the last.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Levels(Vec<Option<usize>>);

impl Levels {
    /// A menu just open of `items`: the first item highlighted when the keyboard opened it.
    pub fn open(items: &[Item], keyboard: bool) -> Levels {
        Levels(vec![if keyboard { next_choice(items, None, true) } else { None }])
    }

    /// The menu open, nothing highlighted: the pointer opened it.
    pub fn new() -> Levels {
        Levels(vec![None])
    }

    /// The highlighted item at `depth`: of the menu at 0, of the submenu open from it at 1…
    pub fn highlighted(&self, depth: usize) -> Option<usize> {
        self.0.get(depth).copied().flatten()
    }

    /// Whether the submenu of the item `item` at `depth` is open.
    pub fn is_open(&self, depth: usize, item: usize) -> bool {
        self.0.len() > depth + 1 && self.0[depth] == Some(item)
    }

    /// The path of the item `item` at `depth`: the items it is reached through, and it.
    fn path(&self, depth: usize, item: usize) -> Vec<usize> {
        let mut path: Vec<usize> = self.0[..depth].iter().map(|item| item.unwrap_or(0)).collect();
        path.push(item);
        path
    }

    /// The pointer is over the item `item` at `depth`: it is highlighted, and its submenu opens if
    /// it `opens` one, the submenus of other items close. Returns whether anything changed.
    pub fn hover(&mut self, depth: usize, item: usize, opens: bool) -> bool {
        if depth >= self.0.len() || self.is_open(depth, item) {
            return false;
        }
        let before = self.clone();
        self.0.truncate(depth + 1);
        self.0[depth] = Some(item);
        if opens {
            self.0.push(None);
        }
        *self != before
    }

    /// The items at `depth` of the menu of `items`, as far as the submenus open down to it.
    fn items_at<'a>(&self, items: &'a [Item], depth: usize) -> Option<&'a [Item]> {
        let mut current = items;
        for level in &self.0[..depth] {
            current = level.and_then(|item| current.get(item)).and_then(Item::submenu)?;
        }
        Some(current)
    }

    /// The items of the deepest open level: the keys of the menu go to them.
    pub fn current<'a>(&self, items: &'a [Item]) -> &'a [Item] {
        self.items_at(items, self.0.len() - 1).unwrap_or_default()
    }

    /// A key — see [`menu_key`] — in the open menu of `items`.
    pub fn key(&mut self, key: &str, items: &[Item]) -> Step {
        let depth = self.0.len() - 1;
        let Some(current) = self.items_at(items, depth) else { return Step::Stay };
        let highlighted = self.0[depth];
        let submenu = highlighted.and_then(|item| current.get(item)).and_then(Item::submenu);
        match (key, submenu) {
            ("down" | "up", _) => self.0[depth] = next_choice(current, highlighted, key == "down"),
            ("right" | "enter" | "space", Some(submenu)) => self.0.push(next_choice(submenu, None, true)),
            ("right", None) => return Step::Right,
            ("left" | "escape", _) if depth > 0 => {
                self.0.pop();
            }
            ("left", _) => return Step::Left,
            ("escape", _) => return Step::Escape,
            ("enter" | "space", None) => return self.choose(current),
            (letter, _) => {
                let mut chars = letter.chars();
                let (Some(letter), None) = (chars.next(), chars.next()) else { return Step::Stay };
                let letter = letter.to_lowercase().next().unwrap_or(letter);
                let Some(found) = current.iter().position(|item| item.is_enabled() && item.mnemonic() == Some(letter))
                else {
                    return Step::Stay;
                };
                self.0[depth] = Some(found);
                match current[found].submenu() {
                    Some(submenu) => self.0.push(next_choice(submenu, None, true)),
                    None => return self.choose(current),
                }
            }
        }
        Step::Stay
    }

    /// Enter on the highlighted item of the deepest level, of `current`.
    fn choose(&self, current: &[Item]) -> Step {
        let depth = self.0.len() - 1;
        match self.0[depth] {
            Some(item) if matches!(current.get(item), Some(Item::Choice { enabled: true, .. })) => {
                Step::Choose(self.path(depth, item))
            }
            _ => Step::Stay,
        }
    }
}

/// What draws a menu: the menu bar of a window, or a context menu.
pub(crate) trait MenuHost: Sized + 'static {
    fn levels(&self) -> &Levels;
    fn levels_mut(&mut self) -> &mut Levels;
    /// Whether the mnemonics are underlined: while the keyboard drives the menu, or Alt is held.
    fn shows_mnemonics(&self) -> bool;
    /// The item at `path` of the open menu is chosen.
    fn choose(&mut self, path: Vec<usize>, window: &mut gpui::Window, cx: &mut Context<Self>);
}

/// A label with its mnemonic underlined while they show.
pub(crate) fn label(text: &SharedString, mnemonic: Option<usize>, shown: bool) -> StyledText {
    let underline = mnemonic_range(text, mnemonic).filter(|_| shown);
    let style = HighlightStyle {
        underline: Some(UnderlineStyle { thickness: px(1.), color: None, wavy: false }),
        ..Default::default()
    };
    StyledText::new(text.clone()).with_highlights(underline.map(|range| (range, style)))
}

/// The panel of `items`, the menu at `depth` of the host — 0 for the menu itself — with the
/// submenu open from its highlighted item to its right, over it: deferred with `priority` and more.
pub(crate) fn panel<H: MenuHost>(
    host: &H,
    title: SharedString,
    items: Vec<ItemSpec>,
    depth: usize,
    priority: usize,
    cx: &mut Context<H>,
) -> Stateful<gpui::Div> {
    let theme = theme(cx);
    let shadow = BoxShadow {
        color: rgba(theme.shadow).into(),
        offset: point(px(0.), px(3.)),
        blur_radius: px(10.),
        spread_radius: px(0.),
        inset: false,
    };
    let levels = host.levels();
    let highlighted = levels.highlighted(depth);
    let mnemonics = host.shows_mnemonics();
    let items: Vec<AnyElement> = items
        .into_iter()
        .enumerate()
        .map(|(i, item)| {
            let (label_text, mnemonic, enabled, keys, checked, submenu) = match item {
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
            let has_submenu = submenu.is_some();
            let expanded = has_submenu && levels.is_open(depth, i);
            let text = if enabled { theme.text } else { theme.text_disabled };
            let keys_color = if lit && enabled { theme.text } else { theme.text_muted };
            let role = if checked.is_some() { Role::MenuItemCheckBox } else { Role::MenuItem };
            let sub_panel = submenu.filter(|_| expanded).map(|sub_items| {
                let panel = panel(host, label_text.clone(), sub_items, depth + 1, priority + 1, cx);
                // To the right of its item, over its menu.
                let panel = deferred(anchored().snap_to_window().child(panel)).with_priority(priority + 1);
                div().absolute().top(px(-5.)).left_full().child(panel)
            });
            let path = levels.path(depth, i);
            let opens = has_submenu && enabled;
            div()
                .id(("item", i))
                .role(role)
                .aria_label(label_text.clone())
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
                .on_mouse_move(cx.listener(move |host, _, _, cx| {
                    if host.levels_mut().hover(depth, i, opens) {
                        cx.notify();
                    }
                }))
                .on_click(cx.listener(move |host, _, window, cx| {
                    if opens {
                        // A click opens a submenu too, as the menus of Windows do.
                        host.levels_mut().hover(depth, i, true);
                        cx.notify();
                    } else if enabled && !has_submenu {
                        host.choose(path.clone(), window, cx);
                    }
                }))
                .child(div().flex_none().w(px(24.)).flex().justify_center().child(if checked == Some(true) {
                    "✓"
                } else {
                    ""
                }))
                .child(div().flex_1().whitespace_nowrap().child(label(&label_text, mnemonic, mnemonics)))
                .children(keys.map(|keys| div().flex_none().pl(px(28.)).text_color(rgb(keys_color)).child(keys)))
                .when(has_submenu, |item| {
                    item.child(div().flex_none().pl(px(28.)).text_color(rgb(keys_color)).child("▸"))
                })
                .children(sub_panel)
                .into_any_element()
        })
        .collect();
    div()
        .id(SharedString::from(format!("menu-{depth}")))
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

#[cfg(test)]
mod tests {
    use super::*;

    const A: Item = Item::Choice { enabled: true, mnemonic: Some('a') };
    const B: Item = Item::Choice { enabled: true, mnemonic: Some('b') };
    const OFF: Item = Item::Choice { enabled: false, mnemonic: Some('o') };
    const SEP: Item = Item::Separator;

    fn sub(mnemonic: char, items: Vec<Item>) -> Item {
        Item::Submenu { enabled: true, mnemonic: Some(mnemonic), items }
    }

    #[test]
    fn arrows_move_over_items_skipping_separators() {
        let items = [A, SEP, B, OFF];
        let mut levels = Levels::open(&items, true);
        assert_eq!(levels.highlighted(0), Some(0));
        levels.key("down", &items);
        assert_eq!(levels.highlighted(0), Some(2), "past the separator");
        levels.key("down", &items);
        assert_eq!(levels.highlighted(0), Some(3), "a disabled item is highlighted, not chosen");
        assert_eq!(levels.key("enter", &items), Step::Stay);
        levels.key("down", &items);
        assert_eq!(levels.highlighted(0), Some(0), "round the end");
        levels.key("up", &items);
        assert_eq!(levels.highlighted(0), Some(3));
        assert_eq!(Levels::new().highlighted(0), None);
    }

    #[test]
    fn enter_and_mnemonics_choose_and_tell_the_path() {
        let items = [A, SEP, B];
        let mut levels = Levels::open(&items, true);
        assert_eq!(levels.key("enter", &items), Step::Choose(vec![0]));
        assert_eq!(levels.key("b", &items), Step::Choose(vec![2]));
        assert_eq!(levels.key("o", &items), Step::Stay, "no such mnemonic");
        assert_eq!(levels.key("escape", &items), Step::Escape);
        assert_eq!(levels.key("left", &items), Step::Left);
        assert_eq!(levels.key("right", &items), Step::Right);
    }

    #[test]
    fn submenus_open_and_close_at_any_depth() {
        let inner = sub('i', vec![A, B]);
        let middle = sub('m', vec![SEP, inner, OFF]);
        let off = Item::Submenu { enabled: false, mnemonic: Some('x'), items: vec![A] };
        let items = [A, middle, off];
        let mut levels = Levels::open(&items, true);
        levels.key("down", &items);
        // Right opens the submenu, its first item highlighted, past the separator.
        assert_eq!(levels.key("right", &items), Step::Stay);
        assert_eq!((levels.highlighted(0), levels.highlighted(1)), (Some(1), Some(1)));
        assert!(levels.is_open(0, 1));
        // Enter opens the next one; a mnemonic there chooses: the path goes through all three.
        levels.key("enter", &items);
        assert!(levels.is_open(1, 1));
        assert_eq!(levels.key("b", &items), Step::Choose(vec![1, 1, 1]));
        // Left and Esc close the deepest one.
        levels.key("left", &items);
        assert!(!levels.is_open(1, 1) && levels.is_open(0, 1));
        levels.key("escape", &items);
        assert!(!levels.is_open(0, 1));
        assert_eq!(levels.highlighted(0), Some(1));
        // A mnemonic opens a submenu; right on an item without one goes on to the next menu.
        assert_eq!(levels.key("m", &items), Step::Stay);
        assert!(levels.is_open(0, 1));
        levels.key("down", &items);
        assert_eq!(levels.key("right", &items), Step::Right, "a disabled item opens nothing");
        // A disabled submenu does not open.
        let mut levels = Levels::open(&items, true);
        levels.key("up", &items);
        assert_eq!(levels.highlighted(0), Some(2));
        assert_eq!(levels.key("right", &items), Step::Right);
    }

    #[test]
    fn the_pointer_opens_submenus_and_closes_the_others() {
        let items = [A, sub('m', vec![sub('i', vec![A]), B]), sub('n', vec![A])];
        let mut levels = Levels::new();
        assert!(levels.hover(0, 1, true));
        assert!(levels.is_open(0, 1));
        assert!(!levels.hover(0, 1, true), "already open: what is open in it stays");
        assert!(levels.hover(1, 0, true));
        assert!(levels.is_open(1, 0));
        // Another item of the menu closes both.
        assert!(levels.hover(0, 2, true));
        assert!(!levels.is_open(1, 0) && levels.is_open(0, 2));
        assert!(levels.hover(0, 0, false));
        assert_eq!(levels.current(&items), &items[..]);
        assert!(!levels.hover(3, 0, false), "a depth not open");
    }

    #[test]
    fn items_are_found_by_their_path() {
        let action = || Box::new(gpui::NoAction) as Box<dyn Action>;
        let item = |label: &'static str, enabled| ItemSpec::Action {
            label: label.into(),
            mnemonic: None,
            keys: None,
            checked: None,
            enabled,
            action: action(),
        };
        let items = vec![
            item("a", true),
            ItemSpec::Submenu {
                label: "m".into(),
                mnemonic: None,
                enabled: true,
                items: vec![item("b", true), item("c", false)],
            },
        ];
        assert!(action_at(&items, &[0]).is_some());
        assert!(action_at(&items, &[1, 0]).is_some());
        assert!(action_at(&items, &[1, 1]).is_none(), "disabled");
        assert!(action_at(&items, &[1]).is_none(), "a submenu does not act");
        assert!(action_at(&items, &[2]).is_none());
        assert!(action_at(&items, &[]).is_none());
    }

    #[test]
    fn mnemonics_are_the_letters_marked() {
        assert_eq!(mnemonic_char("Вы&резать".replace('&', "").as_str(), Some(4)), Some('р'));
        assert_eq!(mnemonic_char("File", Some(0)), Some('f'));
        assert_eq!(mnemonic_char("File", None), None);
        assert_eq!(mnemonic_range("Правка", Some(0)), Some(0..2));
    }
}
