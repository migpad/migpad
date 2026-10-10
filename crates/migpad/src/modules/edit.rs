//! Editing: undo and redo, the clipboard, deleting, selecting all. The editor handles these
//! actions and binds their keys; the module puts them into the Edit menu, as the actions of the
//! system they are, so that the items work in the text fields of the system too. The context menu
//! of the text has them too.

use gpui::OsAction;
use migpad_editor::actions::{Copy, Cut, Delete, LowerCase, Paste, Redo, SelectAll, Undo, UpperCase};

use crate::commands::{Command, MenuId, Module, Registry, by_os};
use crate::strings::Key;

pub struct EditModule;

impl Module for EditModule {
    fn id(&self) -> &'static str {
        "edit"
    }

    fn register(&self, registry: &mut Registry) {
        let menu = |group| Some((MenuId::Edit, group));
        let undo = Command::new("edit.undo", Key::EditUndo, Undo)
            .os_action(OsAction::Undo)
            .enabled(|workspace, cx| workspace.document().read(cx).can_undo());
        registry.add(undo, menu(0));
        let redo = Command::new("edit.redo", Key::EditRedo, Redo)
            .os_action(OsAction::Redo)
            .enabled(|workspace, cx| workspace.document().read(cx).can_redo());
        registry.add(redo, menu(0));
        registry.add(Command::new("edit.cut", Key::EditCut, Cut).os_action(OsAction::Cut), menu(1));
        registry.add(Command::new("edit.copy", Key::EditCopy, Copy).os_action(OsAction::Copy), menu(1));
        registry.add(Command::new("edit.paste", Key::EditPaste, Paste).os_action(OsAction::Paste), menu(1));
        registry.add(Command::new("edit.delete", Key::EditDelete, Delete), menu(1));
        let select_all = Command::new("edit.select_all", Key::EditSelectAll, SelectAll).os_action(OsAction::SelectAll);
        registry.add(select_all, menu(1));
        let upper = Command::new("edit.upper_case", Key::EditUpperCase, UpperCase)
            .keys(by_os(&["cmd-shift-u"], &["ctrl-shift-u"], &["ctrl-shift-u"]))
            .enabled(|workspace, cx| {
                let editor = workspace.editor();
                let sel = editor.read(cx).selection();
                sel.anchor != sel.head || editor.read(cx).is_block_selection()
            });
        registry.add(upper, menu(2));
        let lower = Command::new("edit.lower_case", Key::EditLowerCase, LowerCase)
            .keys(by_os(&["cmd-shift-l"], &["ctrl-shift-l"], &["ctrl-shift-l"]))
            .enabled(|workspace, cx| {
                let editor = workspace.editor();
                let sel = editor.read(cx).selection();
                sel.anchor != sel.head || editor.read(cx).is_block_selection()
            });
        registry.add(lower, menu(2));
    }
}
