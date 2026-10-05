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
        let page = self.page_lines as isize;
        if let Motion::PageUp | Motion::PageDown = motion {
            // The text scrolls by a page, and the caret moves by as many lines.
            let by = if motion == Motion::PageUp { -page } else { page };
            self.scroll_to(self.scroll_top + by as f64, cx);
        }
        let doc = self.document.read(cx);
        let (text, lines) = (doc.text(), doc.lines());
        let Selection { anchor, head } = self.selection;
        let (start, end) = (anchor.min(head), anchor.max(head));
        // Left and right without Shift put the caret at the edge of the selection.
        let collapse = !select && start != end;
        let mut goal_x = None;
        let pos = match motion {
            Motion::Left if collapse => start,
            Motion::Right if collapse => end,
            Motion::Left => movement::prev_char(text, lines, head),
            Motion::Right => movement::next_char(text, lines, head),
            Motion::WordLeft => movement::word_left(text, lines, head),
            Motion::WordRight => movement::word_right(text, lines, head, WordStop::PLATFORM),
            Motion::LineStart => lines.start(lines.line_of(head)),
            Motion::LineEnd => movement::line_end(text, lines, head),
            Motion::DocStart => 0,
            Motion::DocEnd => text.len(),
            Motion::Up | Motion::Down | Motion::PageUp | Motion::PageDown => {
                let by = match motion {
                    Motion::Up => -1,
                    Motion::Down => 1,
                    Motion::PageUp => -page,
                    _ => page,
                };
                let line = lines.line_of(head);
                let row = |line| ScreenLine::new(text, lines, line, self.scroll_x, &self.metrics, window);
                let x = self.goal_x.unwrap_or_else(|| row(line).x_of(head));
                goal_x = Some(x);
                let target = (line as isize + by).clamp(0, lines.count() as isize - 1) as usize;
                // Up on the first line goes to the start of the text, down on the last to the end.
                if target != line {
                    row(target).boundary_at(x)
                } else if by < 0 {
                    0
                } else {
                    text.len()
                }
            }
        };
        self.selection = if select { Selection { anchor, head: pos } } else { Selection::caret(pos) };
        self.goal_x = goal_x;
        self.reveal_caret(window, cx);
        self.restart_blink(window, cx);
    }

    pub(crate) fn select_all(&mut self, cx: &mut Context<Self>) {
        let len = self.document.read(cx).text().len();
        self.selection = Selection { anchor: 0, head: len };
        self.goal_x = None;
        cx.notify();
    }

    /// Scrolls as little as possible to show the caret: its whole line, and a margin to the left
    /// and to the right of it.
    fn reveal_caret(&mut self, window: &Window, cx: &App) {
        let head = self.selection.head;
        let line = self.document.read(cx).lines().line_of(head);
        if (line as f64) < self.scroll_top {
            self.scroll_to(line as f64, cx);
        } else if line as f64 + 1.0 > self.scroll_top + self.view_lines {
            self.scroll_to(line as f64 + 1.0 - self.view_lines, cx);
        }

        let doc = self.document.read(cx);
        let row = ScreenLine::new(doc.text(), doc.lines(), line, self.scroll_x, &self.metrics, window);
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
            self.scroll_x = self.long_line_scroll(line, head, offset, window, cx);
        }
    }

    /// The scrolling that puts `pos` of the long line `line` `offset` pixels from the left edge of
    /// the text.
    fn long_line_scroll(&self, line: usize, pos: usize, offset: f64, window: &Window, cx: &App) -> f64 {
        let doc = self.document.read(cx);
        let (text, lines) = (doc.text(), doc.lines());
        let (range, _) = lines.line_range(text, line);
        // Lay out enough of the text before `pos` to span `offset` at a column and four bytes per
        // character, and find the character `offset` pixels before `pos`. A long line puts the
        // character at the left edge as many bytes from its start as there are columns scrolled.
        let columns = (offset / self.metrics.char_width).ceil() as usize + 1;
        let from = text.floor_char_boundary(pos.saturating_sub(4 * columns).max(range.start), range.start);
        let before = ScreenLine::part(text, range.clone(), from..pos, &self.metrics, window);
        let edge = before.boundary_at(before.right() - offset);
        (edge - range.start) as f64 * self.metrics.char_width
    }
}
