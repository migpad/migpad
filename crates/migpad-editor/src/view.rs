//! The view of a document: the selection and the caret, scrolling, and the layout of the visible
//! lines.

mod caret;
mod edit;
mod input;
mod mouse;

use std::ops::Range;
use std::time::Duration;

use gpui::{
    App, Bounds, Context, Entity, FocusHandle, Focusable, Pixels, Render, ScrollWheelEvent, Subscription, Task, Window,
    div, point, prelude::*, px, size,
};
use migpad_core::document::Document;
use migpad_core::history::Selection;

pub(crate) use caret::Motion;
pub(crate) use edit::Deletion;
use mouse::Drag;

use crate::element::EditorElement;
use crate::keymap::{self, CONTEXT};
use crate::layout::{Geometry, Layout, Metrics, ScreenLine};
use crate::movement;

/// Space between the gutter and the text.
const PAD_LEFT: f32 = 4.0;
const SCROLLBAR_WIDTH: f32 = 12.0;
const CARET_WIDTH: f32 = 2.0;
/// How long the blinking caret is shown, and then hidden.
const BLINK: Duration = Duration::from_millis(500);

/// Colors of the light theme, until the theme of the interface takes over.
pub(crate) mod colors {
    pub const BACKGROUND: u32 = 0xffffff;
    pub const TEXT: u32 = 0x1f1f1f;
    pub const GUTTER: u32 = 0xf5f5f5;
    pub const LINE_NUMBER: u32 = 0x8a8a8a;
    pub const TRACK: u32 = 0xf4f4f4;
    pub const THUMB: u32 = 0xc0c0c0;
    pub const SELECTION: u32 = 0xb4d5fe;
    /// The selection of a view without focus or in an inactive window.
    pub const SELECTION_INACTIVE: u32 = 0xdcdcdc;
    pub const CARET: u32 = 0x1f1f1f;
}

/// The view of a document in a window.
pub struct EditorView {
    document: Entity<Document>,
    focus: FocusHandle,
    metrics: Metrics,
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
    pub fn new(document: Entity<Document>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        let subscriptions = vec![
            cx.observe(&document, |view, _, cx| view.document_changed(cx)),
            cx.on_focus(&focus, window, Self::restart_blink),
            cx.on_blur(&focus, window, Self::blurred),
            cx.observe_window_activation(window, Self::restart_blink),
        ];
        EditorView {
            document,
            focus,
            metrics: Metrics::new(window),
            scroll_top: 0.0,
            scroll_x: 0.0,
            page_lines: 1,
            view_lines: 1.0,
            text_width: 0.0,
            widest: 0.0,
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

    pub fn document(&self) -> &Entity<Document> {
        &self.document
    }

    /// Keeps the selection where the caret can be once the text has changed under it.
    fn document_changed(&mut self, cx: &mut Context<Self>) {
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

    fn max_top(&self, count: usize) -> f64 {
        count.saturating_sub(self.page_lines) as f64
    }

    /// Scrolls so that `top` is the first visible line, as far as the document allows.
    fn scroll_to(&mut self, top: f64, cx: &App) {
        let count = self.document.read(cx).lines().count();
        self.scroll_top = top.clamp(0.0, self.max_top(count));
    }

    /// Lays out the lines that fit in `bounds`, with the selection and the caret.
    pub(crate) fn layout(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut Context<Self>) -> Layout {
        let active = self.focus.is_focused(window) && window.is_window_active();
        let doc = self.document.read(cx);
        let (text, lines) = (doc.text(), doc.lines());
        let count = lines.count();
        let metrics = &self.metrics;
        let line_height = metrics.line_height;
        let digits = count.to_string().len().max(3);
        let gutter_width = px((digits as f64 * metrics.char_width + 14.0) as f32);
        let track = Bounds::new(
            point(bounds.right() - px(SCROLLBAR_WIDTH), bounds.top()),
            size(px(SCROLLBAR_WIDTH), bounds.size.height),
        );
        let gutter = Bounds::from_corners(bounds.origin, point(bounds.left() + gutter_width, bounds.bottom()));
        let text_area = Bounds::from_corners(point(gutter.right(), bounds.top()), point(track.left(), bounds.bottom()));
        let text_left = text_area.left() + px(PAD_LEFT);
        self.view_lines = f64::from(bounds.size.height) / f64::from(line_height);
        self.page_lines = (self.view_lines as usize).max(1);
        self.text_width = f64::from(text_area.size.width) - f64::from(PAD_LEFT);
        let max_top = self.max_top(count);
        self.scroll_top = self.scroll_top.clamp(0.0, max_top);

        let first = self.scroll_top.floor() as usize;
        let top = bounds.top() - px(((self.scroll_top - first as f64) * f64::from(line_height)) as f32);
        let last = count.min(first + self.page_lines + 2);
        let scroll_x = self.scroll_x;
        let screen_x = |x: f64| text_left + px((x - scroll_x) as f32);
        let Selection { anchor, head } = self.selection;
        let (start, end) = (anchor.min(head), anchor.max(head));
        let mut layout = Layout {
            geometry: Geometry { bounds, gutter, text_area, text_left, track, thumb: None },
            line_height,
            lines: Vec::with_capacity(last - first),
            numbers: Vec::with_capacity(last - first),
            selection: Vec::new(),
            selection_color: if active { colors::SELECTION } else { colors::SELECTION_INACTIVE },
            caret: None,
            hitbox: None,
        };
        let mut widest: f64 = 0.0;
        for (i, line) in (first..last).enumerate() {
            let y = top + line_height * i as f32;
            let row = ScreenLine::new(text, lines, line, scroll_x, self.marked.as_ref(), metrics, window);
            widest = widest.max(row.right());
            if start < end && start <= row.range.end && end > row.range.start {
                let from = row.x_of(start.max(row.range.start));
                // A selected line break shows as a sliver after the end of the line.
                let to = if end > row.range.end {
                    row.x_of(row.range.end) + metrics.char_width * 0.5
                } else {
                    row.x_of(end)
                };
                layout
                    .selection
                    .push(Bounds::from_corners(point(screen_x(from), y), point(screen_x(to), y + line_height)));
            }
            if active && self.caret_on && (row.shown.start..=row.shown.end).contains(&head) {
                let x = screen_x(row.x_of(head)).round() - px(CARET_WIDTH / 2.);
                layout.caret = Some(Bounds::new(point(x, y), size(px(CARET_WIDTH), line_height)));
            }
            layout.lines.push((row.shaped, point(screen_x(row.x), y)));

            let number = (line + 1).to_string();
            let run = metrics.run(number.len(), colors::LINE_NUMBER);
            let shaped = window.text_system().shape_line(number.into(), metrics.font_size, &[run], None);
            let x = gutter.right() - px(7.) - shaped.width();
            layout.numbers.push((shaped, point(x, y)));
        }
        self.widest = widest;

        layout.geometry.thumb = (count > self.page_lines).then(|| {
            let track_height = f64::from(track.size.height);
            let height = (track_height * self.page_lines as f64 / count as f64).max(24.0);
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
        let delta = event.delta.pixel_delta(self.metrics.line_height);
        self.scroll_to(self.scroll_top - f64::from(delta.y) / f64::from(self.metrics.line_height), cx);
        // Scrolling right stops when the widest visible line is half out of view.
        let max_x = (self.widest - self.text_width * 0.5).max(0.0);
        self.scroll_x = (self.scroll_x - f64::from(delta.x)).clamp(0.0, max_x.max(self.scroll_x));
        // A selection being dragged follows the text under the mouse.
        self.select_to_mouse(window, cx);
        cx.notify();
    }
}

impl Focusable for EditorView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for EditorView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = div().key_context(CONTEXT).track_focus(&self.focus).size_full();
        let view = keymap::on_edits(view, cx);
        keymap::on_motions(view, cx).child(EditorElement::new(cx.entity()))
    }
}
