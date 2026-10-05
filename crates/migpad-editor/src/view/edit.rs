//! Editing with the keyboard: typing, deleting, line breaks, the clipboard, undo and redo.

use std::borrow::Cow;
use std::ops::Range;
use std::time::Instant;

use gpui::{App, ClipboardItem, Context, Window};
use migpad_core::history::{EditKind, Selection};
use migpad_core::text::TextStore;

use super::EditorView;
use crate::movement::{self, WordStop};

/// What Backspace, Delete and their variants delete when nothing is selected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Deletion {
    CharLeft,
    CharRight,
    WordLeft,
    WordRight,
    /// Back to the start of the line; at its start, the line break before it.
    LineStart,
}

impl EditorView {
    /// Whether the document takes no edits: the preview of a file that is still loading.
    pub(super) fn read_only(&self, cx: &App) -> bool {
        self.document.read(cx).is_preview()
    }

    /// Replaces `range` with `text` as an edit of `kind`, and puts the selection at `after` in the
    /// text that results. Returns whether the text changed: a preview takes no edits.
    pub(super) fn replace(
        &mut self,
        range: Range<usize>,
        text: &[u8],
        kind: EditKind,
        after: Selection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if range.is_empty() && text.is_empty() {
            return false;
        }
        let before = self.selection;
        let edited = self.document.update(cx, |doc, cx| {
            let edited = doc.edit(&[(range, text)], before, after, kind, Instant::now()).is_ok();
            if edited {
                cx.notify();
            }
            edited
        });
        if edited {
            // Columns and wraps of the edited line have moved; the rest are found again as needed.
            self.text_changed();
            self.selection = after;
            self.goal_x = None;
            self.caret_moved(window, cx);
        }
        edited
    }

    /// Text from the keyboard, an input method or the clipboard as the view takes it: an input
    /// field makes it one line.
    pub(super) fn accepted<'a>(&self, text: &'a str) -> Cow<'a, str> {
        if self.single_line { one_line(text) } else { Cow::Borrowed(text) }
    }

    /// Types `text` over the selection.
    pub(crate) fn type_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.accepted(text);
        let range = self.selected_range();
        let after = Selection::caret(range.start + text.len());
        self.replace(range, text.as_bytes(), EditKind::Typing, after, window, cx);
    }

    /// Enter: the line break the document uses most.
    pub(crate) fn newline(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let line_ending = self.document.read(cx).format.line_ending;
        let text = std::str::from_utf8(line_ending.as_bytes()).expect("line breaks are ASCII");
        self.type_text(text, window, cx);
    }

    /// Deletes the selection, or what `deletion` reaches from the caret.
    pub(crate) fn delete(&mut self, deletion: Deletion, window: &mut Window, cx: &mut Context<Self>) {
        let mut range = self.selected_range();
        if range.is_empty() {
            let doc = self.document.read(cx);
            let (text, lines) = (doc.text(), doc.lines());
            let head = self.selection.head;
            range = match deletion {
                Deletion::CharLeft => movement::prev_char(text, lines, head)..head,
                Deletion::CharRight => head..movement::next_char(text, lines, head),
                Deletion::WordLeft => movement::word_left(text, lines, head)..head,
                Deletion::WordRight => head..movement::word_right(text, lines, head, WordStop::PLATFORM),
                Deletion::LineStart => match lines.start(lines.line_of(head)) {
                    start if start < head => start..head,
                    _ => movement::prev_char(text, lines, head)..head,
                },
            };
        }
        let after = Selection::caret(range.start);
        self.replace(range, b"", EditKind::Deleting, after, window, cx);
    }

    pub(crate) fn undo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.step_history(true, window, cx);
    }

    pub(crate) fn redo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.step_history(false, window, cx);
    }

    /// Undoes the last step, or redoes the last undone one, and restores the selection it had.
    fn step_history(&mut self, undo: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.end_composition(cx);
        let selection = self.document.update(cx, |doc, cx| {
            let selection = if undo { doc.undo() } else { doc.redo() };
            if selection.is_some() {
                cx.notify();
            }
            selection
        });
        if let Some(selection) = selection {
            self.text_changed();
            let doc = self.document.read(cx);
            let snap = |pos| movement::snap(doc.text(), doc.lines(), pos);
            self.selection = Selection { anchor: snap(selection.anchor), head: snap(selection.head) };
            self.goal_x = None;
            self.caret_moved(window, cx);
        }
    }

    /// Copies the selected text; invalid UTF-8 becomes U+FFFD, as it shows.
    pub(crate) fn copy(&mut self, cx: &mut Context<Self>) {
        let range = self.selected_range();
        if range.is_empty() {
            return;
        }
        let bytes = self.document.read(cx).text().to_vec(range);
        cx.write_to_clipboard(ClipboardItem::new_string(String::from_utf8_lossy(&bytes).into_owned()));
    }

    pub(crate) fn cut(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let range = self.selected_range();
        if range.is_empty() || self.read_only(cx) {
            return;
        }
        self.copy(cx);
        let after = Selection::caret(range.start);
        self.replace(range, b"", EditKind::Other, after, window, cx);
    }

    /// Pastes the text of the clipboard over the selection, with the line breaks of the document;
    /// in an input field, as one line.
    pub(crate) fn paste(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else { return };
        let bytes = if self.single_line {
            one_line(&text).into_owned().into_bytes()
        } else {
            with_line_ending(&text, self.document.read(cx).format.line_ending.as_bytes())
        };
        let range = self.selected_range();
        let after = Selection::caret(range.start + bytes.len());
        self.replace(range, &bytes, EditKind::Other, after, window, cx);
    }
}

/// `text` with each of its line breaks, LF, CRLF or CR, made `line_ending`.
fn with_line_ending(text: &str, line_ending: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len());
    let mut rest = text.as_bytes();
    while let Some(i) = memchr::memchr2(b'\r', b'\n', rest) {
        out.extend_from_slice(&rest[..i]);
        out.extend_from_slice(line_ending);
        let crlf = rest[i] == b'\r' && rest.get(i + 1) == Some(&b'\n');
        rest = &rest[i + if crlf { 2 } else { 1 }..];
    }
    out.extend_from_slice(rest);
    out
}

/// `text` as one line: line breaks at its ends are dropped, and each one inside — LF, CRLF or CR —
/// becomes a space. A line copied with its line break comes without it.
fn one_line(text: &str) -> Cow<'_, str> {
    let text = text.trim_matches(['\r', '\n']);
    if !text.contains(['\r', '\n']) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find(['\r', '\n']) {
        out.push_str(&rest[..i]);
        out.push(' ');
        let crlf = rest[i..].starts_with("\r\n");
        rest = &rest[i + if crlf { 2 } else { 1 }..];
    }
    out.push_str(rest);
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pasted_line_breaks_become_those_of_the_document() {
        let text = "a\nb\r\nc\rd\r";
        assert_eq!(with_line_ending(text, b"\r\n"), b"a\r\nb\r\nc\r\nd\r\n");
        assert_eq!(with_line_ending(text, b"\n"), b"a\nb\nc\nd\n");
        assert_eq!(with_line_ending("без переводов", b"\r"), "без переводов".as_bytes());
    }

    #[test]
    fn input_fields_take_text_as_one_line() {
        assert!(matches!(one_line("одна строка"), Cow::Borrowed("одна строка")));
        assert_eq!(one_line("one\ntwo\r\nthree\rfour"), "one two three four");
        assert_eq!(one_line("\r\n  line\t\n"), "  line\t");
        assert_eq!(one_line("a\n\nb\r\n\r\nc\r\rd"), "a  b  c  d");
        assert_eq!(one_line("\n\r\n\r"), "");
    }
}
