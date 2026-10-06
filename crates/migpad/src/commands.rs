//! Commands: what the menus and the keys of the application do. Each feature is a module that
//! registers its commands, where they go in the menus and what handles them; the menu bar and
//! the key bindings are built from that registry.

use std::any::TypeId;
use std::collections::HashSet;
use std::rc::Rc;

use gpui::{
    Action, App, Context, Div, DummyKeyboardMapper, FocusHandle, Focusable, Global, InteractiveElement, KeyBinding,
    Menu, MenuItem, OsAction, SharedString, SystemMenuType, Window, WindowId,
};

use migpad_ui::Button;
use migpad_ui::menu_bar::{ItemSpec, MenuSpec};

use crate::keys;
use crate::strings::{Key, mnemonic, tr};
use crate::windows;
use crate::workspace::Workspace;

/// A feature of the application, which registers its part of the commands and the menus.
pub trait Module {
    /// The name of the module, which starts the identifiers of its commands: `view`.
    fn id(&self) -> &'static str;

    fn register(&self, registry: &mut Registry);
}

/// A command of the application.
pub struct Command {
    /// The module, then the command: `view.word_wrap`.
    pub id: &'static str,
    pub label: Key,
    pub action: Box<dyn Action>,
    /// The action of the system that the command is, so that its menu item works in the text
    /// fields of the system as well: copy, paste, undo.
    pub os_action: Option<OsAction>,
    /// The keys of the command on this system, see [`by_os`].
    pub keys: &'static [&'static str],
    /// Whether a toggle is on, for its check mark in the menus.
    pub checked: Option<fn(&Workspace, &App) -> bool>,
    /// Whether the command has something to do now, for the toolbar and the menus MigPad draws:
    /// undo without steps to undo is gray. Without it, a command is available when something
    /// handles its action.
    pub enabled: Option<fn(&Workspace, &App) -> bool>,
}

impl Command {
    pub fn new(id: &'static str, label: Key, action: impl Action) -> Self {
        Command { id, label, action: Box::new(action), os_action: None, keys: &[], checked: None, enabled: None }
    }

    pub fn keys(self, keys: &'static [&'static str]) -> Self {
        Command { keys, ..self }
    }

    pub fn os_action(self, os_action: OsAction) -> Self {
        Command { os_action: Some(os_action), ..self }
    }

    pub fn checked(self, checked: fn(&Workspace, &App) -> bool) -> Self {
        Command { checked: Some(checked), ..self }
    }

    pub fn enabled(self, enabled: fn(&Workspace, &App) -> bool) -> Self {
        Command { enabled: Some(enabled), ..self }
    }
}

/// The keys of a command on this system: they are fixed, and follow each system.
pub fn by_os(
    macos: &'static [&'static str],
    windows: &'static [&'static str],
    linux: &'static [&'static str],
) -> &'static [&'static str] {
    if cfg!(target_os = "macos") {
        macos
    } else if cfg!(target_os = "windows") {
        windows
    } else {
        linux
    }
}

/// The menus of the menu bar, in their order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuId {
    /// The menu of the application on macOS, named after it.
    App,
    File,
    Edit,
    View,
    Window,
}

impl MenuId {
    const ALL: [MenuId; 5] = [MenuId::App, MenuId::File, MenuId::Edit, MenuId::View, MenuId::Window];

    fn title(self) -> &'static str {
        self.key().map_or("MigPad", tr)
    }

    /// The key of the title; the menu of the application is named after it.
    fn key(self) -> Option<Key> {
        match self {
            MenuId::App => None,
            MenuId::File => Some(Key::FileMenu),
            MenuId::Edit => Some(Key::EditMenu),
            MenuId::View => Some(Key::ViewMenu),
            MenuId::Window => Some(Key::WindowMenu),
        }
    }
}

/// Whether windows have the menu bar MigPad draws: on Windows and Linux, where GPUI makes none.
/// A debug build shows it on macOS too with `MIGPAD_MENU_BAR=1`, to try it there.
pub fn own_menu_bar() -> bool {
    !cfg!(target_os = "macos") || (cfg!(debug_assertions) && std::env::var_os("MIGPAD_MENU_BAR").is_some())
}

/// What a menu item is.
enum Entry {
    Command(&'static str),
    /// The Services menu of macOS, which the system fills.
    Services,
    /// The open windows, one item each, to bring forward.
    Windows,
}

/// Brings the window forward: an item of the list of windows.
#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = window, no_json)]
pub struct ActivateWindow(pub WindowId);

/// An item in a group of a menu: groups go in the order of their numbers, separated by lines,
/// and items within a group in the order they were added.
struct Placement {
    menu: MenuId,
    group: u8,
    entry: Entry,
}

/// Adds the handler of an action to the root of each window.
type WindowHandler = Rc<dyn Fn(Div, &mut Context<Workspace>) -> Div>;
/// Installs the handler of an action in the application, once.
type AppHandler = Box<dyn FnOnce(&mut App)>;

/// The commands of the application and the menu items, as the modules registered them.
#[derive(Default)]
pub struct Registry {
    commands: Vec<Command>,
    placements: Vec<Placement>,
    /// The buttons of the toolbar: commands, by groups.
    toolbar: Vec<(u8, &'static str)>,
    /// The actions the application handles whatever window they come from.
    app_actions: HashSet<TypeId>,
    window_handlers: Vec<WindowHandler>,
    app_handlers: Vec<AppHandler>,
}

impl Global for Registry {}

impl Registry {
    /// Adds `command`, with a menu item in a group of a menu, if it has one there.
    pub fn add(&mut self, command: Command, menu: Option<(MenuId, u8)>) {
        if let Some((menu, group)) = menu {
            self.placements.push(Placement { menu, group, entry: Entry::Command(command.id) });
        }
        self.commands.push(command);
    }

    /// Adds the Services menu of macOS to a group of a menu.
    pub fn add_services(&mut self, menu: MenuId, group: u8) {
        self.placements.push(Placement { menu, group, entry: Entry::Services });
    }

    /// Adds a button for the command `id` to a group of the toolbar: groups go in the order of their
    /// numbers, buttons within a group in the order they were added.
    pub fn add_to_toolbar(&mut self, id: &'static str, group: u8) {
        self.toolbar.push((group, id));
    }

    fn command(&self, id: &str) -> &Command {
        self.commands.iter().find(|command| command.id == id).expect("a command of the registry")
    }

    /// Whether `command` can act in `window`, whose root is `workspace`: the application handles
    /// its action, or the window does where the focus is — or is to go back to, from the menus —
    /// and the command has something to do. Which actions the window handles is known once it has
    /// drawn a frame: before, they are taken as handled.
    fn is_enabled(
        &self,
        command: &Command,
        workspace: &Workspace,
        focus: Option<&FocusHandle>,
        window: &Window,
        cx: &App,
    ) -> bool {
        let action = command.action.as_ref();
        let handled = !workspace.has_drawn()
            || self.app_actions.contains(&action.as_any().type_id())
            || focus.map_or_else(
                || window.is_action_available(action, cx),
                |focus| window.is_action_available_in(action, focus),
            );
        handled && command.enabled.is_none_or(|enabled| enabled(workspace, cx))
    }

    /// The buttons of the toolbar of `window`, by groups.
    pub fn toolbar(&self, workspace: &Workspace, window: &Window, cx: &App) -> Vec<Vec<Button>> {
        let focus = workspace.editor().focus_handle(cx);
        let mut groups: Vec<u8> = self.toolbar.iter().map(|(group, _)| *group).collect();
        groups.sort_unstable();
        groups.dedup();
        groups
            .into_iter()
            .map(|group| {
                let ids = self.toolbar.iter().filter(|(of, _)| *of == group).map(|(_, id)| *id);
                ids.map(|id| {
                    let command = self.command(id);
                    let action = command.action.boxed_clone();
                    let keys = keys::for_action(action.as_ref(), &focus, window).map(SharedString::from);
                    Button::new(SharedString::new_static(id), tr(command.label))
                        .disabled(!self.is_enabled(command, workspace, None, window, cx))
                        .tooltip(tr(command.label), keys)
                        .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
                })
                .collect()
            })
            .collect()
    }

    /// The menus of the bar MigPad draws, for the window of `workspace`: those of macOS, without the
    /// menu of the application and what macOS fills in. `target` has the focus the chosen command
    /// goes to.
    pub fn menu_bar(
        &self,
        workspace: &Workspace,
        target: Option<&FocusHandle>,
        window: &Window,
        cx: &App,
    ) -> Vec<MenuSpec> {
        let focus = workspace.editor().focus_handle(cx);
        let menu = |id: MenuId| {
            let mut placements: Vec<&Placement> =
                self.placements.iter().filter(|placement| placement.menu == id).collect();
            placements.sort_by_key(|placement| placement.group);
            let mut items = Vec::with_capacity(placements.len() + 2);
            let mut group = None;
            for placement in placements {
                let Entry::Command(command) = placement.entry else { continue };
                if group.is_some_and(|group| group != placement.group) {
                    items.push(ItemSpec::Separator);
                }
                group = Some(placement.group);
                let command = self.command(command);
                items.push(ItemSpec::Action {
                    label: tr(command.label).into(),
                    mnemonic: mnemonic(command.label),
                    keys: keys::for_action(command.action.as_ref(), &focus, window).map(SharedString::from),
                    checked: command.checked.is_some_and(|checked| checked(workspace, cx)),
                    enabled: self.is_enabled(command, workspace, target, window, cx),
                    action: command.action.boxed_clone(),
                });
            }
            let title = id.key().map(|key| (SharedString::from(tr(key)), mnemonic(key)));
            title.filter(|_| !items.is_empty()).map(|(title, mnemonic)| MenuSpec { title, mnemonic, items })
        };
        MenuId::ALL.into_iter().filter_map(menu).collect()
    }

    /// Adds the list of open windows to a group of a menu.
    pub fn add_window_list(&mut self, menu: MenuId, group: u8) {
        self.placements.push(Placement { menu, group, entry: Entry::Windows });
    }

    /// Handles `A` in each window: the command acts on its document or on the window itself.
    pub fn on_window_action<A: Action>(
        &mut self,
        handler: fn(&mut Workspace, &A, &mut Window, &mut Context<Workspace>),
    ) {
        self.window_handlers
            .push(Rc::new(move |root: Div, cx: &mut Context<Workspace>| root.on_action(cx.listener(handler))));
    }

    /// Handles `A` in the application, whichever window it comes from.
    pub fn on_app_action<A: Action>(&mut self, handler: fn(&A, &mut App)) {
        self.app_actions.insert(TypeId::of::<A>());
        self.app_handlers.push(Box::new(move |cx: &mut App| {
            cx.on_action(handler);
        }));
    }

    pub(crate) fn window_handlers(&self) -> Vec<WindowHandler> {
        self.window_handlers.clone()
    }

    fn key_bindings(&self) -> Vec<KeyBinding> {
        let bindings = self.commands.iter().flat_map(|command| command.keys.iter().map(move |&keys| (keys, command)));
        bindings
            .map(|(keys, command)| {
                KeyBinding::load(keys, command.action.boxed_clone(), None, false, None, &DummyKeyboardMapper)
                    .unwrap_or_else(|error| panic!("{}: {keys}: {error}", command.id))
            })
            .collect()
    }

    /// The menu bar, with the check marks for `workspace`, the root of the active window.
    fn menus(&self, workspace: Option<&Workspace>, cx: &App) -> Vec<Menu> {
        let menu = |id: MenuId| {
            let mut placements: Vec<&Placement> =
                self.placements.iter().filter(|placement| placement.menu == id).collect();
            placements.sort_by_key(|placement| placement.group);
            let mut items = Vec::with_capacity(placements.len() + 2);
            for (i, placement) in placements.iter().enumerate() {
                let entries = self.menu_items(&placement.entry, workspace, cx);
                if !entries.is_empty() && !items.is_empty() && placements[i - 1].group != placement.group {
                    items.push(MenuItem::separator());
                }
                items.extend(entries);
            }
            (!items.is_empty()).then(|| Menu::new(id.title()).items(items))
        };
        MenuId::ALL.into_iter().filter_map(menu).collect()
    }

    fn menu_items(&self, entry: &Entry, workspace: Option<&Workspace>, cx: &App) -> Vec<MenuItem> {
        let id = match entry {
            Entry::Command(id) => id,
            Entry::Services => return vec![MenuItem::os_submenu(tr(Key::AppServices), SystemMenuType::Services)],
            Entry::Windows => return window_list(workspace, cx),
        };
        let command = self.command(id);
        vec![MenuItem::Action {
            name: tr(command.label).into(),
            action: command.action.boxed_clone(),
            os_action: command.os_action,
            checked: command.checked.zip(workspace).is_some_and(|(checked, workspace)| checked(workspace, cx)),
            disabled: false,
        }]
    }
}

/// The items of the open windows, the active one checked. macOS lists the windows itself in the
/// menu called "Window", as it is in English: then there is no list of ours.
fn window_list(workspace: Option<&Workspace>, cx: &App) -> Vec<MenuItem> {
    if !cfg!(target_os = "macos") || MenuId::Window.title() == "Window" {
        return Vec::new();
    }
    let active = cx.active_window().map(|window| window.window_id());
    let mut windows: Vec<(WindowId, String)> =
        windows::workspaces(cx).map(|(window, workspace)| (window.window_id(), workspace.title().to_owned())).collect();
    // The workspace being updated is not among those that can be read.
    if let Some(workspace) = workspace
        && !windows.iter().any(|(id, _)| *id == workspace.window_id())
    {
        windows.push((workspace.window_id(), workspace.title().to_owned()));
    }
    windows.sort_by_key(|(id, _)| *id);
    windows
        .into_iter()
        .map(|(id, title)| MenuItem::Action {
            name: title.into(),
            action: Box::new(ActivateWindow(id)),
            os_action: None,
            checked: Some(id) == active,
            disabled: false,
        })
        .collect()
}

/// Registers the commands of `modules`, binds their keys and handles them.
pub fn init(modules: &[&dyn Module], cx: &mut App) {
    let mut registry = Registry::default();
    for module in modules {
        let first = registry.commands.len();
        module.register(&mut registry);
        debug_assert!(
            registry.commands[first..].iter().all(|command| command.id.split('.').next() == Some(module.id())),
            "the commands of {} start with its name",
            module.id(),
        );
    }
    cx.bind_keys(registry.key_bindings());
    for install in std::mem::take(&mut registry.app_handlers) {
        install(cx);
    }
    cx.set_global(registry);
}

/// Builds the menu bar again: at the start, after a toggle, and when another window becomes
/// active, for the check marks of its document. Items whose action no one would handle now are
/// disabled by GPUI when the menu opens.
pub fn update_menus(workspace: Option<&Workspace>, cx: &mut App) {
    let menus = cx.global::<Registry>().menus(workspace, cx);
    cx.set_menus(menus);
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::modules;

    fn registry() -> Registry {
        let mut registry = Registry::default();
        for module in modules::all() {
            module.register(&mut registry);
        }
        registry
    }

    #[test]
    fn keys_are_bound_once_with_the_editor() {
        let mut seen = HashMap::new();
        for binding in registry().key_bindings().into_iter().chain(migpad_editor::key_bindings()) {
            let keys = binding.keystrokes().iter().map(ToString::to_string).collect::<Vec<_>>().join(" ");
            let action = binding.action().name();
            if let Some(before) = seen.insert(keys.clone(), action) {
                panic!("{keys} is bound to both {before} and {action}");
            }
        }
    }

    #[test]
    fn mnemonics_tell_the_menus_and_their_items_apart() {
        use crate::strings::{LANGUAGE_LOCK, Language, mnemonic, set_language};

        let _lock = LANGUAGE_LOCK.lock();
        let registry = registry();
        let letter = |key: Key| {
            let at = mnemonic(key)?;
            tr(key)[at..].chars().next().map(|c| c.to_lowercase().to_string())
        };
        for language in [Language::English, Language::Russian] {
            set_language(language);
            let menus: Vec<MenuId> = MenuId::ALL.into_iter().filter(|id| *id != MenuId::App).collect();
            let mut titles: Vec<String> = menus.iter().filter_map(|id| letter(id.key()?)).collect();
            assert_eq!(titles.len(), menus.len(), "{language:?}: every menu has a mnemonic");
            titles.sort();
            titles.dedup();
            assert_eq!(titles.len(), menus.len(), "{language:?}: mnemonics of menus repeat");
            for menu in menus {
                let labels: Vec<Key> = registry
                    .placements
                    .iter()
                    .filter(|placement| placement.menu == menu)
                    .filter_map(|placement| match placement.entry {
                        Entry::Command(id) => Some(registry.command(id).label),
                        Entry::Services | Entry::Windows => None,
                    })
                    // The items of the Window menu that only macOS has go without mnemonics.
                    .filter(|key| ![Key::WindowMinimize, Key::WindowZoom].contains(key))
                    .collect();
                let mut letters: Vec<String> = labels.iter().filter_map(|key| letter(*key)).collect();
                assert_eq!(letters.len(), labels.len(), "{language:?} {menu:?}: every item has a mnemonic");
                letters.sort();
                letters.dedup();
                assert_eq!(letters.len(), labels.len(), "{language:?} {menu:?}: mnemonics of items repeat");
            }
        }
        set_language(Language::English);
    }

    #[test]
    fn commands_are_named_once_and_in_the_menus_once() {
        let registry = registry();
        let mut ids: Vec<&str> = registry.commands.iter().map(|command| command.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), registry.commands.len());
        let mut items: Vec<&str> = registry
            .placements
            .iter()
            .filter_map(|placement| match placement.entry {
                Entry::Command(id) => Some(id),
                Entry::Services | Entry::Windows => None,
            })
            .collect();
        assert!(items.iter().all(|item| ids.contains(item)));
        assert!(registry.toolbar.iter().all(|(_, button)| ids.contains(button)));
        items.sort_unstable();
        items.dedup();
        assert_eq!(items.len(), registry.placements.iter().filter(|p| matches!(p.entry, Entry::Command(_))).count());
    }
}
