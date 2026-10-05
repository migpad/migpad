//! The view of a document: scrolling, font metrics and the layout of the visible lines.

use gpui::{
    Bounds, Context, Entity, FocusHandle, Focusable, Font, Hsla, Pixels, Point, Render, ScrollWheelEvent, ShapedLine,
    Subscription, TextRun, Window, div, point, prelude::*, px, rgb, size,
};
use migpad_core::document::Document;
use migpad_core::text::TextStore;

use crate::display::{DisplayText, MAX_SHAPED};
use crate::element::EditorElement;

/// Columns between tab stops, until the settings give it.
const TAB_WIDTH: usize = 8;
const FONT_SIZE: f32 = 13.0;
/// Space between the gutter and the text.
const PAD_LEFT: f32 = 4.0;
const SCROLLBAR_WIDTH: f32 = 12.0;

/// Colors of the light theme, until the theme of the interface takes over.
pub(crate) mod colors {
    pub const BACKGROUND: u32 = 0xffffff;
    pub const TEXT: u32 = 0x1f1f1f;
    pub const GUTTER: u32 = 0xf5f5f5;
    pub const LINE_NUMBER: u32 = 0x8a8a8a;
    pub const TRACK: u32 = 0xf4f4f4;
    pub const THUMB: u32 = 0xc0c0c0;
}

/// The monospace font of the system.
fn font_family() -> &'static str {
    if cfg!(target_os = "macos") {
        "Menlo"
    } else if cfg!(target_os = "windows") {
        "Consolas"
    } else {
        "DejaVu Sans Mono"
    }
}

pub(crate) struct Metrics {
    pub font: Font,
    pub font_size: Pixels,
    pub line_height: Pixels,
    /// The advance of a character, for the long lines shaped in windows.
    pub char_width: f64,
}

impl Metrics {
    fn new(window: &Window) -> Self {
        let font = gpui::font(font_family());
        let font_size = px(FONT_SIZE);
        let text_system = window.text_system();
        let id = text_system.resolve_font(&font);
        let char_width = text_system.advance(id, font_size, 'm').map(|advance| f64::from(advance.width)).unwrap_or(7.8);
        let ascent = f32::from(text_system.ascent(id, font_size)).abs().round();
        let descent = f32::from(text_system.descent(id, font_size)).abs().round();
        Metrics { font, font_size, line_height: px((ascent + descent).max(FONT_SIZE)), char_width }
    }

    fn run(&self, len: usize, color: u32) -> TextRun {
        let color: Hsla = rgb(color).into();
        TextRun { len, font: self.font.clone(), color, background_color: None, underline: None, strikethrough: None }
    }
}

/// The visible part of the document, laid out for painting.
pub(crate) struct Layout {
    pub bounds: Bounds<Pixels>,
    pub gutter: Bounds<Pixels>,
    pub text_area: Bounds<Pixels>,
    pub line_height: Pixels,
    pub lines: Vec<VisibleLine>,
    pub numbers: Vec<(ShapedLine, Point<Pixels>)>,
    pub track: Bounds<Pixels>,
    pub thumb: Option<Bounds<Pixels>>,
}

pub(crate) struct VisibleLine {
    pub shaped: ShapedLine,
    /// Where the shaped text starts, on screen.
    pub origin: Point<Pixels>,
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
    page_lines: usize,
    text_width: f64,
    /// The widest line laid out last time, which limits scrolling to the right.
    widest: f64,
    _observe: Subscription,
}

impl EditorView {
    pub fn new(document: Entity<Document>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&document, |_, _, cx| cx.notify());
        EditorView {
            document,
            focus: cx.focus_handle(),
            metrics: Metrics::new(window),
            scroll_top: 0.0,
            scroll_x: 0.0,
            page_lines: 1,
            text_width: 0.0,
            widest: 0.0,
            _observe: observe,
        }
    }

    pub fn document(&self) -> &Entity<Document> {
        &self.document
    }

    /// Lays out the lines that fit in `bounds`.
    pub(crate) fn layout(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut Context<Self>) -> Layout {
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
        self.page_lines = ((f64::from(bounds.size.height) / f64::from(line_height)) as usize).max(1);
        self.text_width = f64::from(text_area.size.width) - f64::from(PAD_LEFT);
        let max_top = count.saturating_sub(self.page_lines) as f64;
        self.scroll_top = self.scroll_top.clamp(0.0, max_top);

        let first = self.scroll_top.floor() as usize;
        let top = bounds.top() - px(((self.scroll_top - first as f64) * f64::from(line_height)) as f32);
        let last = count.min(first + self.page_lines + 2);
        let text_left = text_area.left() + px(PAD_LEFT) - px(self.scroll_x as f32);
        let mut visible = Vec::with_capacity(last - first);
        let mut numbers = Vec::with_capacity(last - first);
        let mut widest: f64 = 0.0;
        for (i, line) in (first..last).enumerate() {
            let y = top + line_height * i as f32;
            let (range, _) = lines.line_range(text, line);
            // A very long line is shaped in a window around the visible part; before the window,
            // one column per byte is close enough.
            let (start, column) = if range.len() > MAX_SHAPED {
                let skip = ((self.scroll_x / metrics.char_width) as usize).saturating_sub(MAX_SHAPED / 4);
                let wanted = range.start + skip;
                // The start of the character that the wanted byte belongs to.
                let start =
                    if wanted >= range.end { range.end } else { text.prev_char_boundary(wanted + 1, range.start) };
                (start, start - range.start)
            } else {
                (range.start, 0)
            };
            let end = if range.end - start > MAX_SHAPED {
                text.prev_char_boundary(start + MAX_SHAPED + 1, start)
            } else {
                range.end
            };
            let shown = DisplayText::new(&text.to_vec(start..end), column, TAB_WIDTH);
            let run = metrics.run(shown.text.len(), colors::TEXT);
            let shaped = window.text_system().shape_line(shown.text.into(), metrics.font_size, &[run], None);
            let x = column as f64 * metrics.char_width;
            widest = widest.max(x + f64::from(shaped.width()));
            visible.push(VisibleLine { shaped, origin: point(text_left + px(x as f32), y) });

            let number = (line + 1).to_string();
            let run = metrics.run(number.len(), colors::LINE_NUMBER);
            let shaped = window.text_system().shape_line(number.into(), metrics.font_size, &[run], None);
            let x = gutter.right() - px(7.) - shaped.width();
            numbers.push((shaped, point(x, y)));
        }
        self.widest = widest;

        let thumb = (count > self.page_lines).then(|| {
            let track_height = f64::from(track.size.height);
            let height = (track_height * self.page_lines as f64 / count as f64).max(24.0);
            let offset = (track_height - height) * self.scroll_top / max_top;
            Bounds::new(
                point(track.left() + px(2.), track.top() + px(offset as f32)),
                size(px(SCROLLBAR_WIDTH - 4.), px(height as f32)),
            )
        });
        Layout { bounds, gutter, text_area, line_height, lines: visible, numbers, track, thumb }
    }

    pub(crate) fn scroll(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let delta = event.delta.pixel_delta(self.metrics.line_height);
        self.scroll_top -= f64::from(delta.y) / f64::from(self.metrics.line_height);
        // Scrolling right stops when the widest visible line is half out of view.
        let max_x = (self.widest - self.text_width * 0.5).max(0.0);
        self.scroll_x = (self.scroll_x - f64::from(delta.x)).clamp(0.0, max_x.max(self.scroll_x));
        cx.notify();
    }
}

impl Focusable for EditorView {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for EditorView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().key_context("Editor").track_focus(&self.focus).size_full().child(EditorElement::new(cx.entity()))
    }
}
