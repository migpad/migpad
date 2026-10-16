//! The preview of a Markdown document: its blocks laid out as a page — headings, paragraphs,
//! lists, quotes, tables, code, images from files, Mermaid diagrams — in a list that lays out only
//! what is in view. Its text can be selected with the mouse and copied; links are not followed, and
//! nothing is loaded from the network.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, App, ClipboardItem, Context, FocusHandle, Focusable, FontStyle, FontWeight, HighlightStyle, Hsla,
    Image, ImageFormat, IntoElement, KeyBinding, ListAlignment, ListState, MouseButton, MouseDownEvent, MouseMoveEvent,
    Pixels, Point, Render, SharedString, StrikethroughStyle, StyledImage, StyledText, Task, TextLayout, UnderlineStyle,
    Window, canvas, div, img, list, prelude::*, px, relative, rgb,
};
use migpad_core::markdown::{self, Align, Block, Inline, Item, Style, Text, mermaid};
use migpad_editor::actions::{Copy, SelectAll};
use migpad_editor::{EditorSettings, system_font};

use crate::theme::{Appearance, Theme, theme};

const CONTEXT: &str = "MarkdownPreview";

/// The widest the text of the page gets: lines longer than that are hard to read.
const PAGE_WIDTH: f32 = 860.;

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

/// A diagram, by its source and theme.
enum Diagram {
    Drawing,
    Drawn(Arc<Image>),
    /// What is wrong with its source.
    Failed(SharedString),
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
    /// What comes before what is wrong with a diagram, in the language of the interface.
    diagram_error: SharedString,
    /// Where the selection started and where it ends now.
    selection: Option<(Place, Place)>,
    selecting: bool,
    /// Whether the preview has the keyboard in the active window: the selection shows in full color.
    focused: bool,
    /// The segments painted in the last frame, where the mouse can find them.
    layouts: Rc<RefCell<Vec<(usize, TextLayout)>>>,
    reading: Option<Task<()>>,
}

impl MarkdownPreview {
    /// A preview that tells what is wrong with a diagram after `diagram_error`.
    pub fn new(diagram_error: SharedString, cx: &mut Context<Self>) -> Self {
        MarkdownPreview {
            focus: cx.focus_handle(),
            list: ListState::new(0, ListAlignment::Top, px(600.)),
            blocks: Rc::new(Vec::new()),
            starts: Vec::new(),
            segments: Vec::new(),
            folder: None,
            diagrams: HashMap::new(),
            diagram_error,
            selection: None,
            selecting: false,
            focused: false,
            layouts: Rc::default(),
            reading: None,
        }
    }

    /// Shows the document `markdown`, whose images with a relative path are in `folder`. It is read
    /// in the background: what showed before stays until then, and where the page was scrolled.
    pub fn set_markdown(&mut self, markdown: String, folder: Option<PathBuf>, cx: &mut Context<Self>) {
        self.folder = folder;
        let read = cx.background_spawn(async move {
            let blocks = markdown::parse(&markdown);
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
                let same = preview.segments == segments;
                preview.list.reset(blocks.len());
                preview.list.scroll_to(scrolled);
                // Diagrams no longer in the document go.
                let sources: Vec<&str> = blocks.iter().flat_map(diagram_sources).collect();
                preview.diagrams.retain(|(source, _), _| sources.contains(&source.as_str()));
                (preview.blocks, preview.starts, preview.segments) = (Rc::new(blocks), starts, segments);
                if !same {
                    preview.selection = None;
                }
                preview.reading = None;
                cx.notify();
            });
        }));
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

    /// The part of the segment `segment`, `len` bytes long, that is selected.
    fn selected_in(&self, segment: usize, len: usize) -> Option<Range<usize>> {
        let (start, end) = self.ordered_selection()?;
        if segment < start.segment || segment > end.segment {
            return None;
        }
        let from = if segment == start.segment { start.offset } else { 0 };
        let to = if segment == end.segment { end.offset } else { len };
        (from < to).then_some(from..to.min(len))
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

    /// The place of the text under `position`, or the nearest one: before a segment above it,
    /// after one below it.
    fn place_at(&self, position: Point<Pixels>) -> Option<Place> {
        let layouts = self.layouts.borrow();
        let distance = |layout: &TextLayout| {
            let bounds = layout.bounds();
            if position.y < bounds.top() {
                bounds.top() - position.y
            } else if position.y > bounds.bottom() {
                position.y - bounds.bottom()
            } else {
                px(0.)
            }
        };
        let (segment, layout) =
            layouts.iter().min_by(|(_, a), (_, b)| f32::from(distance(a)).total_cmp(&f32::from(distance(b))))?;
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
        if !self.selecting || event.pressed_button != Some(MouseButton::Left) {
            self.selecting = false;
            return;
        }
        if let (Some((anchor, _)), Some(place)) = (self.selection, self.place_at(event.position)) {
            self.selection = Some((anchor, place));
            cx.notify();
        }
    }

    fn block_at(&mut self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let blocks = self.blocks.clone();
        let Some(block) = blocks.get(index) else { return div().into_any_element() };
        let mut segment = self.starts.get(index).copied().unwrap_or_default();
        let content = self.block(block, 0, &mut segment, cx);
        let last = index + 1 == blocks.len();
        div()
            .w_full()
            .flex()
            .justify_center()
            .child(
                div()
                    .w_full()
                    .max_w(px(PAGE_WIDTH))
                    .px(px(32.))
                    .when(index == 0, |page| page.pt(px(24.)))
                    .pb(px(if last { 48. } else { 14. }))
                    .child(content),
            )
            .into_any_element()
    }

    /// The element of `block`, `depth` lists deep; `segment` is its first segment, and after it the
    /// first of the next block.
    fn block(&mut self, block: &Block, depth: usize, segment: &mut usize, cx: &mut Context<Self>) -> AnyElement {
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
            Block::Code { code, .. } => self.code_block(&plain(code), None, Some(segment), cx),
            Block::Html(html) => self.code_block(&plain(html), None, Some(segment), cx),
            Block::Mermaid(source) => self.diagram(source, cx),
            Block::Quote(blocks) => div()
                .border_l(px(3.))
                .border_color(rgb(theme.border))
                .pl(px(14.))
                .text_color(rgb(theme.text_muted))
                .flex()
                .flex_col()
                .gap(px(10.))
                .children(blocks.iter().map(|block| self.block(block, depth, segment, cx)).collect::<Vec<_>>())
                .into_any_element(),
            Block::List { start, items } => {
                let items: Vec<AnyElement> = items
                    .iter()
                    .enumerate()
                    .map(|(i, item)| self.item(item, start.map(|start| start + i as u64), depth, segment, cx))
                    .collect();
                div().flex().flex_col().gap(px(4.)).children(items).into_any_element()
            }
            Block::Table { columns, header, rows } => self.table(columns, header, rows, segment, cx),
            Block::Rule => div().my(px(6.)).h(px(2.)).w_full().bg(rgb(theme.border)).into_any_element(),
        }
    }

    fn item(
        &mut self,
        item: &Item,
        number: Option<u64>,
        depth: usize,
        segment: &mut usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let marker = match (item.checked, number) {
            (Some(true), _) => "☑".to_owned(),
            (Some(false), _) => "☐".to_owned(),
            (None, Some(number)) => format!("{number}."),
            (None, None) => ["•", "◦", "▪"][depth % 3].to_owned(),
        };
        let blocks: Vec<AnyElement> =
            item.blocks.iter().map(|block| self.block(block, depth + 1, segment, cx)).collect();
        div()
            .flex()
            .gap(px(6.))
            .child(div().flex_none().min_w(px(22.)).flex().justify_end().child(marker))
            .child(div().flex_1().min_w_0().flex().flex_col().gap(px(4.)).children(blocks))
            .into_any_element()
    }

    fn table(
        &mut self,
        columns: &[Align],
        header: &[Vec<Inline>],
        rows: &[Vec<Vec<Inline>>],
        segment: &mut usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = theme(cx);
        let count = columns.len().max(1);
        let mut cells = Vec::with_capacity(count * (rows.len() + 1));
        for (row, is_header) in std::iter::once((header, true)).chain(rows.iter().map(|row| (row.as_slice(), false))) {
            for column in 0..count {
                let content = row.get(column).map(|content| self.inlines(content, segment, cx));
                let cell = div()
                    .px(px(12.))
                    .py(px(6.))
                    .border_b_1()
                    .border_r_1()
                    .border_color(rgb(theme.border))
                    .when(is_header, |cell| cell.font_weight(FontWeight::SEMIBOLD).bg(rgb(theme.bar)))
                    .map(|cell| match columns.get(column) {
                        Some(Align::Center) => cell.text_center(),
                        Some(Align::Right) => cell.text_right(),
                        _ => cell,
                    })
                    .children(content);
                cells.push(cell);
            }
        }
        div()
            .grid()
            .grid_cols_min_content(count as u16)
            .border_t_1()
            .border_l_1()
            .border_color(rgb(theme.border))
            .children(cells)
            .into_any_element()
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
            _ => div().flex().flex_col().gap(px(4.)).children(children).into_any_element(),
        }
    }

    /// Text with its styles; a segment, if `segment` is its number, with the part of the selection
    /// in it, which the mouse can find once it is painted.
    fn text(&self, text: &Text, segment: Option<usize>, cx: &App) -> AnyElement {
        let theme = theme(cx);
        let selected = segment.and_then(|segment| self.selected_in(segment, text.text.len()));
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

    /// Code on a background of its own, with what is wrong with it if `error` tells; its text a
    /// segment if `segment` is its number, and then the next.
    fn code_block(&self, text: &Text, error: Option<&str>, segment: Option<&mut usize>, cx: &App) -> AnyElement {
        let theme = theme(cx);
        let code = self.text(text, segment.as_deref().copied(), cx);
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
            .child(code)
            .when_some(error, |code, error| {
                code.child(div().pt(px(8.)).text_color(rgb(theme.error_edge)).child(error.to_owned()))
            })
            .into_any_element()
    }

    /// An image from a file; one from anywhere else shows as its text.
    fn image(&self, url: &str, alt: &str, cx: &App) -> AnyElement {
        let theme = theme(cx);
        let alt = SharedString::from(if alt.is_empty() { url.to_owned() } else { alt.to_owned() });
        let missing = {
            let alt = alt.clone();
            move || div().text_color(rgb(theme.text_muted)).child(format!("[{alt}]")).into_any_element()
        };
        match markdown::image_file(url, self.folder.as_deref()) {
            Some(path) => img(path).max_w_full().with_fallback(missing).into_any_element(),
            None => missing(),
        }
    }

    /// A Mermaid diagram, drawn in the background the first time it shows in a theme; until then its
    /// source, and the source with what is wrong if it cannot be drawn.
    fn diagram(&mut self, source: &str, cx: &mut Context<Self>) -> AnyElement {
        let dark = theme(cx).appearance == Appearance::Dark;
        let key = (source.to_owned(), dark);
        match self.diagrams.get(&key) {
            Some(Diagram::Drawn(image)) => {
                return div()
                    .w_full()
                    .flex()
                    .justify_center()
                    .child(img(image.clone()).max_w_full())
                    .into_any_element();
            }
            Some(Diagram::Failed(error)) => {
                let error = format!("{} {error}", self.diagram_error);
                return self.code_block(&plain(source), Some(&error), None, cx);
            }
            Some(Diagram::Drawing) => {}
            None => {
                self.diagrams.insert(key.clone(), Diagram::Drawing);
                let source = source.to_owned();
                let draw = cx.background_spawn(async move { mermaid::render(&source, dark) });
                cx.spawn(async move |preview, cx| {
                    let diagram = match draw.await {
                        Ok(svg) => Diagram::Drawn(Arc::new(Image::from_bytes(ImageFormat::Svg, svg.into_bytes()))),
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
        // Painted before the blocks: the segments of the last frame go, those painted now come.
        let layouts = self.layouts.clone();
        let forget = canvas(|_, _, _| {}, move |_, _, _, _| layouts.borrow_mut().clear()).absolute().size_0();
        div()
            .id("markdown-preview")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::select_all))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(|preview, _, _, _| preview.selecting = false))
            .size_full()
            .bg(rgb(theme.editor.background))
            .text_color(rgb(theme.editor.text))
            .text_size(text_size(cx))
            .line_height(relative(1.5))
            .child(forget)
            .child(
                list(self.list.clone(), cx.processor(|preview, index, _, cx| preview.block_at(index, cx))).size_full(),
            )
    }
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
        Block::Code { code, .. } => text(code),
        Block::Html(html) => text(html),
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

/// The sources of the diagrams of `block`.
fn diagram_sources(block: &Block) -> Vec<&str> {
    match block {
        Block::Mermaid(source) => vec![source.as_str()],
        Block::Quote(blocks) => blocks.iter().flat_map(diagram_sources).collect(),
        Block::List { items, .. } => items.iter().flat_map(|item| &item.blocks).flat_map(diagram_sources).collect(),
        _ => Vec::new(),
    }
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
    let text = text.strip_suffix('\n').unwrap_or(text);
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
    }
}
