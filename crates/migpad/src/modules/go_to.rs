//! Go to Line: the command of the Edit menu and the keys of its bar, which is [`crate::go_to`].

use gpui::actions;

use crate::commands::{Command, MenuId, Module, Registry, by_os};
use crate::go_to::{self, CONTEXT};
use crate::strings::Key;

actions!(go_to, [GoToLine, ConfirmGoTo, CloseGoTo]);

pub struct GoToModule;

impl Module for GoToModule {
    fn id(&self) -> &'static str {
        "go_to"
    }

    fn register(&self, registry: &mut Registry) {
        let line =
            Command::new("go_to.line", Key::GoToLine, GoToLine).keys(by_os(&["cmd-l"], &["ctrl-g"], &["ctrl-g"]));
        registry.add(line, Some((MenuId::Edit, 2)));
        registry.add(Command::new("go_to.confirm", Key::GoToLine, ConfirmGoTo).keys(&["enter"]).context(CONTEXT), None);
        registry.add(Command::new("go_to.close", Key::GoToClose, CloseGoTo).keys(&["escape"]).context(CONTEXT), None);

        registry.on_window_action(|workspace, _: &GoToLine, window, cx| go_to::open(workspace, window, cx));
        registry.on_window_action(|workspace, _: &ConfirmGoTo, window, cx| go_to::confirm(workspace, window, cx));
        registry.on_window_action(|workspace, _: &CloseGoTo, window, cx| go_to::close(workspace, window, cx));
    }
}
