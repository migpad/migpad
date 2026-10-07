//! Go to Line: a field over the text that takes a line, or a line and a column — "120:15", as
//! compilers write them — and puts the caret there, the line in the middle of the view if it was
//! out of it. Columns are counted as the status bar counts them.

use gpui::{
    App, AppContext, Context, Entity, Focusable, InteractiveElement, IntoElement, Render, SharedString, Subscription,
    WeakEntity, Window, div, prelude::*, px, rgb,
};
use migpad_core::history::Selection;
use migpad_editor::EditorView;
use migpad_ui::{Button, TextField, theme};

use crate::find;
use crate::keys;
use crate::modules::go_to::CloseGoTo;
use crate::strings::{Key, fill, number, tr};
use crate::workspace::Workspace;

/// The key context of the bar: Enter goes, Esc closes it.
pub const CONTEXT: &str = "GoToBar";

/// The bar of a window: the field of the place to go to.
pub struct GoToBar {
    workspace: WeakEntity<Workspace>,
    field: Entity<EditorView>,
    /// Whether the input was not a place, which the bar tells until it is typed anew.
    invalid: bool,
    _subscriptions: Vec<Subscription>,
}

impl GoToBar {
    pub fn new(workspace: WeakEntity<Workspace>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let field = cx.new(|cx| EditorView::single_line(window, cx));
        let input = field.read(cx).document().clone();
        let typed = cx.observe(&input, |bar: &mut GoToBar, _, cx| {
            if std::mem::take(&mut bar.invalid) {
                cx.notify();
            }
        });
        let menu = find::context_menu_of(&field, &workspace, window, cx);
        GoToBar { workspace, field, invalid: false, _subscriptions: vec![typed, menu] }
    }

    /// Shows the bar for a document of `lines` lines: the field takes the focus, all its text
    /// selected.
    pub fn open(&mut self, lines: usize, window: &mut Window, cx: &mut Context<Self>) {
        let hint = fill(Key::GoToPlaceholder, &[("count", &number(lines as u64))]);
        self.field.update(cx, |field, cx| {
            field.set_placeholder(hint, cx);
            field.select_all(window, cx);
        });
        window.focus(&self.field.focus_handle(cx), cx);
        cx.notify();
    }

    /// The field, if it has the focus.
    pub fn focused_field(&self, window: &Window, cx: &App) -> Option<Entity<EditorView>> {
        self.field.focus_handle(cx).is_focused(window).then(|| self.field.clone())
    }
}

/// A place as typed: a line, and a column after a colon or a comma, both from 1 — "120",
/// "120:15", " 120 , 15 ".
pub fn parse(input: &str) -> Option<(usize, Option<usize>)> {
    let input = input.trim();
    let (line, column) = match input.split_once([':', ',']) {
        Some((line, column)) => (line, Some(column)),
        None => (input, None),
    };
    let number = |text: &str| {
        let text = text.trim();
        let digits = !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit());
        digits.then(|| text.parse::<usize>().ok()).flatten().filter(|&n| n > 0)
    };
    let column = match column {
        Some(column) => Some(number(column)?),
        None => None,
    };
    Some((number(line)?, column))
}

/// Go to Line…: shows the bar of the window.
pub fn open(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let lines = workspace.document().read(cx).lines().count();
    let bar = workspace.show_go_to_bar(window, cx);
    bar.update(cx, |bar, cx| bar.open(lines, window, cx));
}

/// Enter in the bar: the caret goes to the place typed, and the bar closes; what is not a place
/// is told, and the bar stays.
pub fn confirm(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let Some(bar) = workspace.go_to_bar_shown().cloned() else { return };
    let input = bar.read(cx).field.read(cx).text(cx);
    let Some((line, column)) = parse(&input) else {
        bar.update(cx, |bar, cx| {
            bar.invalid = true;
            cx.notify();
        });
        return;
    };
    go_to(workspace, line - 1, column.map_or(0, |column| column - 1), window, cx);
    workspace.hide_go_to_bar(cx);
    window.focus(&workspace.editor().focus_handle(cx), cx);
}

/// Puts the caret of the active document at `column` of `line`, both from zero, as the status
/// bar counts columns: the line in the middle of the view if it was out of view.
pub fn go_to(workspace: &mut Workspace, line: usize, column: usize, window: &mut Window, cx: &mut Context<Workspace>) {
    let editor = workspace.editor().clone();
    let pos = editor.read(cx).position_at(line, column, cx);
    editor.update(cx, |editor, cx| editor.select_centered(Selection::caret(pos), window, cx));
}

/// Esc in the bar: it closes, the focus goes back to the text.
pub fn close(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    workspace.hide_go_to_bar(cx);
    window.focus(&workspace.editor().focus_handle(cx), cx);
}

impl Render for GoToBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme(cx);
        let workspace = self.workspace.clone();
        let keys = keys::for_action(&CloseGoTo, &self.field.focus_handle(cx), window).map(SharedString::from);
        let status = self.invalid.then(|| {
            div()
                .flex_none()
                .pl(px(4.))
                .whitespace_nowrap()
                .text_color(rgb(theme.error_edge))
                .child(tr(Key::GoToInvalid))
        });
        div()
            .id("go-to-bar")
            .key_context(CONTEXT)
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.))
            .px(px(8.))
            .py(px(5.))
            .bg(rgb(theme.bar))
            .border_b_1()
            .border_color(rgb(theme.border))
            .child(div().flex_shrink(1.).w(px(300.)).min_w(px(100.)).child(TextField::new(self.field.clone())))
            .children(status)
            .child(div().flex_1())
            .child(Button::new("go-to-close", "×").name(tr(Key::GoToClose)).tooltip(tr(Key::GoToClose), keys).on_click(
                move |_, window, cx| {
                    let _ = workspace.update(cx, |workspace, cx| close(workspace, window, cx));
                },
            ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn places_are_a_line_and_maybe_a_column() {
        assert_eq!(parse("120"), Some((120, None)));
        assert_eq!(parse("120:15"), Some((120, Some(15))));
        assert_eq!(parse(" 120 , 15 "), Some((120, Some(15))));
        assert_eq!(parse("7:"), None);
        assert_eq!(parse(":7"), None);
        assert_eq!(parse("0"), None);
        assert_eq!(parse("1:0"), None);
        assert_eq!(parse("+5"), None);
        assert_eq!(parse("строка"), None);
        assert_eq!(parse(""), None);
        assert_eq!(parse("1:2:3"), None);
        assert_eq!(parse("99999999999999999999999"), None);
    }
}
