//! Moving the caret and the selection with the keyboard, and keeping the caret in view.

use gpui::{App, Context, Window};
use migpad_core::history::Selection;
use migpad_core::text::TextStore;

use super::EditorView;
use crate::layout::ScreenLine;
use crate::movement::{self, WordStop};

/// Where a key moves the caret.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Motion {
    Left,
    Right,
    Up,
    Down,
    WordLeft,
    WordRight,
    LineStart,
    LineEnd,
    PageUp,
    PageDown,
    DocStart,
    DocEnd,
}

/// Characters kept between the caret and the left and right edges of the view.
const MARGIN: f64 = 4.0;

impl EditorView {
    /// Moves the caret; with `select`, the selection follows it from its anchor.
    pub(crate) fn move_caret(&mut self, motion: Motion, select: bool, window: &mut Window, cx: &mut Context<Self>) {
        let page = self.page_lines;
        if let Motion::PageUp | Motion::PageDown = motion {
            // The text scrolls by a page, and the caret moves by as many rows.
            let by = if motion == Motion::PageUp { -(page as f64) } else { page as f64 };
            self.scroll_rows(by, cx);
        }
        let doc = self.document.read(cx);
        let (text, lines) = (doc.text(), doc.lines());
        let Selection { anchor, head } = self.selection;
        let (start, end) = (anchor.min(head), anchor.max(head));
        // Left and right without Shift put the caret at the edge of the selection.
        let collapse = !select && start != end;
        let mut goal_x = None;
        let mut at_row_end = false;
        let pos = match motion {
            Motion::Left if collapse => start,
            Motion::Right if collapse => end,
            Motion::Left => movement::prev_char(text, lines, head),
            Motion::Right => movement::next_char(text, lines, head),
            Motion::WordLeft => movement::word_left(text, lines, head),
            Motion::WordRight => movement::word_right(text, lines, head, WordStop::PLATFORM),
            // When lines wrap, Home and End go to the edges of the row on screen.
            Motion::LineStart => {
                let at = self.caret_row(text, lines);
                self.row_starts(text, lines, at.line)[at.row]
            }
            Motion::LineEnd => {
                let at = self.caret_row(text, lines);
                match self.row_starts(text, lines, at.line).get(at.row + 1) {
                    Some(&next) => {
                        at_row_end = true;
                        next
                    }
                    None => movement::line_end(text, lines, head),
                }
            }
            Motion::DocStart => 0,
            Motion::DocEnd => text.len(),
            Motion::Up | Motion::Down | Motion::PageUp | Motion::PageDown => {
                let from = self.caret_row(text, lines);
                let x = self.goal_x.unwrap_or_else(|| self.screen_row(text, lines, from, window).x_of(head));
                goal_x = Some(x);
                let by = if let Motion::Up | Motion::Down = motion { 1 } else { page };
                let target = match motion {
                    Motion::Up | Motion::PageUp => self.row_above(text, lines, from, by),
                    _ => self.row_below(text, lines, from, by),
                };
                // Up on the first row goes to the start of the text, down on the last to the end.
                if target == from {
                    if let Motion::Up | Motion::PageUp = motion { 0 } else { text.len() }
                } else {
                    let row = self.screen_row(text, lines, target, window);
                    let pos = row.boundary_at(x);
                    at_row_end = pos == row.shown.end && !self.last_row_of_line(text, lines, target);
                    pos
                }
            }
        };
        self.selection = if select { Selection { anchor, head: pos } } else { Selection::caret(pos) };
        self.goal_x = goal_x;
        self.caret_at_row_end = at_row_end;
        // Typing after the caret moved is a new undo step.
        self.seal_undo_step(cx);
        self.caret_moved(window, cx);
    }

    pub(crate) fn select_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let len = self.document.read(cx).text().len();
        self.selection = Selection { anchor: 0, head: len };
        self.goal_x = None;
        self.caret_at_row_end = false;
        self.seal_undo_step(cx);
        window.invalidate_character_coordinates();
        cx.notify();
    }

    /// Scrolls as little as possible to show the caret: its whole row, and a margin to the left
    /// and to the right of it.
    pub(super) fn reveal_caret(&mut self, window: &Window, cx: &App) {
        let doc = self.document.read(cx);
        let (text, lines) = (doc.text(), doc.lines());
        let caret = self.caret_row(text, lines);
        let (top, past) = self.top_row(text, lines);
        let below = self.rows_between(text, lines, top, caret, self.page_lines + 1);
        let fits = below.is_some_and(|rows| rows as f64 >= past && rows as f64 + 1.0 <= past + self.view_lines);
        let max_top = self.max_top(text, lines);
        if caret < top || (caret == top && past > 0.0) {
            self.scroll_top = self.scroll_top_at(text, lines, caret, 0.0).clamp(0.0, max_top);
        } else if !fits {
            // The caret's row at the bottom of the view, rows above it filling the view.
            let above = (self.view_lines - 1.0).max(0.0);
            let whole = above.ceil() as usize;
            let first = self.row_above(text, lines, caret, whole);
            let reached = self.rows_between(text, lines, first, caret, whole).unwrap_or(0);
            let past = if reached == whole { whole as f64 - above } else { 0.0 };
            self.scroll_top = self.scroll_top_at(text, lines, first, past).clamp(0.0, max_top);
        }
        if self.wrapping() {
            return;
        }

        let head = self.selection.head;
        let row = self.screen_row(text, lines, caret, window);
        let margin = MARGIN * self.metrics.char_width;
        let x = row.x_of(head);
        let left = head < row.shown.start || x < self.scroll_x + margin;
        let right = head > row.shown.end || x > self.scroll_x + self.text_width - margin;
        if !left && !right {
            return;
        }
        if row.shown == row.range {
            // The whole line is shaped: where the caret is does not depend on the scrolling.
            self.scroll_x = if left { (x - margin).max(0.0) } else { x - self.text_width + margin };
        } else {
            let offset = if left { margin } else { self.text_width - margin };
            self.scroll_x = self.long_line_scroll(caret.line, head, offset, window, cx);
        }
    }

    /// The scrolling that puts `pos` of the long line `line` `offset` pixels from the left edge of
    /// the text.
    fn long_line_scroll(&self, line: usize, pos: usize, offset: f64, window: &Window, cx: &App) -> f64 {
        let doc = self.document.read(cx);
        let (text, lines) = (doc.text(), doc.lines());
        let (range, _) = lines.line_range(text, line);
        // Lay out enough of the text before `pos` to span `offset` at a column and four bytes per
        // character, and find the character `offset` pixels before `pos`: scrolled to its column,
        // a long line shows it at the left edge.
        let columns = (offset / self.metrics.char_width).ceil() as usize + 1;
        let from = text.floor_char_boundary(pos.saturating_sub(4 * columns).max(range.start), range.start);
        let before = ScreenLine::part(text, range.clone(), from..pos, &self.line_style(), window);
        let edge = before.boundary_at(before.right() - offset);
        self.columns.column_of(text, &range, edge) as f64 * self.metrics.char_width
    }
}
