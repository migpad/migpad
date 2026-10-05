//! MigPad editor view: line layout, rendering, selections, caret, text input and IME,
//! and the single-line mode used by input fields.

mod columns;
mod display;
mod element;
mod indent;
mod keymap;
mod layout;
mod movement;
mod utf16;
mod view;
mod wrap;

pub use keymap::key_bindings;
pub use view::EditorView;

/// The editing actions that the application puts into its menus.
pub mod actions {
    pub use crate::keymap::{Copy, Cut, Paste, Redo, SelectAll, Undo};
}

/// Binds the keys of the editor; called once when the application starts.
pub fn init(cx: &mut gpui::App) {
    cx.bind_keys(keymap::key_bindings());
}
