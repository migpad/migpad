//! How documents show, as the settings of the application have them: their font and its size, the
//! width of a tab, and the toggles of the View menu. A global of GPUI, as the colors are: each view
//! of a document takes it as it lays out its lines, and input fields keep the defaults.

use gpui::{App, Font, FontId, Global, SharedString, TextSystem};

/// How documents show.
#[derive(Clone, Debug, PartialEq)]
pub struct EditorSettings {
    /// The family of the font; `None` for the monospace font of the system, see [`system_font`].
    pub font: Option<SharedString>,
    pub font_size: f32,
    /// Columns between tab stops.
    pub tab_width: usize,
    /// Whether lines wrap to the width of the view; those of a large file never do.
    pub word_wrap: bool,
    /// Whether spaces, tabs and line breaks are marked.
    pub show_whitespace: bool,
    /// Whether levels of indentation are marked by vertical lines.
    pub show_indent_guides: bool,
}

impl Default for EditorSettings {
    fn default() -> Self {
        EditorSettings {
            font: None,
            font_size: 13.0,
            tab_width: 8,
            word_wrap: true,
            show_whitespace: false,
            show_indent_guides: false,
        }
    }
}

impl Global for EditorSettings {}

impl EditorSettings {
    /// The settings the application gave, or the defaults.
    pub fn current(cx: &App) -> EditorSettings {
        cx.try_global::<EditorSettings>().cloned().unwrap_or_default()
    }
}

/// Whether the system has the font `family`.
pub fn font_installed(family: &str, cx: &App) -> bool {
    installed(cx.text_system(), family).is_some()
}

/// The font `family` and its id, if the system has it: a family it has not resolves to a fallback,
/// which may not even be monospace.
pub(crate) fn installed(text_system: &TextSystem, family: &str) -> Option<(Font, FontId)> {
    let font = gpui::font(SharedString::from(family.to_owned()));
    let id = text_system.resolve_font(&font);
    text_system.get_font_for_id(id).is_some_and(|resolved| resolved.family == font.family).then_some((font, id))
}

/// The monospace font of the system.
pub fn system_font() -> &'static str {
    if cfg!(target_os = "macos") {
        "Menlo"
    } else if cfg!(target_os = "windows") {
        "Consolas"
    } else {
        "DejaVu Sans Mono"
    }
}
