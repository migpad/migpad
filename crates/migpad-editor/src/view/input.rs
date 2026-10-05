//! Text input from the system: typed text and what input methods compose. Input methods see the
//! line of the caret, or a window of a long line, and count UTF-16 units from its start: on a
//! document of gigabytes, offsets from the start of the text would take too long to count.

use std::ops::Range;

use gpui::{App, Bounds, Context, EntityInputHandler, Pixels, Point, UTF16Selection, Window, point, px};
use migpad_core::history::{EditKind, Selection};
use migpad_core::text::TextStore;

use super::EditorView;
use super::rows::RowAt;
use crate::utf16::{from_utf16, to_utf16};

/// Of a line longer than this, input methods see a window around the caret.
const MAX_IME_LINE: usize = 64 << 10;
/// Bytes of that window on each side of the caret.
const IME_WINDOW: usize = 4 << 10;

impl EditorView {
    /// The text input methods see, and where it starts: the line of the composition or of the
    /// caret, or a window around them in a long line.
    fn ime_text(&self, cx: &App) -> (usize, Vec<u8>) {
        let doc = self.document.read(cx);
        let (text, lines) = (doc.text(), doc.lines());
        let at = self.marked.as_ref().map_or(self.selection.head, |marked| marked.start);
        let (mut range, _) = lines.line_range(text, lines.line_of(at));
        if range.len() > MAX_IME_LINE {
            let start = text.floor_char_boundary(at.saturating_sub(IME_WINDOW).max(range.start), range.start);
            range = start..text.floor_char_boundary((at + IME_WINDOW).min(range.end), start);
        }
        (range.start, text.to_vec(range))
    }

    /// The bytes of a range of UTF-16 offsets in the text input methods see.
    fn ime_range_bytes(&self, range: &Range<usize>, cx: &App) -> Range<usize> {
        let (start, bytes) = self.ime_text(cx);
        let from = from_utf16(&bytes, range.start);
        start + from..start + from_utf16(&bytes, range.end).max(from)
    }

    /// What input methods replace: the range they give, or the text being composed, or the
    /// selection.
    fn ime_target(&self, range_utf16: Option<&Range<usize>>, cx: &App) -> Range<usize> {
        match (range_utf16, &self.marked) {
            (Some(range), _) => self.ime_range_bytes(range, cx),
            (None, Some(marked)) => marked.clone(),
            (None, None) => self.selected_range(),
        }
    }
}

impl EntityInputHandler for EditorView {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let (_, bytes) = self.ime_text(cx);
        let start = from_utf16(&bytes, range_utf16.start);
        let end = from_utf16(&bytes, range_utf16.end).max(start);
        *adjusted_range = Some(to_utf16(&bytes, start)..to_utf16(&bytes, end));
        Some(String::from_utf8_lossy(&bytes[start..end]).into_owned())
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, cx: &mut Context<Self>) -> Option<UTF16Selection> {
        let (start, bytes) = self.ime_text(cx);
        let end = start + bytes.len();
        let to_ime = |pos: usize| to_utf16(&bytes, pos.clamp(start, end) - start);
        let Selection { anchor, head } = self.selection;
        let seen = |pos: usize| (start..=end).contains(&pos);
        // A selection beyond what input methods see shows them the caret alone.
        let (range, reversed) = if seen(anchor) && seen(head) {
            (to_ime(anchor.min(head))..to_ime(anchor.max(head)), head < anchor)
        } else {
            (to_ime(head)..to_ime(head), false)
        };
        Some(UTF16Selection { range, reversed })
    }

    fn marked_text_range(&self, _: &mut Window, cx: &mut Context<Self>) -> Option<Range<usize>> {
        let marked = self.marked.clone()?;
        let (start, bytes) = self.ime_text(cx);
        Some(to_utf16(&bytes, marked.start - start)..to_utf16(&bytes, marked.end - start))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.end_composition(cx);
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = self.accepted(text);
        let range = self.ime_target(range_utf16.as_ref(), cx);
        // Text committed by an input method ends its composition, in the same undo step.
        let kind = if self.marked.is_some() { EditKind::Composing } else { EditKind::Typing };
        let after = Selection::caret(range.start + text.len());
        self.replace(range, text.as_bytes(), kind, after, window, cx);
        self.end_composition(cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only(cx) {
            return;
        }
        let range = self.ime_target(range_utf16.as_ref(), cx);
        if self.marked.is_none() {
            // A composition is an undo step of its own.
            self.seal_undo_step(cx);
        }
        let start = range.start;
        let joined = self.accepted(new_text);
        // Where the input method puts the selection in its text; in an input field that took the
        // text as a shorter line, at its end.
        let selected = match new_selected_range {
            Some(selected) if joined.len() == new_text.len() => {
                from_utf16(joined.as_bytes(), selected.start)..from_utf16(joined.as_bytes(), selected.end)
            }
            _ => joined.len()..joined.len(),
        };
        let new_text = &*joined;
        let after = Selection { anchor: start + selected.start, head: start + selected.end };
        if !self.replace(range, new_text.as_bytes(), EditKind::Composing, after, window, cx) {
            self.selection = after;
        }
        if new_text.is_empty() {
            self.end_composition(cx);
        } else {
            self.marked = Some(start..start + new_text.len());
        }
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        _: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = self.ime_range_bytes(&range_utf16, cx);
        let doc = self.document.read(cx);
        let (text, lines) = (doc.text(), doc.lines());
        // The row of the start of the range, and how far it is below the top of the view: where it
        // is now, not in the last frame, as input methods ask as soon as they change it.
        let line = lines.line_of(range.start);
        let starts = self.row_starts(text, lines, line);
        let at = RowAt { line, row: starts.partition_point(|&start| start <= range.start).saturating_sub(1) };
        let (top, past) = self.top_row(text, lines);
        let below = self.rows_between(text, lines, top, at, self.page_lines + 1).map_or(0.0, |rows| rows as f64 - past);
        let row = self.screen_row(text, lines, at, window);
        let line_height = self.metrics.line_height;
        let top = self.geometry.text_area.top() + px((below * f64::from(line_height)) as f32);
        let x = |pos| self.geometry.text_left + px((row.x_of(pos) - self.scroll_x) as f32);
        let left = x(range.start);
        let right = x(range.end).max(left + px(1.));
        Some(Bounds::from_corners(point(left, top), point(right, top + line_height)))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<usize> {
        let (at, x) = self.hit(point, cx);
        let (start, bytes) = self.ime_text(cx);
        let doc = self.document.read(cx);
        if doc.lines().line_of(start) != at.line {
            return None;
        }
        let pos = self.screen_row(doc.text(), doc.lines(), at, window).boundary_at(x);
        (start..=start + bytes.len()).contains(&pos).then(|| to_utf16(&bytes, pos - start))
    }

    fn accepts_text_input(&self, _: &mut Window, cx: &mut Context<Self>) -> bool {
        !self.read_only(cx)
    }
}
