//! The view of a document: the selection and the caret, scrolling, and the layout of the visible
//! lines.

mod caret;
mod edit;
mod input;
mod mouse;
mod rows;

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, Bounds, Context, Entity, FocusHandle, Focusable, Pixels, Render, ScrollWheelEvent, Subscription, Task, Window,
    div, point, prelude::*, px, size,
};
use migpad_core::document::Document;
use migpad_core::history::Selection;
use migpad_core::text::TextStore;

pub(crate) use caret::Motion;
pub(crate) use edit::Deletion;
use mouse::Drag;

use crate::colors::EditorColors;
use crate::columns::Columns;
use crate::element::EditorElement;
use crate::indent;
use crate::keymap;
use crate::layout::{Geometry, Layout, LineStyle, Metrics, TAB_WIDTH};
use crate::movement;

/// Space between the gutter and the text.
const PAD_LEFT: f32 = 4.0;
const SCROLLBAR_WIDTH: f32 = 12.0;
const CARET_WIDTH: f32 = 2.0;
/// How long the blinking caret is shown, and then hidden.
const BLINK: Duration = Duration::from_millis(500);
/// Lines do not wrap in a view narrower than this many cells.
const MIN_WRAP_CELLS: usize = 8;

/// The view of a document in a window, or the one line of an input field.
pub struct EditorView {
    document: Entity<Document>,
    focus: FocusHandle,
    /// An input field: one line, without line numbers, a scrollbar or wrapping.
    single_line: bool,
    metrics: Metrics,
    /// The colors of the last layout: those of the theme.
    colors: EditorColors,
    /// The first visible line, with a fraction for smooth scrolling. Lines rather than pixels:
    /// millions of lines times their height do not fit the precision of `Pixels`.
    scroll_top: f64,
    /// Pixels scrolled to the right.
    scroll_x: f64,
    /// Whole lines that fit in the view.
    page_lines: usize,
    /// Lines that fit in the view, with a fraction.
    view_lines: f64,
    text_width: f64,
    /// The widest line laid out last time, which limits scrolling to the right.
    widest: f64,
    /// Columns of the long lines, found as they are laid out.
    columns: Columns,
    /// Whether spaces, tabs and line breaks are marked.
    show_whitespace: bool,
    show_indent_guides: bool,
    /// Whether lines wrap to the width of the view; they do not in a large file.
    word_wrap: bool,
    /// The cells of a row lines wrap to; none when they do not wrap.
    wrap_cells: usize,
    /// Where the rows of the lines laid out since the text or the width last changed start, by
    /// the start of the line.
    wraps: RefCell<HashMap<usize, Rc<[usize]>>>,
    /// Whether the caret at a wrap is at the end of the upper row rather than the start of the
    /// lower one.
    caret_at_row_end: bool,
    /// Where the parts of the view were in the last layout, for the mouse.
    geometry: Geometry,
    selection: Selection,
    /// The text an input method is composing: underlined, and replaced until it is committed.
    marked: Option<Range<usize>>,
    /// Where vertical moves keep the caret, in pixels from the start of a line: past shorter lines
    /// it comes back to its column.
    goal_x: Option<f64>,
    drag: Option<Drag>,
    /// Scrolls while a selection is dragged past the edges of the text.
    autoscroll: Option<Task<()>>,
    /// Whether the blinking caret is shown at the moment.
    caret_on: bool,
    blink: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EditorView {
    /// The view of `document`.
    pub fn new(document: Entity<Document>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::create(document, false, window, cx)
    }

    /// An input field: one line of text in a document of its own, without a journal. Enter, Tab
    /// and Escape go on to the element around the field, which binds them in its key context to
    /// what the field is for — find, go to a line — or to moving the focus; the field is a tab
    /// stop. Text with line breaks becomes one line: breaks at its ends are dropped, and each one
    /// inside becomes a space.
    pub fn single_line(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let document = cx.new(|_| Document::new());
        Self::create(document, true, window, cx)
    }

    fn create(document: Entity<Document>, single_line: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle().tab_stop(single_line);
        let subscriptions = vec![
            cx.observe(&document, |view, _, cx| view.document_changed(cx)),
            cx.on_focus(&focus, window, Self::restart_blink),
            cx.on_blur(&focus, window, Self::blurred),
            cx.observe_window_activation(window, Self::restart_blink),
        ];
        EditorView {
            document,
            focus,
            single_line,
            metrics: Metrics::new(window),
            colors: EditorColors::current(cx),
            scroll_top: 0.0,
            scroll_x: 0.0,
            page_lines: 1,
            view_lines: 1.0,
            text_width: 0.0,
            widest: 0.0,
            columns: Columns::new(TAB_WIDTH),
            show_whitespace: false,
            show_indent_guides: false,
            word_wrap: false,
            wrap_cells: 0,
            wraps: RefCell::default(),
            caret_at_row_end: false,
            geometry: Geometry::default(),
            selection: Selection::default(),
            marked: None,
            goal_x: None,
            drag: None,
            autoscroll: None,
            caret_on: true,
            blink: None,
            _subscriptions: subscriptions,
        }
    }

    /// The document of the view; the owner of an input field observes it to hear of edits.
    pub fn document(&self) -> &Entity<Document> {
        &self.document
    }

    /// The selection: where it was started, and the caret.
    pub fn selection(&self) -> Selection {
        self.selection
    }

    /// Selects from `selection.anchor` to the caret at `selection.head`, both kept within the text
    /// and on character boundaries, and scrolls to show the caret: the view of a reopened document
    /// comes back to where it was.
    pub fn select(&mut self, selection: Selection, window: &mut Window, cx: &mut Context<Self>) {
        let doc = self.document.read(cx);
        let snap = |pos| movement::snap(doc.text(), doc.lines(), pos);
        self.selection = Selection { anchor: snap(selection.anchor), head: snap(selection.head) };
        self.goal_x = None;
        self.caret_at_row_end = false;
        self.seal_undo_step(cx);
        self.caret_moved(window, cx);
    }

    /// The line of the caret and its column on screen, both from zero: a tab reaches to its stop,
    /// any other character takes one column.
    pub fn caret_position(&self, cx: &App) -> (usize, usize) {
        let doc = self.document.read(cx);
        let (text, lines) = (doc.text(), doc.lines());
        let head = self.selection.head.min(text.len());
        let line = lines.line_of(head);
        let (range, _) = lines.line_range(text, line);
        (line, self.columns.column_of(text, &range, head))
    }

    /// The text, invalid UTF-8 as U+FFFD: what an input field holds.
    pub fn text(&self, cx: &App) -> String {
        let text = self.document.read(cx).text();
        String::from_utf8_lossy(&text.to_vec(0..text.len())).into_owned()
    }

    /// The height of an input field: one line. The view of a document takes what it is given.
    pub(crate) fn fixed_height(&self) -> Option<Pixels> {
        self.single_line.then_some(self.metrics.line_height)
    }

    /// Whether spaces, tabs and line breaks are marked.
    pub fn shows_whitespace(&self) -> bool {
        self.show_whitespace
    }

    pub fn set_show_whitespace(&mut self, show: bool, cx: &mut Context<Self>) {
        self.show_whitespace = show;
        cx.notify();
    }

    /// Whether levels of indentation are marked by vertical lines.
    pub fn shows_indent_guides(&self) -> bool {
        self.show_indent_guides
    }

    pub fn set_show_indent_guides(&mut self, show: bool, cx: &mut Context<Self>) {
        self.show_indent_guides = show;
        cx.notify();
    }

    /// Whether lines wrap to the width of the view: they do not in a large file even so.
    pub fn wraps_lines(&self) -> bool {
        self.word_wrap
    }

    pub fn set_word_wrap(&mut self, wrap: bool, cx: &mut Context<Self>) {
        self.word_wrap = wrap;
        self.wraps.get_mut().clear();
        self.caret_at_row_end = false;
        cx.notify();
    }

    /// Forgets what was found about the lines: the text has changed.
    fn text_changed(&mut self) {
        self.columns.clear();
        self.wraps.get_mut().clear();
        self.caret_at_row_end = false;
    }

    /// How the lines of the view are laid out now.
    fn line_style(&self) -> LineStyle<'_> {
        LineStyle {
            metrics: &self.metrics,
            colors: &self.colors,
            columns: &self.columns,
            scroll_x: self.scroll_x,
            whitespace: self.show_whitespace,
            underline: self.marked.as_ref(),
        }
    }

    /// Keeps the selection where the caret can be once the text has changed under it.
    fn document_changed(&mut self, cx: &mut Context<Self>) {
        self.text_changed();
        let doc = self.document.read(cx);
        let snap = |pos| movement::snap(doc.text(), doc.lines(), pos);
        self.selection = Selection { anchor: snap(self.selection.anchor), head: snap(self.selection.head) };
        cx.notify();
    }

    fn selected_range(&self) -> Range<usize> {
        let Selection { anchor, head } = self.selection;
        anchor.min(head)..anchor.max(head)
    }

    /// Ends the undo step being typed: the next edit starts a new one.
    fn seal_undo_step(&mut self, cx: &mut Context<Self>) {
        self.document.update(cx, |doc, _| doc.seal_undo_step());
    }

    /// Takes the text being composed as it is.
    fn end_composition(&mut self, cx: &mut Context<Self>) {
        if self.marked.take().is_some() {
            self.seal_undo_step(cx);
        }
    }

    /// After the caret moved or the text changed: shows the caret, and tells input methods
    /// where it is now.
    fn caret_moved(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reveal_caret(window, cx);
        self.restart_blink(window, cx);
        window.invalidate_character_coordinates();
    }

    /// Without focus, what is being composed stays as it is, and typing after the focus is back
    /// starts a new undo step.
    fn blurred(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        self.seal_undo_step(cx);
        self.restart_blink(window, cx);
    }

    /// Shows the caret and starts its blinking over, if the view has focus in an active window;
    /// stops the blinking otherwise.
    fn restart_blink(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.caret_on = true;
        let blinking = self.focus.is_focused(window) && window.is_window_active();
        self.blink = blinking.then(|| {
            cx.spawn(async move |view, cx| {
                loop {
                    cx.background_executor().timer(BLINK).await;
                    let blinked = view.update(cx, |view, cx| {
                        view.caret_on = !view.caret_on;
                        cx.notify();
                    });
                    if blinked.is_err() {
                        break;
                    }
                }
            })
        });
        cx.notify();
    }

    /// Lays out the rows that fit in `bounds`, with the selection and the caret.
    pub(crate) fn layout(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut Context<Self>) -> Layout {
        let active = self.focus.is_focused(window) && window.is_window_active();
        self.colors = EditorColors::current(cx);
        let colors = self.colors;
        let (count, large) = {
            let doc = self.document.read(cx);
            (doc.lines().count(), doc.is_large())
        };
        let line_height = self.metrics.line_height;
        let char_width = self.metrics.char_width;
        // An input field has neither line numbers nor a scrollbar, and its text keeps off both
        // edges.
        let (gutter_width, scrollbar_width, pad_right) = if self.single_line {
            (px(0.), px(0.), PAD_LEFT)
        } else {
            let digits = count.to_string().len().max(3);
            (px((digits as f64 * char_width + 14.0) as f32), px(SCROLLBAR_WIDTH), 0.0)
        };
        let track = Bounds::new(
            point(bounds.right() - scrollbar_width, bounds.top()),
            size(scrollbar_width, bounds.size.height),
        );
        let gutter = Bounds::from_corners(bounds.origin, point(bounds.left() + gutter_width, bounds.bottom()));
        let text_area = Bounds::from_corners(point(gutter.right(), bounds.top()), point(track.left(), bounds.bottom()));
        let text_left = text_area.left() + px(PAD_LEFT);
        self.view_lines = if self.single_line { 1.0 } else { f64::from(bounds.size.height) / f64::from(line_height) };
        self.page_lines = (self.view_lines as usize).max(1);
        self.text_width = f64::from(text_area.size.width) - f64::from(PAD_LEFT + pad_right);
        // Lines wrap to the whole cells of the text width, one left for the caret; not those of a
        // large file or of an input field.
        let cells =
            if self.word_wrap && !large && !self.single_line { (self.text_width / char_width) as usize } else { 0 };
        let cells = if cells > MIN_WRAP_CELLS { cells - 1 } else { 0 };
        if cells != self.wrap_cells {
            self.wrap_cells = cells;
            self.wraps.get_mut().clear();
        }
        if self.wrapping() {
            self.scroll_x = 0.0;
        }
        let doc = self.document.read(cx);
        let (text, lines) = (doc.text(), doc.lines());
        let max_top = self.max_top(text, lines);
        self.scroll_top = self.scroll_top.clamp(0.0, max_top);

        // The rows in view, from the one at the top.
        let (top_row, past) = self.top_row(text, lines);
        if self.single_line {
            // The text fills the field: while some of it is scrolled out on the left, its end
            // stays at the right edge, however the text shrinks or the field widens.
            let row = self.screen_row(text, lines, top_row, window);
            if row.shown.end == row.range.end {
                self.scroll_x = self.scroll_x.min((row.right() - self.text_width).max(0.0));
            }
        }
        let top = bounds.top() - px((past * f64::from(line_height)) as f32);
        let mut rows = Vec::with_capacity(self.page_lines + 2);
        let mut next = Some(top_row);
        while let Some(at) = next
            && rows.len() < self.page_lines + 2
        {
            rows.push(at);
            next = self.next_row(text, lines, at);
        }
        let first_line = top_row.line;
        let end_line = rows.last().map_or(first_line, |at| at.line) + 1;

        let scroll_x = self.scroll_x;
        let screen_x = |x: f64| text_left + px((x - scroll_x) as f32);
        let guides = (self.show_indent_guides && !self.single_line).then(|| {
            let levels = indent::guide_levels(text, lines, first_line..end_line, TAB_WIDTH);
            let indents: Vec<Option<usize>> = (first_line..end_line)
                .map(|line| indent::indentation(text, lines.line_range(text, line).0, TAB_WIDTH))
                .collect();
            (levels, indent::indent_step(&indents, TAB_WIDTH))
        });
        let Selection { anchor, head } = self.selection;
        let (start, end) = (anchor.min(head), anchor.max(head));
        let metrics = &self.metrics;
        // Room for the caret at either end of the text of an input field.
        let clip = if self.single_line {
            let room = px(CARET_WIDTH / 2.);
            let right = text_left + px(self.text_width as f32) + room;
            Bounds::from_corners(point(text_left - room, bounds.top()), point(right, bounds.bottom()))
        } else {
            text_area
        };
        let mut layout = Layout {
            geometry: Geometry { bounds, gutter, text_area, text_left, track, thumb: None },
            clip,
            line_height,
            lines: Vec::with_capacity(rows.len()),
            numbers: Vec::with_capacity(rows.len()),
            selection: Vec::new(),
            selection_color: if active { colors.selection } else { colors.selection_inactive },
            colors,
            caret: None,
            guides: Vec::new(),
            labels: Vec::new(),
            hitbox: None,
        };
        let mut widest: f64 = 0.0;
        for (i, &at) in rows.iter().enumerate() {
            let y = top + line_height * i as f32;
            let row = self.screen_row(text, lines, at, window);
            // The number and the guides go to the first row of a line, the label of its line
            // break and the sliver of a selected one to the last.
            let first = at.row == 0;
            let last = self.last_row_of_line(text, lines, at);
            widest = widest.max(row.right());
            if first && let Some((levels, step)) = &guides {
                // A guide at the start of each level the line is indented past.
                for column in (0..levels[at.line - first_line]).step_by(*step) {
                    let x = screen_x(column as f64 * char_width).round();
                    layout.guides.push(Bounds::new(point(x, y), size(px(1.), line_height)));
                }
            }
            let eol = lines.line_range(text, at.line).1;
            if self.show_whitespace && last && eol > 0 && row.range.end <= row.shown.end {
                let label = match (eol, text.byte(row.range.end)) {
                    (2, _) => "CRLF",
                    (_, b'\r') => "CR",
                    _ => "LF",
                };
                let run = metrics.run(label.len(), colors.label);
                let font_size = metrics.font_size * 0.75;
                let shaped = window.text_system().shape_line(label.into(), font_size, &[run], None);
                let x = screen_x(row.x_of(row.range.end)) + px(3.);
                let background = Bounds::from_corners(
                    point(x - px(2.), y + px(2.)),
                    point(x + shaped.width() + px(2.), y + line_height - px(2.)),
                );
                layout.labels.push((shaped, point(x, y), background));
            }
            if start < end && start <= row.shown.end && end > row.shown.start {
                let from = row.x_of(start.max(row.shown.start));
                let to = if end > row.range.end && last {
                    row.x_of(row.range.end) + char_width * 0.5
                } else {
                    row.x_of(end.min(row.shown.end))
                };
                if to > from {
                    let selected = Bounds::from_corners(point(screen_x(from), y), point(screen_x(to), y + line_height));
                    layout.selection.push(selected);
                }
            }
            // At a wrap the caret is at the start of the lower row, or at the end of the upper one.
            let on_row = (row.shown.start..=row.shown.end).contains(&head)
                && !(head == row.shown.end && !last && !self.caret_at_row_end)
                && !(head == row.shown.start && !first && self.caret_at_row_end);
            if active && self.caret_on && on_row {
                let x = screen_x(row.x_of(head)).round() - px(CARET_WIDTH / 2.);
                layout.caret = Some(Bounds::new(point(x, y), size(px(CARET_WIDTH), line_height)));
            }
            layout.lines.push((row.shaped, point(screen_x(row.x), y)));

            if first && !self.single_line {
                let number = (at.line + 1).to_string();
                let run = metrics.run(number.len(), colors.line_number);
                let shaped = window.text_system().shape_line(number.into(), metrics.font_size, &[run], None);
                let x = gutter.right() - px(7.) - shaped.width();
                layout.numbers.push((shaped, point(x, y)));
            }
        }
        self.widest = widest;

        layout.geometry.thumb = (max_top > 0.0 && !self.single_line).then(|| {
            let track_height = f64::from(track.size.height);
            let page = self.page_lines as f64;
            let height = (track_height * page / (max_top + page)).max(24.0);
            let offset = (track_height - height) * self.scroll_top / max_top;
            Bounds::new(
                point(track.left() + px(2.), track.top() + px(offset as f32)),
                size(px(SCROLLBAR_WIDTH - 4.), px(height as f32)),
            )
        });
        self.geometry = layout.geometry;
        layout
    }

    pub(crate) fn scroll(&mut self, event: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        let line_height = f64::from(self.metrics.line_height);
        let delta = event.delta.pixel_delta(self.metrics.line_height);
        self.scroll_rows(-f64::from(delta.y) / line_height, cx);
        if !self.wrapping() {
            // A view scrolled past the limit to show the caret stays there.
            let max_x = self.max_scroll_x().max(self.scroll_x);
            self.scroll_x = (self.scroll_x - f64::from(delta.x)).clamp(0.0, max_x);
        }
        // A selection being dragged follows the text under the mouse.
        self.select_to_mouse(window, cx);
        cx.notify();
    }

    /// How far the view scrolls to the right: an input field until the end of its text is at the
    /// right edge, a document until the widest visible line is half out of view.
    pub(super) fn max_scroll_x(&self) -> f64 {
        let reach = if self.single_line { self.text_width } else { self.text_width * 0.5 };
        (self.widest - reach).max(0.0)
    }
}

impl Focusable for EditorView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for EditorView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = div().key_context(keymap::key_context(self.single_line)).track_focus(&self.focus);
        let view = if self.single_line { view.w_full() } else { keymap::on_document_actions(view.size_full(), cx) };
        let view = keymap::on_edits(view, cx);
        keymap::on_motions(view, cx).child(EditorElement::new(cx.entity()))
    }
}
