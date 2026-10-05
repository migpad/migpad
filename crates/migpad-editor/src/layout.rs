//! Lines laid out for the screen: the shaped text of a line and where each of its bytes is; the
//! parts of the view and what the element paints.

use std::ops::Range;

use gpui::{Bounds, Font, Hitbox, Hsla, Pixels, Point, ShapedLine, TextRun, UnderlineStyle, Window, px, rgb};
use migpad_core::document::Text;
use migpad_core::text::{LineIndex, TextStore};

use crate::display::{DisplayText, MAX_SHAPED, OffsetMap};
use crate::view::colors;

/// Columns between tab stops, until the settings give it.
const TAB_WIDTH: usize = 8;
const FONT_SIZE: f32 = 13.0;

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
    /// The advance of a character: the columns of the gutter and the long lines shaped in windows.
    pub char_width: f64,
}

impl Metrics {
    pub fn new(window: &Window) -> Self {
        let font = gpui::font(font_family());
        let font_size = px(FONT_SIZE);
        let text_system = window.text_system();
        let id = text_system.resolve_font(&font);
        let char_width = text_system.advance(id, font_size, 'm').map(|advance| f64::from(advance.width)).unwrap_or(7.8);
        let ascent = f32::from(text_system.ascent(id, font_size)).abs().round();
        let descent = f32::from(text_system.descent(id, font_size)).abs().round();
        Metrics { font, font_size, line_height: px((ascent + descent).max(FONT_SIZE)), char_width }
    }

    pub fn run(&self, len: usize, color: u32) -> TextRun {
        let color: Hsla = rgb(color).into();
        TextRun { len, font: self.font.clone(), color, background_color: None, underline: None, strikethrough: None }
    }
}

/// A line of the document laid out for the screen. A long line is shaped only in a window around
/// the visible part; positions outside the window are clamped to it.
pub(crate) struct ScreenLine {
    /// The bytes of the line, without its line break.
    pub range: Range<usize>,
    /// The bytes that were shaped.
    pub shown: Range<usize>,
    /// Where the shaped text starts, in pixels from the start of the line.
    pub x: f64,
    pub shaped: ShapedLine,
    map: OffsetMap,
}

impl ScreenLine {
    /// Lays out `line`; a long one in a window around `scroll_x`, the pixels scrolled to the right.
    /// The bytes of `underline` are underlined: the text an input method composes.
    pub fn new(
        text: &Text,
        lines: &LineIndex,
        line: usize,
        scroll_x: f64,
        underline: Option<&Range<usize>>,
        metrics: &Metrics,
        window: &Window,
    ) -> Self {
        let (range, _) = lines.line_range(text, line);
        if range.len() <= MAX_SHAPED {
            return Self::part(text, range.clone(), range, underline, metrics, window);
        }
        // The character at the left edge of the view is found by counting one column per byte from
        // the start of the line. The window is shaped around it and placed so that this character
        // is at the edge, moved left by the part of it scrolled past: the caret and the mouse see
        // the same positions whatever window is shaped, and scrolling stays smooth.
        let column = scroll_x / metrics.char_width;
        let wanted = (range.start as f64 + column).min(range.end as f64) as usize;
        let edge = text.floor_char_boundary(wanted, range.start);
        let start = text.floor_char_boundary(edge.saturating_sub(MAX_SHAPED / 4).max(range.start), range.start);
        let end = text.floor_char_boundary((start + MAX_SHAPED).min(range.end), start);
        let mut line = Self::part(text, range.clone(), start..end, underline, metrics, window);
        let next = text.next_char_boundary(edge, end);
        let scrolled_past = if next > edge {
            ((column - (edge - range.start) as f64) / (next - edge) as f64).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let (from, to) = (line.x_of(edge), line.x_of(next));
        line.x = scroll_x - (from + scrolled_past * (to - from));
        line
    }

    /// Lays out the part `shown` of the line with the bytes `range`.
    pub fn part(
        text: &Text,
        range: Range<usize>,
        shown: Range<usize>,
        underline: Option<&Range<usize>>,
        metrics: &Metrics,
        window: &Window,
    ) -> Self {
        let DisplayText { text: shown_text, map } =
            DisplayText::new(&text.to_vec(shown.clone()), shown.start - range.start, TAB_WIDTH);
        let len = shown_text.len();
        let mut runs = vec![metrics.run(len, colors::TEXT)];
        if let Some(underline) = underline
            && underline.start < shown.end
            && underline.end > shown.start
        {
            let at = |pos: usize| map.display_offset(pos.clamp(shown.start, shown.end) - shown.start);
            let (from, to) = (at(underline.start), at(underline.end));
            let style = UnderlineStyle { color: Some(rgb(colors::TEXT).into()), thickness: px(1.), wavy: false };
            runs = vec![
                metrics.run(from, colors::TEXT),
                TextRun { underline: Some(style), ..metrics.run(to - from, colors::TEXT) },
                metrics.run(len - to, colors::TEXT),
            ];
            runs.retain(|run| run.len > 0);
        }
        let shaped = window.text_system().shape_line(shown_text.into(), metrics.font_size, &runs, None);
        ScreenLine { range, shown, x: 0.0, shaped, map }
    }

    /// Where the character boundary at `pos` is, in pixels from the start of the line.
    pub fn x_of(&self, pos: usize) -> f64 {
        let pos = pos.clamp(self.shown.start, self.shown.end);
        self.x + f64::from(self.shaped.x_for_index(self.map.display_offset(pos - self.shown.start)))
    }

    /// The character boundary nearest to `x`, pixels from the start of the line.
    pub fn boundary_at(&self, x: f64) -> usize {
        let x = px((x - self.x) as f32);
        // The starts of the glyphs and the end of the text. (GPUI's `closest_index_for_x` gives
        // the end for anywhere on the last glyph.)
        let mut nearest = (self.shaped.len(), (self.shaped.width() - x).abs());
        for glyph in self.shaped.runs.iter().flat_map(|run| &run.glyphs) {
            let distance = (glyph.position.x - x).abs();
            if distance < nearest.1 {
                nearest = (glyph.index, distance);
            }
        }
        self.shown.start + self.map.byte_offset(nearest.0)
    }

    /// The start of the character under `x`, pixels from the start of the line; past the end of
    /// the shaped text, its end.
    pub fn char_at(&self, x: f64) -> usize {
        let x = px((x - self.x) as f32);
        if x < px(0.) {
            return self.shown.start;
        }
        match self.shaped.index_for_x(x) {
            Some(index) => self.shown.start + self.map.byte_at(index),
            None => self.shown.end,
        }
    }

    /// Where the shaped text ends, in pixels from the start of the line.
    pub fn right(&self) -> f64 {
        self.x + f64::from(self.shaped.width())
    }
}

/// Where the parts of the view are in the window.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Geometry {
    pub bounds: Bounds<Pixels>,
    pub gutter: Bounds<Pixels>,
    pub text_area: Bounds<Pixels>,
    /// Where the lines start when nothing is scrolled to the right.
    pub text_left: Pixels,
    pub track: Bounds<Pixels>,
    pub thumb: Option<Bounds<Pixels>>,
}

/// The visible part of the document, laid out for painting.
pub(crate) struct Layout {
    pub geometry: Geometry,
    pub line_height: Pixels,
    /// The shaped text of the visible lines, and where each starts.
    pub lines: Vec<(ShapedLine, Point<Pixels>)>,
    pub numbers: Vec<(ShapedLine, Point<Pixels>)>,
    pub selection: Vec<Bounds<Pixels>>,
    pub selection_color: u32,
    pub caret: Option<Bounds<Pixels>>,
    /// The text area, for the I-beam mouse cursor; the element adds it.
    pub hitbox: Option<Hitbox>,
}
