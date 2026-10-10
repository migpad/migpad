//! Editor actions and their keys. The keys are fixed and follow each system: Cmd and Option on
//! macOS, Ctrl on Windows and Linux.

use gpui::{Action, Context, Div, InteractiveElement, KeyBinding, KeyContext, actions};

use crate::view::{Deletion, EditorView, Motion};

/// The key context of the editor; its `mode` tells the view of a document, `full`, from an input
/// field, `single_line`.
const CONTEXT: &str = "Editor";
/// Where the keys that only the view of a document takes are bound: an input field leaves them to
/// the element around it.
const DOCUMENT: &str = "Editor && mode == full";

/// The key context of a view.
pub(crate) fn key_context(single_line: bool) -> KeyContext {
    let mut context = KeyContext::default();
    context.add(CONTEXT);
    context.set("mode", if single_line { "single_line" } else { "full" });
    context
}

/// Declares, for each motion of the caret, an action that moves the caret and one that selects,
/// and the function that handles them.
macro_rules! motions {
    ($($motion:ident: $move:ident, $select:ident;)*) => {
        actions!(editor, [$($move, $select),*]);

        /// Handles the actions of the motions in `div`.
        pub(crate) fn on_motions(div: Div, cx: &mut Context<EditorView>) -> Div {
            div $(
                .on_action(cx.listener(|view, _: &$move, window, cx| {
                    view.move_caret(Motion::$motion, false, window, cx)
                }))
                .on_action(cx.listener(|view, _: &$select, window, cx| {
                    view.move_caret(Motion::$motion, true, window, cx)
                }))
            )*
        }
    };
}

motions! {
    Left: MoveLeft, SelectLeft;
    Right: MoveRight, SelectRight;
    Up: MoveUp, SelectUp;
    Down: MoveDown, SelectDown;
    WordLeft: MoveWordLeft, SelectWordLeft;
    WordRight: MoveWordRight, SelectWordRight;
    LineStart: MoveLineStart, SelectLineStart;
    LineEnd: MoveLineEnd, SelectLineEnd;
    PageUp: MovePageUp, SelectPageUp;
    PageDown: MovePageDown, SelectPageDown;
    DocStart: MoveDocStart, SelectDocStart;
    DocEnd: MoveDocEnd, SelectDocEnd;
}

actions!(
    editor,
    [
        SelectAll,
        BlockSelectUp,
        BlockSelectDown,
        BlockSelectLeft,
        BlockSelectRight,
        Backspace,
        Delete,
        DeleteWordLeft,
        DeleteWordRight,
        DeleteToLineStart,
        Newline,
        Tab,
        Undo,
        Redo,
        Copy,
        Cut,
        Paste,
        UpperCase,
        LowerCase,
        ShowContextMenu
    ]
);

/// Handles the editing actions of every view in `div`.
pub(crate) fn on_edits(div: Div, cx: &mut Context<EditorView>) -> Div {
    div.on_action(cx.listener(|view, _: &SelectAll, window, cx| view.select_all(window, cx)))
        .on_action(cx.listener(|view, _: &Backspace, window, cx| view.delete(Deletion::CharLeft, window, cx)))
        .on_action(cx.listener(|view, _: &Delete, window, cx| view.delete(Deletion::CharRight, window, cx)))
        .on_action(cx.listener(|view, _: &DeleteWordLeft, window, cx| view.delete(Deletion::WordLeft, window, cx)))
        .on_action(cx.listener(|view, _: &DeleteWordRight, window, cx| view.delete(Deletion::WordRight, window, cx)))
        .on_action(cx.listener(|view, _: &DeleteToLineStart, window, cx| view.delete(Deletion::LineStart, window, cx)))
        .on_action(cx.listener(|view, _: &Undo, window, cx| view.undo(window, cx)))
        .on_action(cx.listener(|view, _: &Redo, window, cx| view.redo(window, cx)))
        .on_action(cx.listener(|view, _: &Copy, _, cx| view.copy(cx)))
        .on_action(cx.listener(|view, _: &Cut, window, cx| view.cut(window, cx)))
        .on_action(cx.listener(|view, _: &Paste, window, cx| view.paste(window, cx)))
        .on_action(cx.listener(|view, _: &UpperCase, window, cx| view.change_case(true, window, cx)))
        .on_action(cx.listener(|view, _: &LowerCase, window, cx| view.change_case(false, window, cx)))
        .on_action(cx.listener(|view, _: &ShowContextMenu, _, cx| view.context_menu_at_caret(cx)))
}

/// Handles in `div` the actions that only the view of a document takes: line breaks and tabs. In
/// an input field they go on to the element around it.
pub(crate) fn on_document_actions(div: Div, cx: &mut Context<EditorView>) -> Div {
    div.on_action(cx.listener(|view, _: &Newline, window, cx| view.newline(window, cx)))
        .on_action(cx.listener(|view, _: &Tab, window, cx| view.type_text("\t", window, cx)))
        .on_action(cx.listener(|view, _: &BlockSelectUp, window, cx| view.block_move(Motion::Up, window, cx)))
        .on_action(cx.listener(|view, _: &BlockSelectDown, window, cx| view.block_move(Motion::Down, window, cx)))
        .on_action(cx.listener(|view, _: &BlockSelectLeft, window, cx| view.block_move(Motion::Left, window, cx)))
        .on_action(cx.listener(|view, _: &BlockSelectRight, window, cx| view.block_move(Motion::Right, window, cx)))
}

/// `keys` move the caret; with Shift they select.
fn motion(keys: &str, to: impl Action, select: impl Action) -> [KeyBinding; 2] {
    [KeyBinding::new(keys, to, Some(CONTEXT)), KeyBinding::new(&format!("shift-{keys}"), select, Some(CONTEXT))]
}

/// The keys of the editor on this system; the application checks its own keys against them.
pub fn key_bindings() -> Vec<KeyBinding> {
    let common = [
        motion("left", MoveLeft, SelectLeft),
        motion("right", MoveRight, SelectRight),
        motion("up", MoveUp, SelectUp),
        motion("down", MoveDown, SelectDown),
        motion("home", MoveLineStart, SelectLineStart),
        motion("end", MoveLineEnd, SelectLineEnd),
        motion("pageup", MovePageUp, SelectPageUp),
        motion("pagedown", MovePageDown, SelectPageDown),
    ];
    let system = if cfg!(target_os = "macos") {
        vec![
            motion("cmd-left", MoveLineStart, SelectLineStart),
            motion("cmd-right", MoveLineEnd, SelectLineEnd),
            motion("cmd-up", MoveDocStart, SelectDocStart),
            motion("cmd-down", MoveDocEnd, SelectDocEnd),
            motion("alt-left", MoveWordLeft, SelectWordLeft),
            motion("alt-right", MoveWordRight, SelectWordRight),
            // As in the text fields of macOS.
            motion("ctrl-a", MoveLineStart, SelectLineStart),
            motion("ctrl-e", MoveLineEnd, SelectLineEnd),
            motion("ctrl-b", MoveLeft, SelectLeft),
            motion("ctrl-f", MoveRight, SelectRight),
            motion("ctrl-p", MoveUp, SelectUp),
            motion("ctrl-n", MoveDown, SelectDown),
        ]
    } else {
        vec![
            motion("ctrl-home", MoveDocStart, SelectDocStart),
            motion("ctrl-end", MoveDocEnd, SelectDocEnd),
            motion("ctrl-left", MoveWordLeft, SelectWordLeft),
            motion("ctrl-right", MoveWordRight, SelectWordRight),
        ]
    };
    let mut bindings: Vec<KeyBinding> = common.into_iter().chain(system).flatten().collect();
    let context = Some(CONTEXT);
    bindings.extend([
        KeyBinding::new("secondary-a", SelectAll, context),
        KeyBinding::new("backspace", Backspace, context),
        KeyBinding::new("shift-backspace", Backspace, context),
        KeyBinding::new("delete", Delete, context),
        // In an input field Enter is what the field is for, and Tab moves the focus.
        KeyBinding::new("enter", Newline, Some(DOCUMENT)),
        KeyBinding::new("shift-enter", Newline, Some(DOCUMENT)),
        KeyBinding::new("tab", Tab, Some(DOCUMENT)),
        KeyBinding::new("secondary-z", Undo, context),
        KeyBinding::new("secondary-shift-z", Redo, context),
        KeyBinding::new("secondary-c", Copy, context),
        KeyBinding::new("secondary-x", Cut, context),
        KeyBinding::new("secondary-v", Paste, context),
    ]);
    let document = Some(DOCUMENT);
    bindings.extend([
        KeyBinding::new("alt-shift-up", BlockSelectUp, document),
        KeyBinding::new("alt-shift-down", BlockSelectDown, document),
    ]);
    if cfg!(target_os = "macos") {
        bindings.extend([
            KeyBinding::new("alt-backspace", DeleteWordLeft, context),
            KeyBinding::new("alt-delete", DeleteWordRight, context),
            KeyBinding::new("cmd-backspace", DeleteToLineStart, context),
            // As in the text fields of macOS.
            KeyBinding::new("ctrl-h", Backspace, context),
            KeyBinding::new("ctrl-d", Delete, context),
        ]);
    } else {
        bindings.extend([
            KeyBinding::new("alt-shift-left", BlockSelectLeft, document),
            KeyBinding::new("alt-shift-right", BlockSelectRight, document),
            KeyBinding::new("ctrl-backspace", DeleteWordLeft, context),
            KeyBinding::new("ctrl-delete", DeleteWordRight, context),
            KeyBinding::new("ctrl-y", Redo, context),
            // The keys of the clipboard from the days of IBM CUA, still at home on Windows.
            KeyBinding::new("ctrl-insert", Copy, context),
            KeyBinding::new("shift-delete", Cut, context),
            KeyBinding::new("shift-insert", Paste, context),
            // The context menu at the caret, as Windows has it.
            KeyBinding::new("shift-f10", ShowContextMenu, context),
            KeyBinding::new("menu", ShowContextMenu, context),
        ]);
    }
    bindings
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[test]
    fn keys_are_bound_once() {
        let mut seen = HashMap::new();
        for binding in key_bindings() {
            let keys = binding.keystrokes().iter().map(ToString::to_string).collect::<Vec<_>>().join(" ");
            let action = binding.action().name();
            if let Some(before) = seen.insert(keys.clone(), action) {
                panic!("{keys} is bound to both {before} and {action}");
            }
        }
    }

    #[test]
    fn input_fields_leave_enter_and_tab_to_the_element_around_them() {
        let (document, field) = ([key_context(false)], [key_context(true)]);
        for binding in key_bindings() {
            let action = binding.action().name();
            let predicate = binding.predicate().expect("editor keys have a context");
            assert!(predicate.eval(&document), "{action} in a document");
            let document_only = [
                "editor::Newline",
                "editor::Tab",
                "editor::BlockSelectUp",
                "editor::BlockSelectDown",
                "editor::BlockSelectLeft",
                "editor::BlockSelectRight",
            ]
            .contains(&action);
            assert_eq!(predicate.eval(&field), !document_only, "{action} in an input field");
        }
    }
}
