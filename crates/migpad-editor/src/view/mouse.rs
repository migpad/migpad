//! The mouse: placing the caret and selecting, scrolling while a selection is dragged past the
//! edges, the scrollbar.

use std::ops::Range;
use std::time::Duration;

use gpui::{App, Context, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Point, Window, px};
use migpad_core::history::Selection;

use super::rows::RowAt;
use super::{ContextMenuEvent, EditorView};
use crate::movement;

/// How often the text scrolls while a selection is dragged past the edges of the view.
const AUTOSCROLL_TICK: Duration = Duration::from_millis(16);

/// What a drag with the left button does.
#[derive(Clone, Debug)]
pub(super) enum Drag {
    /// Selects by characters, words or lines to where the mouse is; `origin` stays selected: the
    /// anchor, or the word or line clicked first.
    Select { unit: Unit, origin: Range<usize>, mouse: Point<Pixels> },
    /// Moves the scrollbar thumb, held `grab` pixels below its top.
    Thumb { grab: f64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Unit {
    Char,
    Word,
    Line,
}

/// The selection that keeps `origin` and reaches out to `range`.
fn spanning(origin: &Range<usize>, range: Range<usize>) -> Selection {
    if range.start < origin.start {
        Selection { anchor: origin.end, head: range.start }
    } else {
        Selection { anchor: origin.start, head: range.end.max(origin.end) }
    }
}

impl EditorView {
    pub(crate) fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        // A click takes what an input method composes as it is, and starts a new undo step.
        self.marked = None;
        self.seal_undo_step(cx);
        if self.geometry.track.contains(&event.position) {
            self.scrollbar_down(event.position, cx);
            return;
        }
        let in_gutter = self.geometry.gutter.contains(&event.position);
        let (at, x) = self.hit(event.position, cx);
        let doc = self.document.read(cx);
        let (text, lines) = (doc.text(), doc.lines());
        let row = self.screen_row(text, lines, at, window);
        // A line number selects its line, as does a triple click; a double click selects a word.
        let (unit, range) = if in_gutter || event.click_count >= 3 {
            (Unit::Line, movement::line_with_break(text, lines, at.line))
        } else if event.click_count == 2 {
            (Unit::Word, movement::word_at(text, lines, row.char_at(x)))
        } else {
            let pos = row.boundary_at(x);
            (Unit::Char, pos..pos)
        };
        // A click past the end of a wrapped row puts the caret there, not at the start of the next.
        self.caret_at_row_end =
            unit == Unit::Char && range.start == row.shown.end && !self.last_row_of_line(text, lines, at);
        // Shift+click selects from the anchor.
        let anchor = self.selection.anchor;
        let origin = if unit == Unit::Char && event.modifiers.shift { anchor..anchor } else { range.clone() };
        self.selection = spanning(&origin, range);
        self.drag = Some(Drag::Select { unit, origin, mouse: event.position });
        self.goal_x = None;
        self.restart_blink(window, cx);
        window.invalidate_character_coordinates();
    }

    /// The right button, or Ctrl with the left one on macOS: a press outside the selection puts the
    /// caret there, one inside keeps it; then the view asks for its context menu.
    pub(crate) fn context_click(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        self.mouse_up();
        self.marked = None;
        self.seal_undo_step(cx);
        if !self.geometry.track.contains(&event.position) {
            let (at, x) = self.hit(event.position, cx);
            let doc = self.document.read(cx);
            let (text, lines) = (doc.text(), doc.lines());
            let row = self.screen_row(text, lines, at, window);
            let pos = row.boundary_at(x);
            let range = self.selected_range();
            if range.is_empty() || (!range.contains(&pos) && pos != range.end) {
                self.caret_at_row_end = pos == row.shown.end && !self.last_row_of_line(text, lines, at);
                self.selection = Selection::caret(pos);
                self.goal_x = None;
            }
            self.restart_blink(window, cx);
            window.invalidate_character_coordinates();
        }
        cx.emit(ContextMenuEvent { position: event.position, keyboard: false });
    }

    pub(crate) fn mouse_move(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.drag.is_none() {
            return;
        }
        if event.pressed_button != Some(MouseButton::Left) {
            // The button went up where the view did not see it.
            self.mouse_up();
            return;
        }
        match self.drag {
            Some(Drag::Thumb { grab }) => self.drag_thumb(event.position.y, grab, cx),
            Some(Drag::Select { ref mut mouse, .. }) => {
                *mouse = event.position;
                self.select_to_mouse(window, cx);
                self.update_autoscroll(window, cx);
            }
            None => {}
        }
    }

    pub(crate) fn mouse_up(&mut self) {
        self.drag = None;
        self.autoscroll = None;
    }

    /// The row under `position`, kept within the text vertically, and the x of `position` in
    /// pixels from the start of the row.
    pub(super) fn hit(&self, position: Point<Pixels>, cx: &App) -> (RowAt, f64) {
        let area = self.geometry.text_area;
        let y = f64::from(position.y - area.top()).clamp(0.0, (f64::from(area.size.height) - 1.0).max(0.0));
        let doc = self.document.read(cx);
        let (text, lines) = (doc.text(), doc.lines());
        let (top, past) = self.top_row(text, lines);
        let at = self.row_below(text, lines, top, (past + y / f64::from(self.metrics.line_height)) as usize);
        (at, f64::from(position.x - self.geometry.text_left) + self.scroll_x)
    }

    /// Extends the selection being dragged to the mouse.
    pub(super) fn select_to_mouse(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(Drag::Select { unit, origin, mouse }) = self.drag.clone() else { return };
        let (at, x) = self.hit(mouse, cx);
        let doc = self.document.read(cx);
        let (text, lines) = (doc.text(), doc.lines());
        let row = self.screen_row(text, lines, at, window);
        let range = match unit {
            Unit::Char => {
                let pos = row.boundary_at(x);
                pos..pos
            }
            Unit::Word => movement::word_at(text, lines, row.char_at(x)),
            Unit::Line => movement::line_with_break(text, lines, at.line),
        };
        self.caret_at_row_end =
            unit == Unit::Char && range.start == row.shown.end && !self.last_row_of_line(text, lines, at);
        self.selection = spanning(&origin, range);
        cx.notify();
    }

    /// How far the mouse dragging a selection is past the edges of the text, in pixels: negative
    /// to the left and above. An input field, one line high, only scrolls sideways.
    fn overshoot(&self) -> (f64, f64) {
        let Some(Drag::Select { mouse, .. }) = self.drag else { return (0.0, 0.0) };
        let area = self.geometry.text_area;
        let past = |at: Pixels, low: Pixels, high: Pixels| {
            f64::from(if at < low {
                at - low
            } else if at > high {
                at - high
            } else {
                px(0.)
            })
        };
        let dy = if self.single_line { 0.0 } else { past(mouse.y, area.top(), area.bottom()) };
        (past(mouse.x, area.left(), area.right()), dy)
    }

    /// Scrolls while the selection is dragged past the edges of the text, until it is back.
    fn update_autoscroll(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.overshoot() == (0.0, 0.0) {
            self.autoscroll = None;
        } else if self.autoscroll.is_none() {
            self.autoscroll = Some(cx.spawn_in(window, async move |view, cx| {
                loop {
                    cx.background_executor().timer(AUTOSCROLL_TICK).await;
                    let going = view.update_in(cx, |view, window, cx| view.autoscroll_step(window, cx));
                    if !going.unwrap_or(false) {
                        break;
                    }
                }
            }));
        }
    }

    /// One step of the scrolling while a selection is dragged past the edges: the farther the
    /// mouse, the faster. Returns whether to go on.
    fn autoscroll_step(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let (dx, dy) = self.overshoot();
        if (dx, dy) == (0.0, 0.0) {
            self.autoscroll = None;
            return false;
        }
        let line_height = f64::from(self.metrics.line_height);
        self.scroll_rows((dy / line_height / 4.0).clamp(-5.0, 5.0), cx);
        if !self.wrapping() {
            let max_x = self.max_scroll_x().max(self.scroll_x);
            self.scroll_x = (self.scroll_x + (dx / 4.0).clamp(-40.0, 40.0)).clamp(0.0, max_x);
        }
        self.select_to_mouse(window, cx);
        true
    }

    /// A press on the scrollbar: on the thumb it starts dragging it, beside it scrolls a page.
    fn scrollbar_down(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(thumb) = self.geometry.thumb else { return };
        if thumb.contains(&position) {
            self.drag = Some(Drag::Thumb { grab: f64::from(position.y - thumb.top()) });
        } else {
            let page = self.page_lines as f64;
            self.scroll_rows(if position.y < thumb.top() { -page } else { page }, cx);
            cx.notify();
        }
    }

    fn drag_thumb(&mut self, y: Pixels, grab: f64, cx: &mut Context<Self>) {
        let (track, Some(thumb)) = (self.geometry.track, self.geometry.thumb) else { return };
        let free = f64::from(track.size.height - thumb.size.height);
        if free > 0.0 {
            let doc = self.document.read(cx);
            let share = ((f64::from(y - track.top()) - grab) / free).clamp(0.0, 1.0);
            self.scroll_top = share * self.max_top(doc.text(), doc.lines());
            cx.notify();
        }
    }
}
