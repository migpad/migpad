//! The preview of a Markdown document: its blocks laid out as a page — headings, paragraphs,
//! lists, quotes, tables, code, images from files, Mermaid diagrams — in a list that lays out only
//! what is in view. Its text can be selected with the mouse and copied; links are not followed, and
//! nothing is loaded from the network.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Read as _;
use std::ops::Range;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::SystemTime;

use gpui::{
    AnyElement, App, Bounds, ClickEvent, ClipboardItem, Context, FocusHandle, Focusable, Font, FontStyle, FontWeight,
    HighlightStyle, Hsla, Image, ImageFormat, ImageSource, IntoElement, IsZero, KeyBinding, ListAlignment, ListOffset,
    ListState, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, OngoingScroll, PinchEvent, Pixels, Point,
    Render, ScrollHandle, ScrollWheelEvent, SharedString, Size, StrikethroughStyle, StyledImage, StyledText, Task,
    TextLayout, TextRun, UnderlineStyle, Window, canvas, div, fill, img, list, point, prelude::*, px, relative, rgb,
    size,
};
use migpad_core::markdown::{self, Align, Block, Inline, Item, Style, Text, mermaid};
use migpad_editor::actions::{Copy, SelectAll};
use migpad_editor::{EditorSettings, system_font};

use crate::theme::{Appearance, Theme, theme};

const CONTEXT: &str = "MarkdownPreview";

/// The most lines of a block of code the page lays out: a log pasted whole would take seconds a
/// frame. Copying takes it whole.
const CODE_LINES: usize = 2000;

/// The largest file an image is read from.
const PICTURE_SIZE: u64 = 32 << 20;

/// How many times a diagram is enlarged at most over its width on the page.
const MAX_ZOOM: f32 = 8.;

/// The width of the scroll bar of the page, as the editor has it; and the height of those of the
/// areas that scroll sideways.
const BAR: f32 = 12.;
const AREA_BAR: f32 = 10.;

/// The room on each side of the text of a cell of a table.
const CELL_PADDING: f32 = 12.;

/// The keys of the preview: copying, selecting all.
pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-c", Copy, Some(CONTEXT)),
        KeyBinding::new("secondary-a", SelectAll, Some(CONTEXT)),
    ]);
    if cfg!(not(target_os = "macos")) {
        cx.bind_keys([KeyBinding::new("ctrl-insert", Copy, Some(CONTEXT))]);
    }
}

/// A place in the text of the preview: a segment — a run of text drawn as one — and a byte in it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
struct Place {
    segment: usize,
    offset: usize,
}

/// Where a diagram was painted, and how far below the top of its block: it grows down from there.
#[derive(Clone, Copy)]
struct DiagramPlace {
    bounds: Bounds<Pixels>,
    below: Pixels,
}

/// A diagram, by its source and theme.
enum Diagram {
    Drawing,
    /// The image, and its size at 100%.
    Drawn(Arc<Image>, Size<f32>),
    /// What is wrong with its source.
    Failed(SharedString),
}

/// An image of the page, read from its file in the background, and when the file was changed then.
enum Picture {
    Reading,
    Read(Arc<Image>, Option<SystemTime>),
    /// Not one the page shows: not a plain file, too large, of a kind unknown, an SVG that refers to
    /// other files.
    Refused(Option<SystemTime>),
}

pub struct MarkdownPreview {
    focus: FocusHandle,
    list: ListState,
    blocks: Rc<Vec<Block>>,
    /// The first segment of each block, and the text of every segment in order: the selection is
    /// made of them.
    starts: Vec<usize>,
    segments: Vec<SharedString>,
    /// The folder of the document: images with a relative path are there.
    folder: Option<PathBuf>,
    diagrams: HashMap<(String, bool), Diagram>,
    pictures: HashMap<PathBuf, Picture>,
    /// What comes before what is wrong with a diagram, in the language of the interface.
    diagram_error: SharedString,
    /// Where the selection started and where it ends now.
    selection: Option<(Place, Place)>,
    selecting: bool,
    /// Whether the preview has the keyboard in the active window: the selection shows in full color.
    focused: bool,
    /// The segments painted in the last frame, where the mouse can find them.
    layouts: Rc<RefCell<Vec<(usize, TextLayout)>>>,
    /// The areas that scroll sideways — wide tables, enlarged diagrams — by their block and their
    /// number in it; and the next of the block being laid out.
    areas: HashMap<(usize, usize), ScrollHandle>,
    area: (usize, usize),
    /// How many times each diagram is enlarged over its width on the page, by its source.
    zooms: HashMap<String, f32>,
    /// The width the page gives each diagram, by its source, as the last frame laid it out; and
    /// where the diagrams were painted, which a press does not select text over.
    diagram_widths: Rc<RefCell<HashMap<String, Pixels>>>,
    diagram_places: Rc<RefCell<HashMap<String, DiagramPlace>>>,
    /// A scroll bar being dragged.
    dragging: Option<Drag>,
    /// The direction a gesture of the trackpad started in, which it keeps to over an area.
    gesture: OngoingScroll,
    reading: Option<Task<()>>,
}

/// A scroll bar being dragged, and where its thumb was taken.
enum Drag {
    Page { grab: Pixels },
    Area { handle: ScrollHandle, grab: Pixels },
}

impl MarkdownPreview {
    /// A preview that tells what is wrong with a diagram after `diagram_error`.
    pub fn new(diagram_error: SharedString, cx: &mut Context<Self>) -> Self {
        MarkdownPreview {
            focus: cx.focus_handle(),
            // Every block measured: the thumb of the scroll bar is as long as the page is.
            list: ListState::new(0, ListAlignment::Top, px(600.)).measure_all(),
            blocks: Rc::new(Vec::new()),
            starts: Vec::new(),
            segments: Vec::new(),
            folder: None,
            diagrams: HashMap::new(),
            pictures: HashMap::new(),
            diagram_error,
            selection: None,
            selecting: false,
            focused: false,
            layouts: Rc::default(),
            areas: HashMap::new(),
            area: (0, 0),
            zooms: HashMap::new(),
            diagram_widths: Rc::default(),
            diagram_places: Rc::default(),
            dragging: None,
            gesture: OngoingScroll::default(),
            reading: None,
        }
    }

    /// Shows the document `markdown`, whose images with a relative path are in `folder`. It is read
    /// in the background: what showed before stays until then, and where the page was scrolled.
    pub fn set_markdown(&mut self, markdown: String, folder: Option<PathBuf>, cx: &mut Context<Self>) {
        let read = cx.background_spawn(async move {
            // What cannot be read shows nothing rather than ending the program.
            let blocks = catch_unwind(|| markdown::parse(&markdown)).unwrap_or_default();
            let mut segments = Vec::new();
            let starts = blocks
                .iter()
                .map(|block| {
                    let start = segments.len();
                    block_segments(block, &mut |text| segments.push(SharedString::from(text.to_owned())));
                    start
                })
                .collect::<Vec<_>>();
            (blocks, starts, segments)
        });
        self.reading = Some(cx.spawn(async move |preview, cx| {
            let (blocks, starts, segments) = read.await;
            let _ = preview.update(cx, |preview, cx| {
                let scrolled = preview.list.logical_scroll_top();
                let same = preview.segments == segments && preview.folder == folder;
                preview.list.reset(blocks.len());
                preview.list.scroll_to(scrolled);
                preview.forget_unused(&blocks, folder.as_deref(), cx);
                preview.folder = folder;
                (preview.blocks, preview.starts, preview.segments) = (Rc::new(blocks), starts, segments);
                if !same {
                    preview.selection = None;
                    // The areas are numbered by their blocks, which are others now.
                    preview.areas.clear();
                }
                // What was painted was of the text before: the mouse finds nothing until the next frame.
                preview.layouts.borrow_mut().clear();
                preview.reading = None;
                cx.notify();
            });
        }));
    }

    /// Lets go of the diagrams and images no longer in `blocks`, and reads again those whose
    /// files changed.
    fn forget_unused(&mut self, blocks: &[Block], folder: Option<&Path>, cx: &mut Context<Self>) {
        let mut sources = Vec::new();
        let mut urls = Vec::new();
        for block in blocks {
            block_resources(block, &mut sources, &mut urls);
        }
        let mut gone = Vec::new();
        self.diagrams.retain(|(source, _), diagram| {
            let keep = sources.contains(&source.as_str());
            if let (false, Diagram::Drawn(image, _)) = (keep, &diagram) {
                gone.push(image.clone());
            }
            keep
        });
        let files: Vec<PathBuf> = urls.iter().filter_map(|url| markdown::image_file(url, folder)).collect();
        let mut changed = Vec::new();
        self.pictures.retain(|path, picture| {
            let keep = files.contains(path);
            match (keep, &picture) {
                (false, Picture::Read(image, _)) => gone.push(image.clone()),
                (true, Picture::Read(_, modified) | Picture::Refused(modified)) => {
                    changed.push((path.clone(), *modified));
                }
                _ => {}
            }
            keep
        });
        for image in gone {
            ImageSource::Image(image).remove_asset(cx);
        }
        // Images changed by another program show as they are now.
        for (path, modified) in changed {
            let check = cx.background_spawn({
                let path = path.clone();
                async move { std::fs::metadata(&path).ok().and_then(|metadata| metadata.modified().ok()) }
            });
            cx.spawn(async move |preview, cx| {
                if check.await != modified {
                    let _ = preview.update(cx, |preview, cx| preview.read_picture(path, cx));
                }
            })
            .detach();
        }
    }

    /// The text selected, segments on lines of their own.
    fn selected_text(&self) -> Option<String> {
        let (start, end) = self.ordered_selection()?;
        let mut text = String::new();
        for segment in start.segment..=end.segment.min(self.segments.len().saturating_sub(1)) {
            let whole = self.segments.get(segment)?;
            let from = if segment == start.segment { start.offset } else { 0 };
            let to = if segment == end.segment { end.offset } else { whole.len() };
            if segment != start.segment {
                text.push('\n');
            }
            text.push_str(whole.get(from.min(to)..to).unwrap_or_default());
        }
        (!text.is_empty()).then_some(text)
    }

    fn ordered_selection(&self) -> Option<(Place, Place)> {
        let (anchor, head) = self.selection?;
        (anchor != head).then(|| if anchor < head { (anchor, head) } else { (head, anchor) })
    }

    /// The part of the segment `segment` drawn as `text` that is selected, on the boundaries of its
    /// characters.
    fn selected_in(&self, segment: usize, text: &str) -> Option<Range<usize>> {
        let (start, end) = self.ordered_selection()?;
        if segment < start.segment || segment > end.segment {
            return None;
        }
        let from = if segment == start.segment { start.offset } else { 0 };
        let to = if segment == end.segment { end.offset } else { text.len() };
        let (from, to) = (char_boundary(text, from), char_boundary(text, to));
        (from < to).then_some(from..to)
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.selected_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(last) = self.segments.len().checked_sub(1) {
            self.selection = Some((Place::default(), Place { segment: last, offset: self.segments[last].len() }));
            cx.notify();
        }
    }

    /// The place of the text under `position`, or the nearest one: the segment nearest above or
    /// below, then nearest beside — of the cells of a row of a table — and before a segment the
    /// place is above, after one it is below.
    fn place_at(&self, position: Point<Pixels>) -> Option<Place> {
        let layouts = self.layouts.borrow();
        let distance = |layout: &TextLayout| {
            let bounds = layout.bounds();
            let apart = |at: Pixels, from: Pixels, to: Pixels| f32::from((from - at).max(at - to).max(px(0.)));
            (apart(position.y, bounds.top(), bounds.bottom()), apart(position.x, bounds.left(), bounds.right()))
        };
        let (segment, layout) = layouts.iter().min_by(|(_, a), (_, b)| {
            let (a, b) = (distance(a), distance(b));
            a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1))
        })?;
        let bounds = layout.bounds();
        let offset = if position.y < bounds.top() {
            0
        } else if position.y > bounds.bottom() {
            layout.len()
        } else {
            layout.index_for_position(position).unwrap_or_else(|nearest| nearest)
        };
        Some(Place { segment: *segment, offset: offset.min(layout.len()) })
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        // A press on a diagram is for enlarging it, not for selecting text.
        if self.diagram_places.borrow().values().any(|place| place.bounds.contains(&event.position)) {
            return;
        }
        let place = self.place_at(event.position);
        self.selection = match (event.modifiers.shift, self.selection, place) {
            (true, Some((anchor, _)), Some(place)) => Some((anchor, place)),
            (_, _, Some(place)) => Some((place, place)),
            _ => None,
        };
        self.selecting = place.is_some();
        cx.notify();
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.dragging.is_some() {
            if event.pressed_button == Some(MouseButton::Left) {
                self.drag_to(event.position, cx);
            } else {
                self.end_drag();
            }
            return;
        }
        if !self.selecting || event.pressed_button != Some(MouseButton::Left) {
            self.selecting = false;
            return;
        }
        if let (Some((anchor, _)), Some(place)) = (self.selection, self.place_at(event.position)) {
            self.selection = Some((anchor, place));
            cx.notify();
        }
    }

    fn mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.selecting = false;
        self.end_drag();
    }

    fn end_drag(&mut self) {
        if let Some(Drag::Page { .. }) = self.dragging.take() {
            self.list.scrollbar_drag_ended();
        }
    }

    /// A press on the scroll bar of the page: on the thumb it takes it, elsewhere the thumb comes
    /// there.
    fn page_bar_down(&mut self, event: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let viewport = self.list.viewport_bounds();
        let Some((top, height)) = page_thumb(&self.list, viewport.size.height) else { return };
        let y = event.position.y - viewport.top();
        let grab = if y >= top && y <= top + height { y - top } else { height / 2. };
        self.list.scrollbar_drag_started();
        self.dragging = Some(Drag::Page { grab });
        self.drag_to(event.position, cx);
    }

    /// A press on the scroll bar of an area, as on that of the page.
    fn area_bar_down(&mut self, handle: &ScrollHandle, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let viewport = handle.bounds();
        let Some((left, width)) = area_thumb(handle, viewport.size.width) else { return };
        cx.stop_propagation();
        let x = event.position.x - viewport.left();
        let grab = if x >= left && x <= left + width { x - left } else { width / 2. };
        self.dragging = Some(Drag::Area { handle: handle.clone(), grab });
        self.drag_to(event.position, cx);
    }

    /// The thumb being dragged goes to `position`, and what it scrolls with it.
    fn drag_to(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let fraction =
            |at: Pixels, free: Pixels| if free > px(0.) { (f32::from(at) / f32::from(free)).clamp(0., 1.) } else { 0. };
        match &self.dragging {
            Some(Drag::Page { grab }) => {
                let viewport = self.list.viewport_bounds();
                let Some((_, height)) = page_thumb(&self.list, viewport.size.height) else { return };
                let max = self.list.max_offset_for_scrollbar().y;
                let part = fraction(position.y - viewport.top() - *grab, viewport.size.height - height);
                self.list.set_offset_from_scrollbar(point(px(0.), -(max * part)));
            }
            Some(Drag::Area { handle, grab }) => {
                let viewport = handle.bounds();
                let Some((_, width)) = area_thumb(handle, viewport.size.width) else { return };
                let max = handle.max_offset().x;
                let part = fraction(position.x - viewport.left() - *grab, viewport.size.width - width);
                handle.set_offset(point(-(max * part), handle.offset().y));
            }
            None => return,
        }
        cx.notify();
    }

    /// The diagram `source` of the block `block`, of the size `natural` at 100%, `zoom` times as
    /// wide as the page gives it — not narrower — the point under `position` staying where it is.
    #[allow(clippy::too_many_arguments)]
    fn zoom(
        &mut self,
        source: &str,
        block: usize,
        natural: Size<f32>,
        zoom: f32,
        position: Point<Pixels>,
        handle: &ScrollHandle,
        cx: &mut Context<Self>,
    ) {
        let old = self.zooms.get(source).copied().unwrap_or(1.);
        let new = zoom.clamp(1., MAX_ZOOM);
        let given = self.diagram_widths.borrow().get(source).copied();
        let place = self.diagram_places.borrow().get(source).copied();
        let (Some(given), Some(place)) = (given, place) else { return };
        if (new - old).abs() < 0.001 || natural.width <= 0. {
            return;
        }
        if new > 1. {
            self.zooms.insert(source.to_owned(), new);
        } else {
            self.zooms.remove(source);
        }
        let fit = px(natural.width).min(given);
        let (before, after) = (fit * old, fit * new);
        // Sideways the point under the pointer stays; a diagram that fits is centred.
        let viewport = handle.bounds();
        let left = if before <= given { (given - before) / 2. } else { handle.offset().x };
        let x = position.x - viewport.left();
        let across = ((x - left) / before).clamp(0., 1.);
        let offset = if after <= given { px(0.) } else { (x - after * across).clamp(given - after, px(0.)) };
        handle.set_offset(point(offset, px(0.)));
        // The diagram grows down from its top: the page scrolls by what grows above the point.
        // Where the block is now comes from the list, which knows how far it was scrolled since
        // the last frame; the heights it has of the blocks above it are right.
        let height = natural.height / natural.width;
        let top = self.list.bounds_for_item(block).map_or(place.bounds.top(), |item| item.top() + place.below);
        let down = ((position.y - top) / (before * height)).clamp(0., 1.);
        let block_top = position.y - after * height * down - place.below;
        let page_top = self.list.viewport_bounds().top();
        if block_top <= page_top {
            // The block is as high as it is now: the list does not know it yet.
            self.list.scroll_to(ListOffset { item_ix: block, offset_in_item: page_top - block_top });
        } else {
            self.list.scroll_to(ListOffset { item_ix: block, offset_in_item: px(0.) });
            self.list.scroll_by(page_top - block_top);
        }
        cx.notify();
    }

    /// The wheel over an area that scrolls sideways: sideways it scrolls the area and not the page;
    /// up and down, the page. A gesture of the trackpad keeps to the direction it started in, and
    /// Shift turns the wheel of a mouse sideways.
    fn area_wheel(&mut self, handle: &ScrollHandle, event: &ScrollWheelEvent, window: &Window, cx: &mut Context<Self>) {
        // Enlarging a diagram.
        if zooming(event) {
            return;
        }
        let mut delta = event.delta.pixel_delta(window.line_height());
        if event.delta.precise() {
            self.gesture.filter(&mut delta, event.touch_phase);
        } else if delta.x.is_zero() && event.modifiers.shift {
            delta = point(delta.y, px(0.));
        }
        let max = handle.max_offset().x;
        if delta.x.is_zero() || max <= px(0.) {
            return;
        }
        let offset = handle.offset();
        handle.set_offset(point((offset.x + delta.x).clamp(-max, px(0.)), offset.y));
        cx.notify();
        if delta.y.is_zero() {
            cx.stop_propagation();
        }
    }

    /// `content` in an area that scrolls sideways when it is wider than the page, with a scroll bar
    /// then, its thumb over the bottom edge; centred if `center`.
    fn area(&mut self, content: AnyElement, center: bool, cx: &mut Context<Self>) -> (AnyElement, ScrollHandle) {
        let theme = theme(cx);
        let key = self.area;
        self.area.1 += 1;
        let handle = self.areas.entry(key).or_default().clone();
        // Wider than the page in the last frame: room for the bar under it.
        let wide = handle.max_offset().x > px(0.);
        let (painted, thumb_color) = (handle.clone(), theme.editor.thumb);
        let thumb = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                if let Some((left, width)) = area_thumb(&painted, bounds.size.width) {
                    let thumb = Bounds::new(point(bounds.left() + left, bounds.top() + px(2.)), size(width, px(6.)));
                    window.paint_quad(fill(thumb, rgb(thumb_color)).corner_radii(px(3.)));
                }
            },
        )
        .size_full();
        let (pressed, wheeled) = (handle.clone(), handle.clone());
        let id = key.0 * 1024 + key.1;
        let element = div()
            .relative()
            .w_full()
            .child(
                div()
                    .id(("area", id))
                    .w_full()
                    .flex()
                    .when(center, |area| area.justify_center())
                    .when(wide, |area| area.pb(px(AREA_BAR + 2.)))
                    // Scrolled by the wheel here, not as GPUI scrolls: it turns the wheel up and
                    // down sideways, and the page scrolls with the area.
                    .overflow_hidden()
                    .track_scroll(&handle)
                    .on_scroll_wheel(cx.listener(move |preview, event: &ScrollWheelEvent, window, cx| {
                        preview.area_wheel(&wheeled, event, window, cx)
                    }))
                    .child(content),
            )
            .child(
                div().id(("area-bar", id)).absolute().left_0().right_0().bottom_0().h(px(AREA_BAR)).child(thumb).when(
                    wide,
                    |bar| {
                        bar.on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |preview, event, _, cx| preview.area_bar_down(&pressed, event, cx)),
                        )
                    },
                ),
            )
            .into_any_element();
        (element, handle)
    }

    fn block_at(&mut self, index: usize, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        self.area = (index, 0);
        let blocks = self.blocks.clone();
        let Some(block) = blocks.get(index) else { return div().into_any_element() };
        let mut segment = self.starts.get(index).copied().unwrap_or_default();
        let content = self.block(block, 0, &mut segment, window, cx);
        let last = index + 1 == blocks.len();
        // The page is as wide as the window, its margins aside; the scroll bar is in the right one.
        div()
            .w_full()
            .pl(px(32.))
            .pr(px(32. + BAR))
            .when(index == 0, |page| page.pt(px(24.)))
            .pb(px(if last { 48. } else { 14. }))
            .child(content)
            .into_any_element()
    }

    /// The element of `block`, `depth` lists deep; `segment` is its first segment, and after it the
    /// first of the next block. Nested blocks take the whole width given them: laid out to the
    /// width of their content, each level would lay out those below it twice.
    fn block(
        &mut self,
        block: &Block,
        depth: usize,
        segment: &mut usize,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = theme(cx);
        let size = text_size(cx);
        match block {
            Block::Heading { level, content } => {
                let scale = match level {
                    1 => 2.0,
                    2 => 1.5,
                    3 => 1.25,
                    4 => 1.0,
                    5 => 0.875,
                    _ => 0.85,
                };
                div()
                    .w_full()
                    .text_size(size * scale)
                    .line_height(relative(1.25))
                    .font_weight(FontWeight::SEMIBOLD)
                    .when(*level <= 2, |heading| heading.pb(px(6.)).border_b_1().border_color(rgb(theme.border)))
                    .when(*level == 6, |heading| heading.text_color(rgb(theme.text_muted)))
                    .pt(px(if *level <= 2 { 10. } else { 6. }))
                    .child(self.inlines(content, segment, cx))
                    .into_any_element()
            }
            Block::Paragraph(content) => self.inlines(content, segment, cx),
            Block::Code { code, .. } => self.code_block(code, None, Some(segment), cx),
            Block::Html(html) => self.code_block(html, None, Some(segment), cx),
            Block::Mermaid(source) => self.diagram(source, cx),
            Block::Quote(blocks) => {
                let blocks: Vec<AnyElement> =
                    blocks.iter().map(|block| self.block(block, depth, segment, window, cx)).collect();
                div()
                    .w_full()
                    .border_l(px(3.))
                    .border_color(rgb(theme.border))
                    .pl(px(14.))
                    .text_color(rgb(theme.text_muted))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .children(blocks)
                    .into_any_element()
            }
            Block::List { start, items } => {
                let items: Vec<AnyElement> = items
                    .iter()
                    .enumerate()
                    .map(|(i, item)| self.item(item, start.map(|start| start + i as u64), depth, segment, window, cx))
                    .collect();
                div().w_full().flex().flex_col().gap(px(4.)).children(items).into_any_element()
            }
            Block::Table { columns, header, rows } => self.table(columns, header, rows, segment, window, cx),
            Block::Rule => div().my(px(6.)).h(px(2.)).w_full().bg(rgb(theme.border)).into_any_element(),
        }
    }

    fn item(
        &mut self,
        item: &Item,
        number: Option<u64>,
        depth: usize,
        segment: &mut usize,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let marker = match (item.checked, number) {
            (Some(true), _) => "☑".to_owned(),
            (Some(false), _) => "☐".to_owned(),
            (None, Some(number)) => format!("{number}."),
            (None, None) => ["•", "◦", "▪"][depth % 3].to_owned(),
        };
        let blocks: Vec<AnyElement> =
            item.blocks.iter().map(|block| self.block(block, depth + 1, segment, window, cx)).collect();
        div()
            .w_full()
            .flex()
            .gap(px(6.))
            .child(div().flex_none().w(px(22.)).flex().justify_end().child(marker))
            .child(div().flex_1().min_w_0().flex().flex_col().gap(px(4.)).children(blocks))
            .into_any_element()
    }

    /// A table as tables of the web are laid out: each column as wide as its text on one line if
    /// the page has room, else no narrower than its widest word, the room there is going to the
    /// columns that need it most; narrower than its words, the table scrolls sideways. The text is
    /// measured here, as GPUI knows no least width of text that wraps: every cell of a column
    /// has the same least, most and share of the room, which keeps the columns straight.
    fn table(
        &mut self,
        columns: &[Align],
        header: &[Vec<Inline>],
        rows: &[Vec<Vec<Inline>>],
        segment: &mut usize,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = theme(cx);
        let count = columns.len().max(1);
        let all_rows = || std::iter::once((header, true)).chain(rows.iter().map(|row| (row.as_slice(), false)));
        let style = window.text_style();
        let size = style.font_size.to_pixels(window.rem_size());
        let code = monospace(cx);
        let mut widths = vec![(px(0.), px(0.)); count];
        for (row, is_header) in all_rows() {
            let font =
                Font { weight: if is_header { FontWeight::SEMIBOLD } else { style.font_weight }, ..style.font() };
            for (width, cell) in widths.iter_mut().zip(row) {
                let (least, most) = cell_widths(cell, &font, &code, size, window);
                *width = (width.0.max(least), width.1.max(most));
            }
        }
        // The padding and the border of a cell, and a pixel for rounding.
        let room = px(2. * CELL_PADDING + 2.);
        let widths: Vec<(Pixels, Pixels)> =
            widths.into_iter().map(|(least, most)| (least.ceil() + room, most.max(least).ceil() + room)).collect();
        let mut lines = Vec::with_capacity(rows.len() + 1);
        for (row, is_header) in all_rows() {
            let mut cells = Vec::with_capacity(count);
            for (column, &(least, most)) in widths.iter().enumerate() {
                let content = row.get(column).map(|content| self.inlines(content, segment, cx));
                let cell = div()
                    .flex_basis(least)
                    .flex_grow(f32::from(most - least))
                    .flex_shrink_0()
                    .min_w(least)
                    .max_w(most)
                    .px(px(CELL_PADDING))
                    .py(px(6.))
                    .border_r_1()
                    .border_color(rgb(theme.border))
                    // The text goes to its place, rather than its lines in its box: GPUI finds the
                    // characters under the mouse as if lines were to the left. It wraps at the
                    // width of the cell.
                    .flex()
                    .map(|cell| match columns.get(column) {
                        Some(Align::Center) => cell.justify_center(),
                        Some(Align::Right) => cell.justify_end(),
                        _ => cell,
                    })
                    .children(content.map(|content| div().min_w_0().child(content)));
                cells.push(cell);
            }
            let line = div()
                .flex()
                .w_full()
                .border_b_1()
                .border_color(rgb(theme.border))
                .when(is_header, |line| line.font_weight(FontWeight::SEMIBOLD).bg(rgb(theme.bar)))
                .children(cells);
            lines.push(line);
        }
        let (least, most) = widths.iter().fold((px(1.), px(1.)), |sum, width| (sum.0 + width.0, sum.1 + width.1));
        let table = div()
            .flex_none()
            .w(most)
            .max_w_full()
            .min_w(least)
            .border_t_1()
            .border_l_1()
            .border_color(rgb(theme.border))
            .children(lines)
            .into_any_element();
        self.area(table, false, cx).0
    }

    /// Text with images among it: the images on lines of their own.
    fn inlines(&mut self, content: &[Inline], segment: &mut usize, cx: &mut Context<Self>) -> AnyElement {
        let mut children: Vec<AnyElement> = Vec::with_capacity(content.len());
        for inline in content {
            match inline {
                Inline::Text(text) => {
                    children.push(self.text(text, Some(*segment), cx));
                    *segment += 1;
                }
                Inline::Image { url, alt } => children.push(self.image(url, alt, cx)),
            }
        }
        match children.len() {
            1 => children.pop().unwrap_or_else(|| div().into_any_element()),
            _ => div().w_full().flex().flex_col().gap(px(4.)).children(children).into_any_element(),
        }
    }

    /// Text with its styles; a segment, if `segment` is its number, with the part of the selection
    /// in it, which the mouse can find once it is painted.
    fn text(&self, text: &Text, segment: Option<usize>, cx: &App) -> AnyElement {
        let theme = theme(cx);
        let selected = segment.and_then(|segment| self.selected_in(segment, &text.text));
        let selection = if self.focused { theme.editor.selection } else { theme.editor.selection_inactive };
        let mut highlights = Vec::with_capacity(text.runs.len() + 2);
        let mut fonts = Vec::new();
        for run in &text.runs {
            for (range, in_selection) in split(run.range.clone(), selected.clone()) {
                let mut highlight = highlight(run.style, theme);
                if in_selection {
                    highlight.background_color = Some(color(selection));
                }
                if run.style.code {
                    fonts.push((range.clone(), monospace(cx)));
                }
                highlights.push((range, highlight));
            }
        }
        let styled = StyledText::new(SharedString::from(text.text.clone()))
            .with_highlights(highlights)
            .with_font_family_overrides(fonts);
        let Some(segment) = segment else { return styled.into_any_element() };
        // Painted, the layout knows where its text is: the bounds come with prepainting, which is
        // done for every element before any paints.
        let (layouts, layout) = (self.layouts.clone(), styled.layout().clone());
        let found = canvas(|_, _, _| {}, move |_, _, _, _| layouts.borrow_mut().push((segment, layout)));
        div().relative().child(styled).child(found.absolute().size_0()).into_any_element()
    }

    /// Code on a background of its own — the first lines of a long one — with what is wrong with
    /// it if `error` tells; its text a segment if `segment` is its number, and then the next.
    fn code_block(&self, code: &str, error: Option<&str>, segment: Option<&mut usize>, cx: &App) -> AnyElement {
        let theme = theme(cx);
        let code = code.strip_suffix('\n').unwrap_or(code);
        let shown = match code.match_indices('\n').nth(CODE_LINES - 1) {
            Some((end, _)) => &code[..end],
            None => code,
        };
        let text = self.text(&plain(shown), segment.as_deref().copied(), cx);
        if let Some(segment) = segment {
            *segment += 1;
        }
        div()
            .w_full()
            .p(px(12.))
            .rounded(px(6.))
            .bg(rgb(theme.editor.gutter))
            .font_family(monospace(cx))
            .text_size(text_size(cx) * 0.9)
            .line_height(relative(1.45))
            .child(text)
            .when(shown.len() < code.len(), |block| block.child(div().text_color(rgb(theme.text_muted)).child("…")))
            .when_some(error, |block, error| {
                block.child(div().pt(px(8.)).text_color(rgb(theme.error_edge)).child(error.to_owned()))
            })
            .into_any_element()
    }

    /// An image from a file, read in the background the first time it shows; one from anywhere
    /// else, or not an image the page shows, shows as its text.
    fn image(&mut self, url: &str, alt: &str, cx: &mut Context<Self>) -> AnyElement {
        let theme = theme(cx);
        let alt = SharedString::from(if alt.is_empty() { url.to_owned() } else { alt.to_owned() });
        let missing = move || div().text_color(rgb(theme.text_muted)).child(format!("[{alt}]")).into_any_element();
        let Some(path) = markdown::image_file(url, self.folder.as_deref()) else { return missing() };
        match self.pictures.get(&path) {
            Some(Picture::Read(image, _)) => img(image.clone()).max_w_full().with_fallback(missing).into_any_element(),
            Some(Picture::Refused(_)) => missing(),
            Some(Picture::Reading) => div().into_any_element(),
            None => {
                self.read_picture(path, cx);
                div().into_any_element()
            }
        }
    }

    /// Reads the image of the file `path` in the background.
    fn read_picture(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if let Some(Picture::Read(image, _)) = self.pictures.insert(path.clone(), Picture::Reading) {
            ImageSource::Image(image).remove_asset(cx);
        }
        let read = cx.background_spawn({
            let path = path.clone();
            async move { read_picture(&path) }
        });
        cx.spawn(async move |preview, cx| {
            let picture = read.await;
            let _ = preview.update(cx, |preview, cx| {
                // Unless the document has no such image any more.
                if let Some(reading) = preview.pictures.get_mut(&path) {
                    *reading = picture;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// A Mermaid diagram, drawn in the background the first time it shows in a theme; until then a
    /// place for it, and its source with what is wrong if it cannot be drawn.
    fn diagram(&mut self, source: &str, cx: &mut Context<Self>) -> AnyElement {
        let dark = theme(cx).appearance == Appearance::Dark;
        let key = (source.to_owned(), dark);
        match self.diagrams.get(&key) {
            Some(Diagram::Drawn(image, natural)) => {
                let (image, natural) = (image.clone(), *natural);
                return self.zoomable(source, image, natural, cx);
            }
            Some(Diagram::Failed(error)) => {
                let error = format!("{} {error}", self.diagram_error);
                return self.code_block(source, Some(&error), None, cx);
            }
            Some(Diagram::Drawing) => {}
            None => {
                self.diagrams.insert(key.clone(), Diagram::Drawing);
                let source = source.to_owned();
                // A diagram that cannot be drawn says so rather than ending the program.
                let draw = cx.background_spawn(async move {
                    catch_unwind(AssertUnwindSafe(|| mermaid::render(&source, dark)))
                        .unwrap_or_else(|_| Err("merman stopped".to_owned()))
                });
                cx.spawn(async move |preview, cx| {
                    let diagram = match draw.await {
                        Ok(svg) => Diagram::Drawn(
                            Arc::new(Image::from_bytes(ImageFormat::Svg, svg.text.into_bytes())),
                            size(svg.width, svg.height),
                        ),
                        Err(error) => Diagram::Failed(error.into()),
                    };
                    let _ = preview.update(cx, |preview, cx| {
                        // Unless the document has no such diagram any more.
                        if let Some(drawing) = preview.diagrams.get_mut(&key) {
                            *drawing = diagram;
                            cx.notify();
                        }
                    });
                })
                .detach();
            }
        }
        div().h(px(80.)).w_full().into_any_element()
    }

    /// A drawn diagram: as wide as the page, or less if it is smaller, enlarged by a pinch or by
    /// the wheel with Cmd — Ctrl on Windows and Linux — and back by a double click; enlarged past
    /// the page it scrolls sideways.
    fn zoomable(&mut self, source: &str, image: Arc<Image>, natural: Size<f32>, cx: &mut Context<Self>) -> AnyElement {
        let zoom = self.zooms.get(source).copied().unwrap_or(1.);
        let given = self.diagram_widths.borrow().get(source).copied();
        // Laid out once, the width the page gives is known; the first time, the image just fits.
        let shown = given.map(|given| px(natural.width).min(given) * zoom);
        let picture = match shown {
            Some(shown) => img(image).w(shown).flex_none().into_any_element(),
            None => img(image).max_w_full().into_any_element(),
        };
        let fits = match (shown, given) {
            (Some(shown), Some(given)) => shown <= given + px(0.5),
            _ => true,
        };
        let (block, id) = (self.area.0, self.area.0 * 1024 + self.area.1);
        let (widths, places, key) = (self.diagram_widths.clone(), self.diagram_places.clone(), source.to_owned());
        let list = self.list.clone();
        let measure = canvas(
            move |area, window, _| {
                let width = area.size.width;
                if widths.borrow_mut().insert(key.clone(), width) != Some(width) {
                    // The width changed: the image takes the new one in the next frame.
                    window.refresh();
                }
                (key, area)
            },
            move |_, (key, area), _, _| {
                let below = list.bounds_for_item(block).map_or(px(0.), |item| area.top() - item.top());
                places.borrow_mut().insert(key, DiagramPlace { bounds: area, below });
            },
        )
        // Over the diagram: with no place given, it would be below it, where the flow puts it.
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        let (area, handle) = self.area(picture, fits, cx);
        let (wheel, pinch, click) = (handle.clone(), handle.clone(), handle);
        let (wheel_source, pinch_source, click_source) = (source.to_owned(), source.to_owned(), source.to_owned());
        div()
            .id(("diagram", id))
            .relative()
            .w_full()
            .child(area)
            .child(measure)
            .on_scroll_wheel(cx.listener(move |preview, event: &ScrollWheelEvent, _, cx| {
                if zooming(event) {
                    let delta = f32::from(event.delta.pixel_delta(px(20.)).y);
                    let zoom = preview.zooms.get(&wheel_source).copied().unwrap_or(1.) * (delta * 0.005).exp();
                    preview.zoom(&wheel_source, block, natural, zoom, event.position, &wheel, cx);
                    cx.stop_propagation();
                }
            }))
            .on_pinch(cx.listener(move |preview, event: &PinchEvent, _, cx| {
                let zoom = preview.zooms.get(&pinch_source).copied().unwrap_or(1.) * (1. + event.delta);
                preview.zoom(&pinch_source, block, natural, zoom, event.position, &pinch, cx);
                cx.stop_propagation();
            }))
            .on_click(cx.listener(move |preview, event: &ClickEvent, _, cx| {
                if event.click_count() == 2 {
                    preview.zoom(&click_source, block, natural, 1., event.position(), &click, cx);
                }
            }))
            .into_any_element()
    }
}

impl Focusable for MarkdownPreview {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for MarkdownPreview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.focused = self.focus.contains_focused(window, cx) && window.is_window_active();
        let theme = theme(cx);
        // Painted before the blocks: the segments and diagrams of the last frame go, those painted now
        // come.
        let (layouts, diagrams) = (self.layouts.clone(), self.diagram_places.clone());
        let forget = canvas(
            |_, _, _| {},
            move |_, _, _, _| {
                layouts.borrow_mut().clear();
                diagrams.borrow_mut().clear();
            },
        )
        .absolute()
        .size_0();
        // The scroll bar of the page, as the editor has it, painted as the list is laid out now.
        let (painted, track, thumb) = (self.list.clone(), theme.editor.track, theme.editor.thumb);
        let bar = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                if let Some((top, height)) = page_thumb(&painted, bounds.size.height) {
                    window.paint_quad(fill(bounds, rgb(track)));
                    let thumb_bounds =
                        Bounds::new(point(bounds.left() + px(2.), bounds.top() + top), size(px(BAR - 4.), height));
                    window.paint_quad(fill(thumb_bounds, rgb(thumb)));
                }
            },
        )
        .size_full();
        let scrolls = self.list.max_offset_for_scrollbar().y > px(0.);
        div()
            .id("markdown-preview")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::select_all))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::mouse_up))
            .relative()
            .size_full()
            .bg(rgb(theme.editor.background))
            .text_color(rgb(theme.editor.text))
            .text_size(text_size(cx))
            .line_height(relative(1.5))
            .child(forget)
            .child(
                list(self.list.clone(), cx.processor(|preview, index, window, cx| preview.block_at(index, window, cx)))
                    .size_full(),
            )
            .child(
                div()
                    .id("page-bar")
                    .absolute()
                    .top_0()
                    .right_0()
                    .bottom_0()
                    .w(px(BAR))
                    .child(bar)
                    .when(scrolls, |bar| bar.on_mouse_down(MouseButton::Left, cx.listener(Self::page_bar_down))),
            )
    }
}

/// The least width the cell `content` takes — its widest word, which does not break — and the
/// most, its lines unwrapped: in `font`, and in `code` for code, at `size`.
fn cell_widths(
    content: &[Inline],
    font: &Font,
    code: &SharedString,
    size: Pixels,
    window: &Window,
) -> (Pixels, Pixels) {
    let (mut least, mut most) = (px(0.), px(0.));
    for inline in content {
        let alt;
        let (text, styled) = match inline {
            Inline::Text(text) => (text.text.as_str(), Some(text)),
            Inline::Image { alt: text, .. } => {
                alt = format!("[{text}]");
                (alt.as_str(), None)
            }
        };
        let mut start = 0;
        for line in text.split('\n') {
            let range = start..start + line.len();
            start = range.end + 1;
            if line.is_empty() {
                continue;
            }
            let runs = match styled {
                Some(styled) => text_runs(styled, range, font, code),
                None => vec![text_run(line.len(), font.clone())],
            };
            let shaped = window.text_system().shape_line(SharedString::from(line.to_owned()), size, &runs, None);
            most = most.max(shaped.width);
            // A line breaks before a word after a space, the space staying with the word before;
            // and around the characters of Chinese, Japanese and Korean, which have no spaces.
            let mut word = 0;
            let mut previous = '\0';
            for (at, c) in line.char_indices() {
                if at > 0 && ((previous == ' ' && c != ' ') || wide(c) || wide(previous)) {
                    least = least.max(shaped.x_for_index(at) - shaped.x_for_index(word));
                    word = at;
                }
                previous = c;
            }
            least = least.max(shaped.width - shaped.x_for_index(word));
        }
    }
    (least, most)
}

/// The runs of the part `range` of `text`: in `font`, bold, italic or code as its styles say.
fn text_runs(text: &Text, range: Range<usize>, font: &Font, code: &SharedString) -> Vec<TextRun> {
    let mut runs = Vec::new();
    let mut at = range.start;
    for run in &text.runs {
        let (from, to) = (run.range.start.max(at), run.range.end.min(range.end));
        if from >= to {
            continue;
        }
        if from > at {
            runs.push(text_run(from - at, font.clone()));
        }
        let mut styled = font.clone();
        if run.style.strong {
            styled.weight = FontWeight::BOLD;
        }
        if run.style.emphasis {
            styled.style = FontStyle::Italic;
        }
        if run.style.code {
            styled.family = code.clone();
        }
        runs.push(text_run(to - from, styled));
        at = to;
    }
    if range.end > at {
        runs.push(text_run(range.end - at, font.clone()));
    }
    runs
}

fn text_run(len: usize, font: Font) -> TextRun {
    TextRun { len, font, color: Hsla::default(), background_color: None, underline: None, strikethrough: None }
}

/// Whether `c` is of the scripts that break between any two characters: Chinese, Japanese, Korean.
fn wide(c: char) -> bool {
    matches!(c,
        '\u{1100}'..='\u{11FF}' | '\u{2E80}'..='\u{A4CF}' | '\u{AC00}'..='\u{D7AF}' | '\u{F900}'..='\u{FAFF}'
        | '\u{FE30}'..='\u{FE4F}' | '\u{FF00}'..='\u{FFEF}' | '\u{20000}'..='\u{3FFFD}')
}

/// Whether the wheel enlarges a diagram: with Cmd on macOS, Ctrl on Windows and Linux.
fn zooming(event: &ScrollWheelEvent) -> bool {
    if cfg!(target_os = "macos") { event.modifiers.platform } else { event.modifiers.control }
}

/// The thumb of the scroll bar of a page in a track `track` high: its top and its height; none if
/// the page fits.
fn page_thumb(list: &ListState, track: Pixels) -> Option<(Pixels, Pixels)> {
    let max = list.max_offset_for_scrollbar().y;
    if max <= px(0.) || track <= px(0.) {
        return None;
    }
    let top = (-list.scroll_px_offset_for_scrollbar().y).clamp(px(0.), max);
    let height = (track * (f32::from(track) / f32::from(track + max))).max(px(24.)).min(track);
    Some(((track - height) * (f32::from(top) / f32::from(max)), height))
}

/// The thumb of the scroll bar of an area in a track `track` wide: its left and its width; none if
/// the area fits.
fn area_thumb(handle: &ScrollHandle, track: Pixels) -> Option<(Pixels, Pixels)> {
    let max = handle.max_offset().x;
    let viewport = handle.bounds().size.width;
    if max <= px(0.) || track <= px(0.) || viewport <= px(0.) {
        return None;
    }
    let offset = (-handle.offset().x).clamp(px(0.), max);
    let width = (track * (f32::from(viewport) / f32::from(viewport + max))).max(px(24.)).min(track);
    Some(((track - width) * (f32::from(offset) / f32::from(max)), width))
}

/// The calls `text` gets for each segment of `block`, in order: what the preview draws as one run
/// of text.
fn block_segments(block: &Block, text: &mut impl FnMut(&str)) {
    let inlines = |content: &[Inline], text: &mut dyn FnMut(&str)| {
        for inline in content {
            if let Inline::Text(run) = inline {
                text(&run.text);
            }
        }
    };
    match block {
        Block::Heading { content, .. } | Block::Paragraph(content) => inlines(content, text),
        Block::Code { code, .. } => text(code.strip_suffix('\n').unwrap_or(code)),
        Block::Html(html) => text(html.strip_suffix('\n').unwrap_or(html)),
        Block::Mermaid(_) | Block::Rule => {}
        Block::Quote(blocks) => blocks.iter().for_each(|block| block_segments(block, text)),
        Block::List { items, .. } => {
            items.iter().flat_map(|item| &item.blocks).for_each(|block| block_segments(block, text))
        }
        Block::Table { columns, header, rows } => {
            for row in std::iter::once(header).chain(rows) {
                for column in 0..columns.len().max(1) {
                    if let Some(content) = row.get(column) {
                        inlines(content, text);
                    }
                }
            }
        }
    }
}

/// The sources of the diagrams of `block`, and the URLs of its images.
fn block_resources<'a>(block: &'a Block, sources: &mut Vec<&'a str>, urls: &mut Vec<&'a str>) {
    let mut inlines = |content: &'a [Inline]| {
        for inline in content {
            if let Inline::Image { url, .. } = inline {
                urls.push(url);
            }
        }
    };
    match block {
        Block::Mermaid(source) => sources.push(source),
        Block::Heading { content, .. } | Block::Paragraph(content) => inlines(content),
        Block::Table { header, rows, .. } => {
            std::iter::once(header).chain(rows).flatten().for_each(|cell| inlines(cell))
        }
        Block::Quote(blocks) => blocks.iter().for_each(|block| block_resources(block, sources, urls)),
        Block::List { items, .. } => {
            items.iter().flat_map(|item| &item.blocks).for_each(|block| block_resources(block, sources, urls))
        }
        Block::Code { .. } | Block::Html(_) | Block::Rule => {}
    }
}

/// The image of the file `path`, if it is one the page shows: a plain file, not too large, of a
/// kind GPUI draws; an SVG that refers to no other file.
fn read_picture(path: &Path) -> Picture {
    let modified = std::fs::metadata(path).ok().and_then(|metadata| metadata.modified().ok());
    let read = || {
        // Not a device, nor a pipe, which would be read forever or wait for a writer.
        if !std::fs::metadata(path).ok()?.is_file() {
            return None;
        }
        let file = std::fs::File::open(path).ok()?;
        let metadata = file.metadata().ok()?;
        if !metadata.is_file() || metadata.len() > PICTURE_SIZE {
            return None;
        }
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.take(PICTURE_SIZE).read_to_end(&mut bytes).ok()?;
        let format = picture_format(&bytes)?;
        if format == ImageFormat::Svg && !self_contained(&String::from_utf8_lossy(&bytes)) {
            return None;
        }
        Some(Arc::new(Image::from_bytes(format, bytes)))
    };
    match read() {
        Some(image) => Picture::Read(image, modified),
        None => Picture::Refused(modified),
    }
}

/// The kind of the image `bytes` are, by how they start.
fn picture_format(bytes: &[u8]) -> Option<ImageFormat> {
    let format = match bytes {
        [0x89, b'P', b'N', b'G', ..] => ImageFormat::Png,
        [0xff, 0xd8, 0xff, ..] => ImageFormat::Jpeg,
        [b'G', b'I', b'F', b'8', ..] => ImageFormat::Gif,
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => ImageFormat::Webp,
        [b'B', b'M', ..] => ImageFormat::Bmp,
        [b'I', b'I', 0x2a, 0, ..] | [b'M', b'M', 0, 0x2a, ..] => ImageFormat::Tiff,
        [0, 0, 1, 0, ..] => ImageFormat::Ico,
        _ => {
            let head = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]);
            let head = head.trim_start_matches('\u{feff}').trim_start();
            if head.starts_with('<') && head.contains("<svg") {
                ImageFormat::Svg
            } else {
                return None;
            }
        }
    };
    Some(format)
}

/// Whether the SVG `svg` refers to no file and no address: drawing it, resvg reads what it refers
/// to, on the network too.
fn self_contained(svg: &str) -> bool {
    let local = |reference: &str| {
        let reference = reference.trim().trim_matches(['"', '\'']).trim();
        reference.is_empty() || reference.starts_with('#') || reference.starts_with("data:")
    };
    let hrefs = svg.match_indices("href").all(|(at, _)| {
        let rest = svg[at + 4..].trim_start();
        let Some(rest) = rest.strip_prefix('=') else { return true };
        let rest = rest.trim_start();
        let Some(quote) = rest.chars().next().filter(|c| *c == '"' || *c == '\'') else { return true };
        rest[1..].split(quote).next().is_none_or(local)
    });
    let urls = svg.match_indices("url(").all(|(at, _)| svg[at + 4..].split(')').next().is_none_or(local));
    hrefs && urls && !svg.contains("@import")
}

/// `range` cut by `selected` into parts, each with whether it is selected.
fn split(range: Range<usize>, selected: Option<Range<usize>>) -> Vec<(Range<usize>, bool)> {
    let Some(selected) = selected else { return vec![(range, false)] };
    let (from, to) = (selected.start.clamp(range.start, range.end), selected.end.clamp(range.start, range.end));
    [(range.start..from, false), (from..to, true), (to..range.end, false)]
        .into_iter()
        .filter(|(part, _)| !part.is_empty())
        .collect()
}

/// The boundary of a character of `text` at or before `offset`.
fn char_boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn highlight(style: Style, theme: &Theme) -> HighlightStyle {
    HighlightStyle {
        font_weight: style.strong.then_some(FontWeight::BOLD),
        font_style: style.emphasis.then_some(FontStyle::Italic),
        strikethrough: style.strikethrough.then_some(StrikethroughStyle { thickness: px(1.), color: None }),
        underline: style.link.then_some(UnderlineStyle { thickness: px(1.), color: None, wavy: false }),
        color: style.link.then(|| color(theme.accent)),
        background_color: style.code.then(|| color(theme.editor.gutter)),
        ..HighlightStyle::default()
    }
}

/// Text of one style, as code is.
fn plain(text: &str) -> Text {
    Text { text: text.to_owned(), runs: vec![markdown::Run { range: 0..text.len(), style: Style::default() }] }
}

fn color(rgb_value: u32) -> Hsla {
    rgb(rgb_value).into()
}

/// The size of the text of the page: a little larger than that of the editor.
fn text_size(cx: &App) -> Pixels {
    px(EditorSettings::current(cx).font_size.max(12.) + 2.)
}

/// The font of code: that of the editor.
fn monospace(cx: &App) -> SharedString {
    EditorSettings::current(cx).font.unwrap_or_else(|| system_font().into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_cuts_runs() {
        assert_eq!(split(0..10, None), [(0..10, false)]);
        assert_eq!(split(0..10, Some(2..5)), [(0..2, false), (2..5, true), (5..10, false)]);
        assert_eq!(split(4..8, Some(0..6)), [(4..6, true), (6..8, false)]);
        assert_eq!(split(4..8, Some(8..9)), [(4..8, false)]);
        assert_eq!(split(4..8, Some(0..20)), [(4..8, true)]);
        // Inside a character a cut goes back to where it starts.
        assert_eq!(char_boundary("аб", 1), 0);
        assert_eq!(char_boundary("аб", 3), 2);
        assert_eq!(char_boundary("аб", 9), 4);
    }

    #[test]
    fn segments_follow_the_order_of_drawing() {
        let blocks = markdown::parse(
            "# a\n\nb ![i](x.png) c\n\n- d\n  > e\n\n| f | g |\n|---|---|\n| h |\n\n```mermaid\nx\n```\n\n```\ni\n```",
        );
        let mut segments = Vec::new();
        for block in &blocks {
            block_segments(block, &mut |text| segments.push(text.to_owned()));
        }
        assert_eq!(segments, ["a", "b ", " c", "d", "e", "f", "g", "h", "i"]);
        let (mut sources, mut urls) = (Vec::new(), Vec::new());
        for block in &blocks {
            block_resources(block, &mut sources, &mut urls);
        }
        assert_eq!((sources, urls), (vec!["x\n"], vec!["x.png"]));
    }

    #[test]
    fn pictures_by_their_kind() {
        assert_eq!(picture_format(b"\x89PNG\r\n\x1a\n"), Some(ImageFormat::Png));
        assert_eq!(picture_format(b"\xff\xd8\xff\xe0"), Some(ImageFormat::Jpeg));
        assert_eq!(picture_format(b"RIFF\0\0\0\0WEBPVP8 "), Some(ImageFormat::Webp));
        assert_eq!(picture_format(b"\xef\xbb\xbf<?xml version=\"1.0\"?>\n<svg xmlns=\"\"/>"), Some(ImageFormat::Svg));
        assert_eq!(picture_format(b"not an image"), None);
        assert_eq!(picture_format(b""), None);
    }

    #[test]
    fn svg_that_refers_elsewhere_is_not_drawn() {
        assert!(self_contained(
            r##"<svg><use href="#a"/><rect fill="url(#g)"/><image href="data:image/png;base64,AA"/></svg>"##
        ));
        assert!(!self_contained(r#"<svg><image href="file:///etc/x.png"/></svg>"#));
        assert!(!self_contained(r#"<svg><image xlink:href = 'https://a/b.png'/></svg>"#));
        assert!(!self_contained(r#"<svg><image href="//host/share/a.png"/></svg>"#));
        assert!(!self_contained(r#"<svg><rect style="fill: url(other.svg#g)"/></svg>"#));
        assert!(!self_contained(r#"<svg><style>@import "a.css";</style></svg>"#));
    }

    #[test]
    fn only_plain_files_are_read() {
        let dir = std::env::temp_dir().join(format!("migpad-pictures-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let png = dir.join("a.png");
        std::fs::write(&png, b"\x89PNG\r\n\x1a\nrest").unwrap();
        assert!(matches!(read_picture(&png), Picture::Read(..)));
        let text = dir.join("a.txt");
        std::fs::write(&text, b"text").unwrap();
        assert!(matches!(read_picture(&text), Picture::Refused(_)));
        assert!(matches!(read_picture(&dir), Picture::Refused(_)));
        assert!(matches!(read_picture(&dir.join("none.png")), Picture::Refused(None)));
        #[cfg(unix)]
        assert!(matches!(read_picture(Path::new("/dev/zero")), Picture::Refused(_)));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
