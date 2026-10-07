//! MigPad interface elements: controls, theme, the menu bar for Windows and Linux,
//! context menus and notification bars.

pub mod button;
pub mod context_menu;
mod menu;
pub mod menu_bar;
pub mod notification;
pub mod status_bar;
pub mod tab_bar;
pub mod text_field;
pub mod theme;
pub mod tooltip;

use gpui::{Pixels, px};

pub use button::Button;
pub use context_menu::ContextMenu;
pub use menu::{ItemSpec, action_at};
pub use menu_bar::MenuBar;
pub use status_bar::StatusBar;
pub use tab_bar::{TabBar, TabInfo};
pub use text_field::TextField;
pub use theme::{Theme, ThemeMode, theme};
pub use tooltip::Tooltip;

/// The size of the text of the interface, as the controls of the system have it: 9 points of
/// Segoe UI on Windows, 13 of the system font on macOS and Linux.
pub fn text_size() -> Pixels {
    if cfg!(target_os = "windows") { px(12.) } else { px(13.) }
}
