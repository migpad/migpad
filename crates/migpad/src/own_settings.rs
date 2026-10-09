//! The settings window MigPad draws: on Windows and Linux, and on macOS in a debug build with
//! `MIGPAD_OWN_SETTINGS=1`, to check it. Lists, fields of numbers with − and + buttons, a check box
//! and a button, laid out as the dialogs of these systems are: the labels on the left, the fields
//! on the right. Tab goes from field to field, Escape closes the window. A change takes effect
//! once the window is done with the event that made it.

use gpui::{
    AnyElement, App, AppContext, Bounds, Context, DismissEvent, Entity, FocusHandle, Focusable, Global, IntoElement,
    KeyBinding, ParentElement, Pixels, Render, SharedString, Styled, Subscription, TextSystem, TitlebarOptions, Window,
    WindowBounds, WindowHandle, WindowOptions, actions, div, prelude::*, px, rgb, size,
};
use migpad_core::history::Selection;
use migpad_core::settings::{FONT_SIZES, Setting, Settings, TAB_WIDTHS};
use migpad_editor::EditorView;
use migpad_ui::{Button, Checkbox, ContextMenu, Dropdown, ItemSpec, TextField, text_size, theme};

use crate::settings;
use crate::settings_window::{self, LANGUAGES, THEMES, font_label, language_label, theme_label};
use crate::strings::{Key, Plural, plural, tr};

/// The key context of the window: Tab, Shift+Tab and Escape.
const CONTEXT: &str = "SettingsWindow";
const LABEL_WIDTH: f32 = 140.;
const LIST_WIDTH: f32 = 240.;
const NUMBER_WIDTH: f32 = 48.;

actions!(settings_window, [FocusNext, FocusPrevious, Close]);

/// A list of the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum List {
    Language,
    Theme,
    Font,
}

/// An item of a list was chosen: its setting, fixed as the list opened — the fonts found meanwhile
/// do not change what an item stands for.
#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = settings_window, no_json)]
struct Choose(Setting);

/// A field of a number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Number {
    FontSize,
    TabWidth,
}

impl Number {
    fn setting(self, n: u32) -> Option<Setting> {
        let (range, setting): (_, fn(u32) -> Setting) = match self {
            Number::FontSize => (FONT_SIZES, Setting::FontSize),
            Number::TabWidth => (TAB_WIDTHS, Setting::TabWidth),
        };
        range.contains(&n).then(|| setting(n))
    }

    fn value(self, settings: &Settings) -> u32 {
        match self {
            Number::FontSize => settings.font_size,
            Number::TabWidth => settings.tab_width,
        }
    }
}

/// The window, while it is open.
struct Opened(WindowHandle<SettingsView>);

impl Global for Opened {}

/// Binds the keys of the window; called once when the application starts.
pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", FocusNext, Some(CONTEXT)),
        KeyBinding::new("shift-tab", FocusPrevious, Some(CONTEXT)),
        KeyBinding::new("escape", Close, Some(CONTEXT)),
    ]);
}

/// Opens the window, or brings it forward.
pub fn show(cx: &mut App) {
    if let Some(window) = cx.try_global::<Opened>().map(|Opened(window)| *window)
        && window.update(cx, |_, window, _| window.activate_window()).is_ok()
    {
        return;
    }
    let bounds = Bounds::centered(None, size(px(540.), px(330.)), cx);
    let options = WindowOptions {
        titlebar: Some(TitlebarOptions { title: Some(settings_window::title().into()), ..Default::default() }),
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        is_resizable: false,
        is_minimizable: false,
        ..Default::default()
    };
    let opened = cx.open_window(options, |window, cx| cx.new(|cx| SettingsView::new(window, cx)));
    match opened {
        Ok(window) => cx.set_global(Opened(window)),
        Err(error) => eprintln!("MigPad could not open its settings window: {error:#}"),
    }
}

/// The settings changed, or the language: the window shows them, if it is open.
pub fn refresh(cx: &mut App) {
    if let Some(window) = cx.try_global::<Opened>().map(|Opened(window)| *window) {
        let _ = window.update(cx, |view, window, cx| view.refresh(window, cx));
    }
}

pub struct SettingsView {
    language: FocusHandle,
    theme: FocusHandle,
    font: FocusHandle,
    font_size: Entity<EditorView>,
    tab_width: Entity<EditorView>,
    restore: FocusHandle,
    open: FocusHandle,
    /// The monospace fonts of the system, once they are found.
    fonts: Option<Vec<String>>,
    /// The items of a list, while they are open.
    menu: Option<(Entity<ContextMenu>, Subscription)>,
    _subscriptions: Vec<Subscription>,
}

impl SettingsView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let font_size = cx.new(|cx| EditorView::single_line(window, cx));
        let tab_width = cx.new(|cx| EditorView::single_line(window, cx));
        let mut subscriptions = Vec::new();
        for (field, number) in [(&font_size, Number::FontSize), (&tab_width, Number::TabWidth)] {
            // A number typed takes effect as it is typed; text that is not one gives way to the
            // setting once the field loses the keyboard.
            let document = field.read(cx).document().clone();
            subscriptions.push(cx.observe(&document, move |view, _, cx| view.typed(number, cx)));
            let focus = field.focus_handle(cx);
            subscriptions.push(cx.on_blur(&focus, window, |view, window, cx| view.refresh(window, cx)));
        }
        // Back in the window: another program may have changed the file of the settings.
        subscriptions.push(cx.observe_window_activation(window, |_, window, cx| {
            if window.is_window_active() {
                cx.defer(settings::check_file);
            }
        }));
        // The fonts of the system are many: they are measured in the background.
        let text_system = cx.text_system().clone();
        let fonts = cx.background_spawn(async move { monospace_fonts(&text_system) });
        cx.spawn(async move |view, cx| {
            let fonts = fonts.await;
            let _ = view.update(cx, |view, cx| {
                view.fonts = Some(fonts);
                cx.notify();
            });
        })
        .detach();
        let stop = |cx: &mut Context<Self>| cx.focus_handle().tab_stop(true);
        let mut view = SettingsView {
            language: stop(cx),
            theme: stop(cx),
            font: stop(cx),
            font_size,
            tab_width,
            restore: stop(cx),
            open: stop(cx),
            fonts: None,
            menu: None,
            _subscriptions: subscriptions,
        };
        view.refresh(window, cx);
        window.focus(&view.language, cx);
        view
    }

    /// The field of `number`.
    fn field(&self, number: Number) -> &Entity<EditorView> {
        match number {
            Number::FontSize => &self.font_size,
            Number::TabWidth => &self.tab_width,
        }
    }

    /// Shows the settings as they are now, in the language of the interface now: a field shows
    /// its setting unless it has the keyboard.
    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.set_window_title(settings_window::title());
        let settings = settings::get(cx).clone();
        for number in [Number::FontSize, Number::TabWidth] {
            let field = self.field(number).clone();
            if !field.focus_handle(cx).is_focused(window) {
                show_number(&field, number.value(&settings), window, cx);
            }
        }
        cx.notify();
    }

    /// A field of a number was typed in: a number it takes is the setting now.
    fn typed(&mut self, number: Number, cx: &mut Context<Self>) {
        let text = self.field(number).read(cx).text(cx);
        if let Some(setting) = text.trim().parse().ok().and_then(|n| number.setting(n)) {
            set(setting, cx);
        }
    }

    /// The settings one step from what a field of a number has: − and + buttons. The field shows
    /// the number even while it has the keyboard.
    fn step(&mut self, number: Number, delta: i64, window: &mut Window, cx: &mut Context<Self>) {
        let value = i64::from(number.value(settings::get(cx))) + delta;
        let Some(n) = u32::try_from(value).ok() else { return };
        if let Some(setting) = number.setting(n) {
            show_number(&self.field(number).clone(), n, window, cx);
            set(setting, cx);
        }
    }

    /// Tab or Shift+Tab reached a field of a number: its number is selected, to type over, as in
    /// the fields of Windows and Linux.
    fn select_focused_number(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for number in [Number::FontSize, Number::TabWidth] {
            let field = self.field(number).clone();
            if field.focus_handle(cx).is_focused(window) {
                field.update(cx, |field, cx| field.select_all(window, cx));
            }
        }
    }

    /// The items of `list` — their labels and settings — and the place of the one chosen now.
    fn items(&self, list: List, settings: &Settings) -> (Vec<(String, Setting)>, Option<usize>) {
        match list {
            List::Language => {
                let chosen = LANGUAGES.iter().position(|&language| language == settings.language);
                let items = LANGUAGES.iter().map(|&language| (language_label(language), Setting::Language(language)));
                (items.collect(), chosen)
            }
            List::Theme => {
                let chosen = THEMES.iter().position(|&theme| theme == settings.theme);
                let items = THEMES.iter().map(|&theme| (theme_label(theme).to_owned(), Setting::Theme(theme)));
                (items.collect(), chosen)
            }
            List::Font => {
                let fonts = self.fonts(settings);
                let chosen = match &settings.font {
                    None => Some(0),
                    Some(font) => fonts.iter().position(|family| family == font).map(|at| at + 1),
                };
                let system = (font_label(None), Setting::Font(None));
                let families = fonts.into_iter().map(|family| (family.clone(), Setting::Font(Some(family))));
                (std::iter::once(system).chain(families).collect(), chosen)
            }
        }
    }

    /// The fonts of the list, after that of the system: the monospace ones, with the font of the
    /// settings among them if it is not one of them.
    fn fonts(&self, settings: &Settings) -> Vec<String> {
        let mut fonts = self.fonts.clone().unwrap_or_default();
        if let Some(font) = &settings.font
            && !fonts.contains(font)
        {
            fonts.push(font.clone());
            fonts.sort_by_key(|family| family.to_lowercase());
        }
        fonts
    }

    fn choose(&mut self, Choose(setting): &Choose, _: &mut Window, cx: &mut Context<Self>) {
        set(setting.clone(), cx);
    }

    /// Up and Down on a list: the item before or after the one chosen, without opening it.
    fn step_list(&mut self, list: List, delta: isize, cx: &mut Context<Self>) {
        let settings = settings::get(cx).clone();
        let (items, chosen) = self.items(list, &settings);
        let index = chosen.map_or(0, |chosen| chosen.saturating_add_signed(delta).min(items.len().saturating_sub(1)));
        if let Some((_, setting)) = items.into_iter().nth(index) {
            set(setting, cx);
        }
    }

    /// Opens the items of `list` under it, at `bounds`.
    fn open_list(
        &mut self,
        list: List,
        bounds: Bounds<Pixels>,
        keyboard: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let settings = settings::get(cx).clone();
        let (choices, chosen) = self.items(list, &settings);
        let mut items: Vec<ItemSpec> = choices
            .into_iter()
            .enumerate()
            .map(|(index, (label, setting))| ItemSpec::Action {
                label: label.into(),
                mnemonic: None,
                keys: None,
                checked: Some(chosen == Some(index)),
                enabled: true,
                action: Box::new(Choose(setting)),
            })
            .collect();
        // The font of the system stands apart from the others.
        let apart = list == List::Font && items.len() > 1;
        if apart {
            items.insert(1, ItemSpec::Separator);
        }
        // The list opens on the item chosen, as the lists of Windows and Linux do.
        let highlighted = chosen.map(|chosen| if apart && chosen > 0 { chosen + 1 } else { chosen });
        let menu = cx.new(|cx| {
            let menu = ContextMenu::new(items, bounds.bottom_left(), keyboard, window, cx);
            match highlighted {
                Some(item) => menu.highlighted(item),
                None => menu,
            }
        });
        let closed = cx.subscribe_in(&menu, window, |view, _, _: &DismissEvent, _, cx| {
            view.menu = None;
            cx.notify();
        });
        self.menu = Some((menu, closed));
        cx.notify();
    }

    fn dropdown(
        &self,
        list: List,
        focus: &FocusHandle,
        name: Key,
        settings: &Settings,
        cx: &Context<Self>,
    ) -> Dropdown {
        let (items, chosen) = self.items(list, settings);
        let label = chosen.and_then(|chosen| items.into_iter().nth(chosen)).map(|(label, _)| label).unwrap_or_default();
        let (opener, stepper) = (cx.weak_entity(), cx.weak_entity());
        Dropdown::new(SharedString::from(format!("{list:?}")), label, tr(name), focus, px(LIST_WIDTH))
            .on_open(move |bounds, keyboard, window, cx| {
                let _ = opener.update(cx, |view, cx| view.open_list(list, bounds, keyboard, window, cx));
            })
            .on_step(move |delta, _, cx| {
                let _ = stepper.update(cx, |view, cx| view.step_list(list, delta, cx));
            })
    }

    fn number(&self, number: Number, cx: &Context<Self>) -> AnyElement {
        let (less, more) = (cx.weak_entity(), cx.weak_entity());
        let id = format!("{number:?}");
        div()
            .flex()
            .items_center()
            .gap(px(4.))
            .child(div().w(px(NUMBER_WIDTH)).child(TextField::new(self.field(number).clone())))
            .child(Button::new(SharedString::from(format!("{id}-less")), "−").on_click(move |_, window, cx| {
                let _ = less.update(cx, |view, cx| view.step(number, -1, window, cx));
            }))
            .child(Button::new(SharedString::from(format!("{id}-more")), "+").on_click(move |_, window, cx| {
                let _ = more.update(cx, |view, cx| view.step(number, 1, window, cx));
            }))
            .into_any_element()
    }
}

/// Changes a setting once the window is done with the event: the window shows it then.
fn set(setting: Setting, cx: &mut App) {
    cx.defer(move |cx| settings::set(setting, cx));
}

/// Puts `n` in a field of a number, the caret after it: a field without the keyboard shows no
/// selection.
fn show_number(field: &Entity<EditorView>, n: u32, window: &mut Window, cx: &mut App) {
    let text = n.to_string();
    if field.read(cx).text(cx) != text {
        field.update(cx, |field, cx| {
            field.set_text(&text, window, cx);
            field.select(Selection::caret(text.len()), window, cx);
        });
    }
}

/// A row of the window: a label on the left, `control` on the right.
fn row(label: &'static str, control: impl IntoElement) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(div().flex().flex_none().justify_end().w(px(LABEL_WIDTH)).child(label))
        .child(control)
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme(cx);
        let settings = settings::get(cx).clone();
        let open_focused = self.open.is_focused(window);
        let unit = plural(Plural::SettingsTabWidthUnit, u64::from(settings.tab_width), &[]);
        div()
            .key_context(CONTEXT)
            .on_action(cx.listener(|view, _: &FocusNext, window, cx| {
                window.focus_next(cx);
                view.select_focused_number(window, cx);
            }))
            .on_action(cx.listener(|view, _: &FocusPrevious, window, cx| {
                window.focus_prev(cx);
                view.select_focused_number(window, cx);
            }))
            .on_action(cx.listener(|_, _: &Close, window, _| window.remove_window()))
            .on_action(cx.listener(Self::choose))
            .size_full()
            .flex()
            .flex_col()
            .gap(px(10.))
            .p(px(20.))
            .bg(rgb(theme.bar))
            .text_color(rgb(theme.text))
            .text_size(text_size())
            .child(row(
                tr(Key::SettingsLanguage),
                self.dropdown(List::Language, &self.language, Key::SettingsLanguage, &settings, cx),
            ))
            .child(row(
                tr(Key::SettingsTheme),
                self.dropdown(List::Theme, &self.theme, Key::SettingsTheme, &settings, cx),
            ))
            .child(row(tr(Key::SettingsFont), self.dropdown(List::Font, &self.font, Key::SettingsFont, &settings, cx)))
            .child(row(tr(Key::SettingsFontSize), self.number(Number::FontSize, cx)))
            .child(row(
                tr(Key::SettingsTabWidth),
                div().flex().items_center().gap(px(6.)).child(self.number(Number::TabWidth, cx)).child(unit),
            ))
            .child(row(
                tr(Key::SettingsStartup),
                Checkbox::new("restore", tr(Key::SettingsRestoreSession), settings.restore_session, &self.restore)
                    .on_toggle(|on, _, cx| set(Setting::RestoreSession(on), cx)),
            ))
            .child(
                div()
                    .pl(px(LABEL_WIDTH + 8. + 20.))
                    .mt(px(-6.))
                    .text_color(rgb(theme.text_muted))
                    .text_size(text_size() * 0.9)
                    .child(tr(Key::SettingsRestoreNote)),
            )
            .child(
                // As wide as its text, not as the window.
                div().flex().pl(px(LABEL_WIDTH + 8.)).pt(px(6.)).child(
                    div()
                        .id("open-file")
                        .track_focus(&self.open)
                        .rounded(px(5.))
                        .border_1()
                        .border_color(rgb(if open_focused { theme.accent } else { theme.border }))
                        .on_key_down(|event: &gpui::KeyDownEvent, _, cx| {
                            let key = event.keystroke.key.as_str();
                            if (key == "enter" || key == "space") && !event.keystroke.modifiers.modified() {
                                cx.defer(settings::open_file);
                                cx.stop_propagation();
                            }
                        })
                        .child(
                            Button::new("open-file-button", tr(Key::SettingsOpenFile))
                                .on_click(|_, _, cx| cx.defer(settings::open_file)),
                        ),
                ),
            )
            .children(self.menu.as_ref().map(|(menu, _)| menu.clone()))
    }
}

/// The families of monospace fonts the system has: those whose narrow and wide letters, digits
/// and dots are as wide. A family the system cannot resolve to itself is left out.
fn monospace_fonts(text_system: &TextSystem) -> Vec<String> {
    let size = px(16.);
    let mut fonts: Vec<String> = text_system
        .all_font_names()
        .into_iter()
        .filter(|name| {
            let font = gpui::font(SharedString::from(name.clone()));
            let id = text_system.resolve_font(&font);
            if text_system.get_font_for_id(id).is_none_or(|resolved| resolved.family != font.family) {
                return false;
            }
            let width = |c| text_system.advance(id, size, c).ok().map(|advance| advance.width);
            let first = width('i');
            first.is_some_and(|first| first > px(0.)) && ['W', 'm', '.', '0'].into_iter().all(|c| width(c) == first)
        })
        .collect();
    fonts.sort_by_key(|family| family.to_lowercase());
    fonts
}
