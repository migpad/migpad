//! Editing with the keyboard: typing, deleting, line breaks, the clipboard, undo and redo.

use std::borrow::Cow;
use std::ops::Range;
use std::time::Instant;

use gpui::{App, ClipboardItem, Context, Window};
use migpad_core::history::{EditKind, Selection};
use migpad_core::text::TextStore;

use super::EditorView;
use crate::movement::{self, WordStop};

const BLOCK_CLIPBOARD_METADATA: &str = "migpad/block";

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
            self.version = self.document.read(cx).version();
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
        if self.block_columns.is_some() {
            self.type_text_block(text, window, cx);
            return;
        }
        let text = self.accepted(text);
        let range = self.selected_range();
        let after = Selection::caret(range.start + text.len());
        self.replace(range, text.as_bytes(), EditKind::Typing, after, window, cx);
    }

    /// Replaces the whole text with `text`, selected: the owner of an input field fills it. It is an
    /// edit like any other, which undo takes back.
    pub fn set_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.end_composition(cx);
        let text = self.accepted(text);
        let len = self.document.read(cx).text().len();
        let after = Selection { anchor: 0, head: text.len() };
        if !self.replace(0..len, text.as_bytes(), EditKind::Other, after, window, cx) {
            self.select_all(window, cx);
        }
    }

    /// Enter: the line break the document uses most.
    pub(crate) fn newline(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let line_ending = self.document.read(cx).format.line_ending;
        let text = std::str::from_utf8(line_ending.as_bytes()).expect("line breaks are ASCII");
        self.type_text(text, window, cx);
    }

    /// Deletes the selection, or what `deletion` reaches from the caret.
    pub(crate) fn delete(&mut self, deletion: Deletion, window: &mut Window, cx: &mut Context<Self>) {
        if self.block_columns.is_some() {
            self.delete_block(window, cx);
            return;
        }
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
            self.version = self.document.read(cx).version();
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
        if self.block_columns.is_some() {
            self.copy_block(cx);
            return;
        }
        let range = self.selected_range();
        if range.is_empty() {
            return;
        }
        let bytes = self.document.read(cx).text().to_vec(range);
        cx.write_to_clipboard(ClipboardItem::new_string(String::from_utf8_lossy(&bytes).into_owned()));
    }

    pub(crate) fn cut(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.block_columns.is_some() {
            if !self.read_only(cx) {
                self.copy_block(cx);
                self.delete_block(window, cx);
            }
            return;
        }
        let range = self.selected_range();
        if range.is_empty() || self.read_only(cx) {
            return;
        }
        self.copy(cx);
        let after = Selection::caret(range.start);
        self.replace(range, b"", EditKind::Other, after, window, cx);
    }

    /// Pastes the text of the clipboard over the selection, with the line breaks of the document;
    /// in an input field, as one line. Block clipboard text is pasted as a column.
    pub(crate) fn paste(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = cx.read_from_clipboard() else { return };
        let Some(text) = item.text() else { return };
        let is_block = item.metadata().is_some_and(|m| m == BLOCK_CLIPBOARD_METADATA);
        if is_block && !self.single_line {
            self.paste_block(&text, window, cx);
            return;
        }
        if self.block_columns.is_some() {
            self.delete_block(window, cx);
        }
        let bytes = if self.single_line {
            one_line(&text).into_owned().into_bytes()
        } else {
            with_line_ending(&text, self.document.read(cx).format.line_ending.as_bytes())
        };
        let range = self.selected_range();
        let after = Selection::caret(range.start + bytes.len());
        self.replace(range, &bytes, EditKind::Other, after, window, cx);
    }

    /// Types `text` into every line of the block selection at the left column.
    fn type_text_block(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only(cx) {
            return;
        }
        let text = self.accepted(text);
        let ranges = self.block_line_ranges(cx);
        if ranges.is_empty() {
            return;
        }
        let text_bytes = text.as_bytes();
        let mut edits: Vec<(Range<usize>, Vec<u8>)> = Vec::with_capacity(ranges.len());
        let mut shift: isize = 0;
        for range in &ranges {
            let start = (range.start as isize + shift) as usize;
            let end = (range.end as isize + shift) as usize;
            edits.push((start..end, text_bytes.to_vec()));
            shift += text_bytes.len() as isize - range.len() as isize;
        }
        let before = self.selection;
        let last = edits.last().unwrap();
        let after_pos = last.0.start + last.1.len();
        let after = Selection::caret(after_pos);
        let edit_refs: Vec<(Range<usize>, &[u8])> = edits.iter().map(|(r, b)| (r.clone(), b.as_slice())).collect();
        let edited = self.document.update(cx, |doc, cx| {
            let ok = doc.edit(&edit_refs, before, after, EditKind::Other, Instant::now()).is_ok();
            if ok {
                cx.notify();
            }
            ok
        });
        if edited {
            self.version = self.document.read(cx).version();
            self.text_changed();
            self.selection = after;
            // Keep block selection as zero-width at the new column.
            let (anchor_col, head_col) = self.block_columns.unwrap();
            let left = anchor_col.min(head_col);
            let new_col = left + column_width(text_bytes, self.columns.tab_width(), left);
            self.block_columns = Some((new_col, new_col));
            // Recompute anchor position for the new column.
            let doc = self.document.read(cx);
            let (txt, lns) = (doc.text(), doc.lines());
            let al = lns.line_of(before.anchor.min(txt.len()));
            let hl = lns.line_of(after_pos.min(txt.len()));
            let anchor_line = al.min(hl);
            let (ar, _) = lns.line_range(txt, anchor_line);
            let anchor_pos = self.columns.char_at(txt, &ar, new_col).0;
            self.selection = Selection { anchor: anchor_pos, head: after_pos };
            self.goal_x = None;
            self.caret_moved(window, cx);
        }
    }

    /// Deletes the content of every line in the block selection.
    fn delete_block(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only(cx) {
            return;
        }
        let ranges = self.block_line_ranges(cx);
        let has_content = ranges.iter().any(|r| !r.is_empty());
        if !has_content {
            return;
        }
        let mut edits: Vec<(Range<usize>, &[u8])> = Vec::with_capacity(ranges.len());
        let mut shift: isize = 0;
        for range in &ranges {
            if !range.is_empty() {
                let start = (range.start as isize + shift) as usize;
                let end = (range.end as isize + shift) as usize;
                edits.push((start..end, b""));
                shift -= range.len() as isize;
            }
        }
        if edits.is_empty() {
            return;
        }
        let before = self.selection;
        let after_pos = edits[0].0.start;
        let after = Selection::caret(after_pos);
        let edited = self.document.update(cx, |doc, cx| {
            let ok = doc.edit(&edits, before, after, EditKind::Other, Instant::now()).is_ok();
            if ok {
                cx.notify();
            }
            ok
        });
        if edited {
            self.version = self.document.read(cx).version();
            self.text_changed();
            self.selection = after;
            // Collapse to zero-width block at the left column.
            let (anchor_col, head_col) = self.block_columns.unwrap();
            let left = anchor_col.min(head_col);
            self.block_columns = Some((left, left));
            // Recompute positions for the new column.
            let doc = self.document.read(cx);
            let (txt, lns) = (doc.text(), doc.lines());
            let al = lns.line_of(before.anchor.min(txt.len()));
            let hl = lns.line_of(before.head.min(txt.len()));
            let (start_line, end_line) = (al.min(hl), al.max(hl));
            let (ar, _) = lns.line_range(txt, start_line);
            let anchor_pos = self.columns.char_at(txt, &ar, left).0;
            let (hr, _) = lns.line_range(txt, end_line.min(lns.count() - 1));
            let head_pos = self.columns.char_at(txt, &hr, left).0;
            self.selection = Selection { anchor: anchor_pos, head: head_pos };
            self.goal_x = None;
            self.caret_moved(window, cx);
        }
    }

    /// Copies the block selection to the clipboard: each line's content separated by newlines,
    /// with metadata marking it as a block.
    fn copy_block(&mut self, cx: &mut Context<Self>) {
        let ranges = self.block_line_ranges(cx);
        let text = self.document.read(cx).text();
        let mut out = String::new();
        for (i, range) in ranges.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            let bytes = text.to_vec(range.clone());
            out.push_str(&String::from_utf8_lossy(&bytes));
        }
        let item = ClipboardItem::new_string_with_metadata(out, BLOCK_CLIPBOARD_METADATA.to_owned());
        cx.write_to_clipboard(item);
    }

    /// Pastes block text as a column at the caret position.
    fn paste_block(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only(cx) {
            return;
        }
        // Delete any existing block selection first.
        if self.block_columns.is_some() {
            let (ac, hc) = self.block_columns.unwrap();
            if ac != hc {
                self.delete_block(window, cx);
            }
        }
        let paste_lines: Vec<&str> = text.lines().collect();
        if paste_lines.is_empty() {
            return;
        }
        let edits = {
            let doc = self.document.read(cx);
            let (txt, lns) = (doc.text(), doc.lines());
            let line_ending = doc.format.line_ending.as_bytes();
            let head = self.selection.head.min(txt.len());
            let head_line = lns.line_of(head);
            let (head_range, _) = lns.line_range(txt, head_line);
            let col = self.columns.column_of(txt, &head_range, head);
            let mut edits: Vec<(Range<usize>, Vec<u8>)> = Vec::new();
            let mut shift: isize = 0;
            for (i, &paste_line) in paste_lines.iter().enumerate() {
                let line = head_line + i;
                if line < lns.count() {
                    let (range, _) = lns.line_range(txt, line);
                    let pos = self.columns.char_at(txt, &range, col).0;
                    let adj = (pos as isize + shift) as usize;
                    edits.push((adj..adj, paste_line.as_bytes().to_vec()));
                    shift += paste_line.len() as isize;
                } else {
                    let end = (txt.len() as isize + shift) as usize;
                    let mut bytes = Vec::from(line_ending);
                    bytes.extend_from_slice(paste_line.as_bytes());
                    shift += bytes.len() as isize;
                    edits.push((end..end, bytes));
                }
            }
            edits
        };
        if edits.is_empty() {
            return;
        }
        let before = self.selection;
        let last = edits.last().unwrap();
        let after_pos = last.0.start + last.1.len();
        let after = Selection::caret(after_pos);
        let edit_refs: Vec<(Range<usize>, &[u8])> = edits.iter().map(|(r, b)| (r.clone(), b.as_slice())).collect();
        let edited = self.document.update(cx, |doc, cx| {
            let ok = doc.edit(&edit_refs, before, after, EditKind::Other, Instant::now()).is_ok();
            if ok {
                cx.notify();
            }
            ok
        });
        if edited {
            self.version = self.document.read(cx).version();
            self.text_changed();
            self.selection = after;
            self.block_columns = None;
            self.goal_x = None;
            self.caret_moved(window, cx);
        }
    }
}

/// Display columns `bytes` takes when starting at `start_col`, with tabs of `tab_width`.
fn column_width(bytes: &[u8], tab_width: usize, start_col: usize) -> usize {
    let mut col = start_col;
    for chunk in bytes.utf8_chunks() {
        for c in chunk.valid().chars() {
            col += if c == '\t' { tab_width - col % tab_width } else { 1 };
        }
        if !chunk.invalid().is_empty() {
            col += chunk.invalid().len();
        }
    }
    col - start_col
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
