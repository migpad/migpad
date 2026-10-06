//! Keys of commands as the interface shows them, in tooltips and in the menus that MigPad draws:
//! ⇧⌘T on macOS, Ctrl+Shift+T on Windows and Linux.

use gpui::{Action, FocusHandle, Keystroke, Window};

/// The label of the keys of `action` in the element with `focus`, the view of the document:
/// those bound first, as the menus of macOS show them.
pub fn for_action(action: &dyn Action, focus: &FocusHandle, window: &Window) -> Option<String> {
    let binding = window.bindings_for_action_in(action, focus).into_iter().next()?;
    let keystrokes: Vec<String> =
        binding.keystrokes().iter().map(|keystroke| keystroke_label(keystroke.inner(), os())).collect();
    Some(keystrokes.join(" "))
}

fn os() -> Os {
    if cfg!(target_os = "macos") {
        Os::Mac
    } else if cfg!(target_os = "windows") {
        Os::Windows
    } else {
        Os::Linux
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Os {
    Mac,
    Windows,
    Linux,
}

#[cfg(test)]
fn label_on(keys: &str, os: Os) -> String {
    keys.split(' ')
        .filter_map(|keystroke| Keystroke::parse(keystroke).ok())
        .map(|keystroke| keystroke_label(&keystroke, os))
        .collect::<Vec<_>>()
        .join(" ")
}

fn keystroke_label(keystroke: &Keystroke, os: Os) -> String {
    let modifiers = &keystroke.modifiers;
    // "cmd-}" is Shift and ] on the keyboard: Cmd+Shift+] is how menus show it.
    let (key, shifted) = match keystroke.key.as_str() {
        "}" => ("]", true),
        "{" => ("[", true),
        key => (key, false),
    };
    let shift = modifiers.shift || shifted;
    if os == Os::Mac {
        let mut label = String::new();
        for (on, symbol) in [(modifiers.control, '⌃'), (modifiers.alt, '⌥'), (shift, '⇧'), (modifiers.platform, '⌘')]
        {
            if on {
                label.push(symbol);
            }
        }
        label + &key_name(key, os)
    } else {
        let platform = if os == Os::Windows { "Win" } else { "Super" };
        let mut parts: Vec<String> =
            [(modifiers.control, "Ctrl"), (modifiers.alt, "Alt"), (shift, "Shift"), (modifiers.platform, platform)]
                .into_iter()
                .filter(|(on, _)| *on)
                .map(|(_, name)| name.to_owned())
                .collect();
        parts.push(key_name(key, os));
        parts.join("+")
    }
}

fn key_name(key: &str, os: Os) -> String {
    let mac = os == Os::Mac;
    let name = match key {
        "tab" if mac => "⇥",
        "enter" if mac => "↩",
        "backspace" if mac => "⌫",
        "delete" if mac => "⌦",
        "escape" if mac => "⎋",
        "pageup" if mac => "⇞",
        "pagedown" if mac => "⇟",
        "home" if mac => "↖",
        "end" if mac => "↘",
        "left" if mac => "←",
        "right" if mac => "→",
        "up" if mac => "↑",
        "down" if mac => "↓",
        "tab" => "Tab",
        "enter" => "Enter",
        "backspace" => "Backspace",
        "delete" => "Del",
        "insert" => "Ins",
        "escape" => "Esc",
        "pageup" => "PgUp",
        "pagedown" => "PgDn",
        "home" => "Home",
        "end" => "End",
        "left" => "Left",
        "right" => "Right",
        "up" => "Up",
        "down" => "Down",
        "space" => "Space",
        key => return key.to_uppercase(),
    };
    name.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_show_as_each_system_shows_them() {
        assert_eq!(label_on("cmd-shift-t", Os::Mac), "⇧⌘T");
        assert_eq!(label_on("cmd-}", Os::Mac), "⇧⌘]");
        assert_eq!(label_on("ctrl-tab", Os::Mac), "⌃⇥");
        assert_eq!(label_on("cmd-alt-h", Os::Mac), "⌥⌘H");
        assert_eq!(label_on("ctrl-shift-t", Os::Windows), "Ctrl+Shift+T");
        assert_eq!(label_on("ctrl-pagedown", Os::Linux), "Ctrl+PgDn");
        assert_eq!(label_on("ctrl-f4", Os::Windows), "Ctrl+F4");
        assert_eq!(label_on("shift-delete", Os::Windows), "Shift+Del");
        assert_eq!(label_on("f10", Os::Linux), "F10");
    }
}
