//! MigPad interface elements: controls, theme, the menu bar for Windows and Linux,
//! context menus and notification bars.

pub mod tab_bar;
pub mod theme;
pub mod tooltip;

use gpui::{Pixels, px};

pub use tab_bar::{TabBar, TabInfo};
pub use theme::{Theme, ThemeMode, theme};
pub use tooltip::Tooltip;

/// The size of the text of the interface, as the controls of the system have it: 9 points of
/// Segoe UI on Windows, 13 of the system font on macOS and Linux.
pub fn text_size() -> Pixels {
    if cfg!(target_os = "windows") { px(12.) } else { px(13.) }
}
