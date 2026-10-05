//! MigPad editor view: line layout, rendering, selections, caret, text input and IME,
//! and the single-line mode used by input fields.

mod display;
mod element;
mod keymap;
mod layout;
mod movement;
mod view;

pub use view::EditorView;

/// Binds the keys of the editor; called once when the application starts.
pub fn init(cx: &mut gpui::App) {
    cx.bind_keys(keymap::key_bindings());
}
