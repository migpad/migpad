//! Find and replace: the commands of the Edit menu — in its Find submenu on macOS, as there — and
//! the keys of the find bar. The bar itself is [`crate::find`].

use gpui::actions;

use crate::commands::{Command, MenuId, Module, Registry, by_os};
use crate::find::{self, CONTEXT, QueryOption, REPLACE_CONTEXT};
use crate::strings::Key;

actions!(
    find,
    [
        Find,
        Replace,
        FindNext,
        FindPrevious,
        ReplaceOne,
        ReplaceAll,
        CloseFind,
        ToggleMatchCase,
        ToggleWholeWord,
        ToggleRegex,
        FocusNextField,
        FocusPreviousField
    ]
);

pub struct FindModule;

impl Module for FindModule {
    fn id(&self) -> &'static str {
        "find"
    }

    fn register(&self, registry: &mut Registry) {
        let find = Command::new("find.find", Key::FindFind, Find).keys(by_os(&["cmd-f"], &["ctrl-f"], &["ctrl-f"]));
        let replace_label = if cfg!(target_os = "macos") { Key::FindFindAndReplace } else { Key::FindReplace };
        let replace =
            Command::new("find.replace", replace_label, Replace).keys(by_os(&["cmd-alt-f"], &["ctrl-h"], &["ctrl-h"]));
        let next = Command::new("find.next", Key::FindNext, FindNext).keys(by_os(&["cmd-g"], &["f3"], &["f3"]));
        let previous = Command::new("find.previous", Key::FindPrevious, FindPrevious).keys(by_os(
            &["cmd-shift-g"],
            &["shift-f3"],
            &["shift-f3"],
        ));
        let commands = vec![find, replace, next, previous];
        if cfg!(target_os = "macos") {
            registry.add_group(Key::FindMenu, commands, MenuId::Edit, 2);
        } else {
            for command in commands {
                registry.add(command, Some((MenuId::Edit, 2)));
            }
        }

        // The keys of the bar: Enter finds, Shift+Enter finds back, Enter in the field of the
        // replacement replaces, Esc closes the bar.
        let bar = [
            Command::new("find.next_in_bar", Key::FindNext, FindNext).keys(&["enter"]),
            Command::new("find.previous_in_bar", Key::FindPrevious, FindPrevious).keys(&["shift-enter"]),
            Command::new("find.replace_all", Key::FindReplaceAll, ReplaceAll).keys(by_os(
                &["cmd-enter"],
                &["ctrl-alt-enter"],
                &["ctrl-alt-enter"],
            )),
            Command::new("find.close", Key::FindClose, CloseFind).keys(&["escape"]),
            Command::new("find.match_case", Key::FindMatchCase, ToggleMatchCase).keys(by_os(
                &["cmd-alt-c"],
                &["alt-c"],
                &["alt-c"],
            )),
            Command::new("find.whole_word", Key::FindWholeWord, ToggleWholeWord).keys(by_os(
                &["cmd-alt-w"],
                &["alt-w"],
                &["alt-w"],
            )),
            Command::new("find.regex", Key::FindRegex, ToggleRegex).keys(by_os(&["cmd-alt-r"], &["alt-r"], &["alt-r"])),
            Command::new("find.focus_next", Key::FindFind, FocusNextField).keys(&["tab"]),
            Command::new("find.focus_previous", Key::FindFind, FocusPreviousField).keys(&["shift-tab"]),
        ];
        for command in bar {
            registry.add(command.context(CONTEXT), None);
        }
        let replace_one = Command::new("find.replace_one", Key::FindReplaceOne, ReplaceOne).keys(&["enter"]);
        registry.add(replace_one.context(REPLACE_CONTEXT), None);

        registry.on_window_action(|workspace, _: &Find, window, cx| find::open(workspace, false, window, cx));
        registry.on_window_action(|workspace, _: &Replace, window, cx| find::open(workspace, true, window, cx));
        registry.on_window_action(|workspace, _: &FindNext, window, cx| find::find(workspace, false, window, cx));
        registry.on_window_action(|workspace, _: &FindPrevious, window, cx| find::find(workspace, true, window, cx));
        registry.on_window_action(|workspace, _: &ReplaceOne, window, cx| find::replace_one(workspace, window, cx));
        registry.on_window_action(|workspace, _: &ReplaceAll, window, cx| find::replace_all(workspace, window, cx));
        registry.on_window_action(|workspace, _: &CloseFind, window, cx| find::close(workspace, window, cx));
        registry.on_window_action(|_, _: &ToggleMatchCase, _, cx| find::toggle(QueryOption::MatchCase, cx));
        registry.on_window_action(|_, _: &ToggleWholeWord, _, cx| find::toggle(QueryOption::WholeWord, cx));
        registry.on_window_action(|_, _: &ToggleRegex, _, cx| find::toggle(QueryOption::Regex, cx));
        registry.on_window_action(|_, _: &FocusNextField, window, cx| window.focus_next(cx));
        registry.on_window_action(|_, _: &FocusPreviousField, window, cx| window.focus_prev(cx));
    }
}
