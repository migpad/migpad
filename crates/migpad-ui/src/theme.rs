//! The theme: the colors of the interface and of the editor, light and dark. By default it follows
//! the appearance of the system and changes with it; it can also stay light or dark.

use gpui::{App, Global, Subscription, Window, WindowAppearance};
use migpad_editor::EditorColors;

/// The colors of the interface as `0xRRGGBB`, a shadow as `0xRRGGBBAA`, and those of the editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub appearance: Appearance,
    /// Bars around the text: the menu bar, the toolbar, the tabs, the status bar.
    pub bar: u32,
    /// Lines between the bars, around menus and tooltips.
    pub border: u32,
    pub text: u32,
    /// Text of less weight: inactive tabs, the status bar, keys in menus.
    pub text_muted: u32,
    pub text_disabled: u32,
    /// Under a control the mouse is over.
    pub hover: u32,
    /// Under a control being pressed, and a tab being dragged over.
    pub pressed: u32,
    /// The tab of the document shown: the background of the text.
    pub tab_active: u32,
    /// The line over the active tab, and keyboard focus.
    pub accent: u32,
    pub menu: u32,
    /// The highlighted item of a menu.
    pub menu_selected: u32,
    pub shadow: u32,
    pub tooltip: u32,
    /// Notification bars: the background, and the edge that tells what kind they are.
    pub info: u32,
    pub info_edge: u32,
    pub warning: u32,
    pub warning_edge: u32,
    pub error: u32,
    pub error_edge: u32,
    pub editor: EditorColors,
}

/// Whether a theme is light or dark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Appearance {
    Light,
    Dark,
}

impl From<WindowAppearance> for Appearance {
    fn from(appearance: WindowAppearance) -> Self {
        match appearance {
            WindowAppearance::Light | WindowAppearance::VibrantLight => Appearance::Light,
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Appearance::Dark,
        }
    }
}

pub const LIGHT: Theme = Theme {
    appearance: Appearance::Light,
    bar: 0xf3f3f3,
    border: 0xd9d9d9,
    text: 0x1f1f1f,
    text_muted: 0x666666,
    text_disabled: 0xa6a6a6,
    hover: 0xe3e3e3,
    pressed: 0xd4d4d4,
    tab_active: 0xffffff,
    accent: 0x2f6fde,
    menu: 0xfbfbfb,
    menu_selected: 0xdfe8f6,
    shadow: 0x00000030,
    tooltip: 0xfdfdfd,
    info: 0xe9f1fd,
    info_edge: 0x2f6fde,
    warning: 0xfdf3dc,
    warning_edge: 0xc98a0b,
    error: 0xfde8e8,
    error_edge: 0xd13438,
    editor: EditorColors::LIGHT,
};

pub const DARK: Theme = Theme {
    appearance: Appearance::Dark,
    bar: 0x2a2a2a,
    border: 0x3d3d3d,
    text: 0xe2e2e2,
    text_muted: 0x9e9e9e,
    text_disabled: 0x666666,
    hover: 0x3a3a3a,
    pressed: 0x474747,
    tab_active: 0x1e1e1e,
    accent: 0x4d8eff,
    menu: 0x2e2e2e,
    menu_selected: 0x264f78,
    shadow: 0x00000080,
    tooltip: 0x303030,
    info: 0x1c2a3d,
    info_edge: 0x4d8eff,
    warning: 0x3a3120,
    warning_edge: 0xe0a83a,
    error: 0x3e2222,
    error_edge: 0xf06060,
    editor: EditorColors::DARK,
};

/// Which theme the interface takes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThemeMode {
    /// Light or dark, as the system is.
    #[default]
    System,
    Light,
    Dark,
}

/// The mode and the theme it gave.
struct ThemeState {
    mode: ThemeMode,
    theme: &'static Theme,
}

impl Global for ThemeState {}

/// The theme the interface paints with.
pub fn theme(cx: &App) -> &'static Theme {
    cx.try_global::<ThemeState>().map_or(&LIGHT, |state| state.theme)
}

/// Sets the theme `mode` from now on: the windows repaint at once.
pub fn set_mode(mode: ThemeMode, cx: &mut App) {
    let theme = match mode {
        ThemeMode::Light => &LIGHT,
        ThemeMode::Dark => &DARK,
        ThemeMode::System => by_appearance(cx.window_appearance().into()),
    };
    apply(ThemeState { mode, theme }, cx);
}

/// Keeps the theme in step with the appearance of the system, as `window` reports it; call once
/// for each window.
pub fn follow_system(window: &mut Window, cx: &mut App) -> Subscription {
    system_appearance(window.appearance().into(), cx);
    window.observe_window_appearance(|window, cx| system_appearance(window.appearance().into(), cx))
}

fn system_appearance(appearance: Appearance, cx: &mut App) {
    let mode = cx.try_global::<ThemeState>().map_or(ThemeMode::System, |state| state.mode);
    if mode == ThemeMode::System {
        apply(ThemeState { mode, theme: by_appearance(appearance) }, cx);
    }
}

fn by_appearance(appearance: Appearance) -> &'static Theme {
    match appearance {
        Appearance::Light => &LIGHT,
        Appearance::Dark => &DARK,
    }
}

fn apply(state: ThemeState, cx: &mut App) {
    let changed = cx.try_global::<ThemeState>().is_none_or(|old| old.theme != state.theme);
    cx.set_global(state.theme.editor);
    cx.set_global(state);
    if changed {
        cx.refresh_windows();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The contrast ratio of two colors, from 1 to 21, as WCAG defines it.
    fn contrast(a: u32, b: u32) -> f64 {
        let luminance = |color: u32| {
            let [_, r, g, b] = color.to_be_bytes().map(|c| {
                let c = f64::from(c) / 255.0;
                if c <= 0.03928 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
            });
            0.2126 * r + 0.7152 * g + 0.0722 * b
        };
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn text_stands_out_from_its_background() {
        for theme in [&LIGHT, &DARK] {
            let editor = theme.editor;
            let pairs = [
                ("text on bars", theme.text, theme.bar),
                ("text on the active tab", theme.text, theme.tab_active),
                ("text on a hovered control", theme.text, theme.hover),
                ("muted text on bars", theme.text_muted, theme.bar),
                ("muted text on the active tab", theme.text_muted, theme.tab_active),
                ("menus", theme.text, theme.menu),
                ("the highlighted item", theme.text, theme.menu_selected),
                ("tooltips", theme.text, theme.tooltip),
                ("information", theme.text, theme.info),
                ("warnings", theme.text, theme.warning),
                ("errors", theme.text, theme.error),
                ("the text of documents", editor.text, editor.background),
                ("selected text", editor.text, editor.selection),
                ("line numbers", editor.line_number, editor.gutter),
            ];
            for (what, text, background) in pairs {
                let ratio = contrast(text, background);
                assert!(ratio >= 4.5, "{:?}: {what}: contrast {ratio:.2}", theme.appearance);
            }
            // Disabled controls and invisible characters are faint, yet seen.
            assert!(contrast(theme.text_disabled, theme.bar) >= 2.0, "{:?}", theme.appearance);
            assert!(contrast(editor.mark, editor.background) >= 2.0, "{:?}", theme.appearance);
        }
    }
}
