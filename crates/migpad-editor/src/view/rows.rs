//! Rows on screen: a line of the document takes one, or several when lines wrap. Scrolling counts
//! lines of the document, its fraction the share of the rows of the line scrolled past.

use std::rc::Rc;

use gpui::{App, Window};
use migpad_core::document::Text;
use migpad_core::text::LineIndex;

use super::EditorView;
use crate::layout::{ScreenLine, TAB_WIDTH};
use crate::wrap;

/// A row on screen: a line of the document, and the row of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct RowAt {
    pub line: usize,
    pub row: usize,
}

impl EditorView {
    /// Whether lines wrap to the width of the view now.
    pub(super) fn wrapping(&self) -> bool {
        self.wrap_cells > 0
    }

    /// Where the rows of `line` start: the start of the line, then a start for each row it wraps
    /// to.
    pub(super) fn row_starts(&self, text: &Text, lines: &LineIndex, line: usize) -> Rc<[usize]> {
        let (range, _) = lines.line_range(text, line);
        if !self.wrapping() {
            return Rc::from([range.start]);
        }
        let mut wraps = self.wraps.borrow_mut();
        let starts = wraps
            .entry(range.start)
            .or_insert_with(|| Rc::from(wrap::row_starts(text, range, self.wrap_cells, TAB_WIDTH)));
        starts.clone()
    }

    fn rows(&self, text: &Text, lines: &LineIndex, line: usize) -> usize {
        if self.wrapping() { self.row_starts(text, lines, line).len() } else { 1 }
    }

    pub(super) fn next_row(&self, text: &Text, lines: &LineIndex, at: RowAt) -> Option<RowAt> {
        if at.row + 1 < self.rows(text, lines, at.line) {
            Some(RowAt { row: at.row + 1, ..at })
        } else {
            (at.line + 1 < lines.count()).then_some(RowAt { line: at.line + 1, row: 0 })
        }
    }

    pub(super) fn prev_row(&self, text: &Text, lines: &LineIndex, at: RowAt) -> Option<RowAt> {
        if at.row > 0 {
            Some(RowAt { row: at.row - 1, ..at })
        } else {
            (at.line > 0).then(|| RowAt { line: at.line - 1, row: self.rows(text, lines, at.line - 1) - 1 })
        }
    }

    /// The row at the top of the view, and how much of it is scrolled past, in rows.
    pub(super) fn top_row(&self, text: &Text, lines: &LineIndex) -> (RowAt, f64) {
        let line = (self.scroll_top as usize).min(lines.count() - 1);
        let rows = self.rows(text, lines, line);
        let into = (self.scroll_top - line as f64).max(0.0) * rows as f64;
        let row = (into as usize).min(rows - 1);
        (RowAt { line, row }, into - row as f64)
    }

    /// The scrolling that puts `at` at the top of the view, `past` of it scrolled past.
    pub(super) fn scroll_top_at(&self, text: &Text, lines: &LineIndex, at: RowAt, past: f64) -> f64 {
        at.line as f64 + (at.row as f64 + past) / self.rows(text, lines, at.line) as f64
    }

    /// How far the view scrolls: until the last row is at the bottom.
    pub(super) fn max_top(&self, text: &Text, lines: &LineIndex) -> f64 {
        let count = lines.count();
        if !self.wrapping() {
            return count.saturating_sub(self.page_lines) as f64;
        }
        let mut needed = self.page_lines as f64;
        for line in (0..count).rev() {
            let rows = self.rows(text, lines, line) as f64;
            if rows >= needed {
                return line as f64 + (rows - needed) / rows;
            }
            needed -= rows;
        }
        0.0
    }

    /// Scrolls so that `top` is the first visible line, as far as the document allows.
    pub(super) fn scroll_to(&mut self, top: f64, cx: &App) {
        let doc = self.document.read(cx);
        self.scroll_top = top.clamp(0.0, self.max_top(doc.text(), doc.lines()));
    }

    /// Scrolls by `rows` rows on screen, down for positive.
    pub(super) fn scroll_rows(&mut self, rows: f64, cx: &App) {
        if !self.wrapping() {
            self.scroll_to(self.scroll_top + rows, cx);
            return;
        }
        let doc = self.document.read(cx);
        let (text, lines) = (doc.text(), doc.lines());
        let (mut at, past) = self.top_row(text, lines);
        let mut into = past + rows;
        while into >= 1.0 {
            match self.next_row(text, lines, at) {
                Some(next) => {
                    at = next;
                    into -= 1.0;
                }
                None => into = into.min(0.999),
            }
        }
        while into < 0.0 {
            match self.prev_row(text, lines, at) {
                Some(prev) => {
                    at = prev;
                    into += 1.0;
                }
                None => into = 0.0,
            }
        }
        let top = self.scroll_top_at(text, lines, at, into);
        self.scroll_to(top, cx);
    }

    /// How many rows `to` is below `from`, if it is no more than `limit` rows.
    pub(super) fn rows_between(
        &self,
        text: &Text,
        lines: &LineIndex,
        from: RowAt,
        to: RowAt,
        limit: usize,
    ) -> Option<usize> {
        if !self.wrapping() {
            return to.line.checked_sub(from.line).filter(|&rows| rows <= limit);
        }
        let mut at = from;
        for rows in 0..=limit {
            if at == to {
                return Some(rows);
            }
            at = self.next_row(text, lines, at)?;
        }
        None
    }

    /// The row `count` rows below `from`, or the last one.
    pub(super) fn row_below(&self, text: &Text, lines: &LineIndex, from: RowAt, count: usize) -> RowAt {
        if !self.wrapping() {
            return RowAt { line: (from.line + count).min(lines.count() - 1), row: 0 };
        }
        let mut at = from;
        for _ in 0..count {
            match self.next_row(text, lines, at) {
                Some(next) => at = next,
                None => break,
            }
        }
        at
    }

    /// The row `count` rows above `from`, or the first one.
    pub(super) fn row_above(&self, text: &Text, lines: &LineIndex, from: RowAt, count: usize) -> RowAt {
        if !self.wrapping() {
            return RowAt { line: from.line.saturating_sub(count), row: 0 };
        }
        let mut at = from;
        for _ in 0..count {
            match self.prev_row(text, lines, at) {
                Some(prev) => at = prev,
                None => break,
            }
        }
        at
    }

    /// The row that the caret is on: at a wrap, the end of the upper row or the start of the
    /// lower one, as `caret_at_row_end` says.
    pub(super) fn caret_row(&self, text: &Text, lines: &LineIndex) -> RowAt {
        let head = self.selection.head;
        let line = lines.line_of(head);
        let starts = self.row_starts(text, lines, line);
        let row = starts.partition_point(|&start| start <= head).saturating_sub(1);
        let row = if self.caret_at_row_end && row > 0 && starts[row] == head { row - 1 } else { row };
        RowAt { line, row }
    }

    /// Lays out a row: when lines wrap, the bytes of the row from its left edge; otherwise the
    /// whole line, or a window of a long one.
    pub(super) fn screen_row(&self, text: &Text, lines: &LineIndex, at: RowAt, window: &Window) -> ScreenLine {
        let style = self.line_style();
        if !self.wrapping() {
            return ScreenLine::new(text, lines, at.line, &style, window);
        }
        let (range, _) = lines.line_range(text, at.line);
        let starts = self.row_starts(text, lines, at.line);
        let end = starts.get(at.row + 1).copied().unwrap_or(range.end);
        ScreenLine::part(text, range, starts[at.row]..end, &style, window)
    }

    /// Whether `at` is the last row of its line.
    pub(super) fn last_row_of_line(&self, text: &Text, lines: &LineIndex, at: RowAt) -> bool {
        at.row + 1 == self.rows(text, lines, at.line)
    }
}
