//! Editor actions and their keys. The keys are fixed and follow each system: Cmd and Option on
//! macOS, Ctrl on Windows and Linux.

use gpui::{Action, Context, Div, InteractiveElement, KeyBinding, actions};

use crate::view::{EditorView, Motion};

/// The key context of the editor.
pub(crate) const CONTEXT: &str = "Editor";

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

actions!(editor, [SelectAll]);

/// `keys` move the caret; with Shift they select.
fn motion(keys: &str, to: impl Action, select: impl Action) -> [KeyBinding; 2] {
    [KeyBinding::new(keys, to, Some(CONTEXT)), KeyBinding::new(&format!("shift-{keys}"), select, Some(CONTEXT))]
}

/// The keys of the editor on this system.
pub(crate) fn key_bindings() -> Vec<KeyBinding> {
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
    bindings.push(KeyBinding::new("secondary-a", SelectAll, Some(CONTEXT)));
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
}
