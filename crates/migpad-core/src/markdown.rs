//! Markdown for the preview: the blocks of a document and their text with its styles —
//! CommonMark with the extensions of GitHub: tables, strikethrough, task lists. A block of code in
//! the language `mermaid` is a diagram, which [`mermaid`] draws. HTML is not interpreted: a block
//! of it shows as it is written; inline tags go, the text between them stays.

pub mod mermaid;

use std::collections::HashMap;
use std::ops::Range;
use std::path::{Component, Path, PathBuf, Prefix};
use std::sync::LazyLock;

use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, LinkType, Options, Parser, Tag, TagEnd};
use regex::Regex;

/// A block of a document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    /// A heading of level 1 to 6.
    Heading {
        level: u8,
        content: Vec<Inline>,
    },
    Paragraph(Vec<Inline>),
    /// Code as it is written, and its language if the block tells it.
    Code {
        language: Option<String>,
        code: String,
    },
    /// The source of a Mermaid diagram.
    Mermaid(String),
    Quote(Vec<Block>),
    /// A list numbered from `start`, or with bullets.
    List {
        start: Option<u64>,
        items: Vec<Item>,
    },
    /// A table: how its columns align, its header and its rows, a cell for each column.
    Table {
        columns: Vec<Align>,
        header: Vec<Vec<Inline>>,
        rows: Vec<Vec<Vec<Inline>>>,
    },
    /// A line across.
    Rule,
    /// HTML as it is written.
    Html(String),
}

/// An item of a list: a task, done or not, if it has a box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Item {
    pub checked: Option<bool>,
    pub blocks: Vec<Block>,
}

/// How a column of a table aligns its cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Default,
    Left,
    Center,
    Right,
}

/// What a paragraph, a heading or a cell is made of: text, and images among it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Inline {
    Text(Text),
    /// An image: where it is, as written, the text for it, and where it leads if it is in a link.
    Image {
        url: String,
        alt: String,
        link: Option<String>,
    },
}

/// Text and the styles of its parts: the runs follow each other and cover it whole. Its links are
/// parts of it too, with where they lead.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Text {
    pub text: String,
    pub runs: Vec<Run>,
    pub links: Vec<Link>,
}

/// A link: its text, as a byte range, and where it leads, as it is written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub range: Range<usize>,
    pub url: String,
}

/// A part of a text with one style, as a byte range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    pub range: Range<usize>,
    pub style: Style,
}

/// The style of a part of a text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Style {
    pub strong: bool,
    pub emphasis: bool,
    pub strikethrough: bool,
    /// Inline code.
    pub code: bool,
    /// The text of a link.
    pub link: bool,
}

impl Text {
    fn push(&mut self, text: &str, style: Style) {
        if text.is_empty() {
            return;
        }
        let start = self.text.len();
        self.text.push_str(text);
        match self.runs.last_mut() {
            Some(run) if run.style == style => run.range.end = self.text.len(),
            _ => self.runs.push(Run { range: start..self.text.len(), style }),
        }
    }
}

/// How deep quotes and lists nest at most: deeper ones go on at that depth. Laying out nested
/// blocks costs more the deeper they are, and so does the stack of what walks them.
pub const MAX_NESTING: usize = 16;

/// The blocks of the Markdown document `markdown`.
pub fn parse(markdown: &str) -> Vec<Block> {
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut builder = Builder { stack: vec![Container::Blocks(Vec::new())], ..Builder::default() };
    for event in Parser::new_ext(markdown, options) {
        builder.event(event);
    }
    builder.finish()
}

/// What blocks are being read into.
enum Container {
    /// The document, or a quote.
    Blocks(Vec<Block>),
    List {
        start: Option<u64>,
        items: Vec<Item>,
    },
    Item(Item),
    Table {
        columns: Vec<Align>,
        header: Vec<Vec<Inline>>,
        rows: Vec<Vec<Vec<Inline>>>,
        row: Vec<Vec<Inline>>,
    },
}

/// Text of a paragraph, a heading or a cell being read.
#[derive(Default)]
struct Inlines {
    inlines: Vec<Inline>,
    text: Text,
    /// How many of each style are open: they nest.
    strong: usize,
    emphasis: usize,
    strikethrough: usize,
    /// The links open: where each starts in the text, and where it leads.
    links: Vec<(usize, String)>,
    /// An image being read: where it is, and its text so far.
    image: Option<(String, String)>,
}

impl Inlines {
    fn style(&self, code: bool) -> Style {
        Style {
            strong: self.strong > 0,
            emphasis: self.emphasis > 0,
            strikethrough: self.strikethrough > 0,
            code,
            link: !self.links.is_empty(),
        }
    }

    fn push(&mut self, text: &str, code: bool) {
        match &mut self.image {
            Some((_, alt)) => alt.push_str(text),
            None => {
                let style = self.style(code);
                self.text.push(text, style);
            }
        }
    }

    fn flush_text(&mut self) {
        if !self.text.text.is_empty() {
            // The links open go on in the text that follows.
            for (start, url) in &mut self.links {
                close_link(&mut self.text, *start, url.clone());
                *start = 0;
            }
            self.inlines.push(Inline::Text(std::mem::take(&mut self.text)));
        }
    }

    fn end_link(&mut self) {
        if let Some((start, url)) = self.links.pop() {
            close_link(&mut self.text, start, url);
        }
    }

    fn finish(mut self) -> Vec<Inline> {
        self.flush_text();
        self.inlines
    }
}

/// Where a run of inline content goes once it ends.
enum Target {
    Paragraph,
    /// The text of a tight list item, which has no paragraph of its own.
    Loose,
    Heading(u8),
    Cell,
}

#[derive(Default)]
struct Builder {
    stack: Vec<Container>,
    /// How many quotes, lists and items are open past [`MAX_NESTING`]: their blocks go into the
    /// deepest container there is.
    flat: usize,
    inlines: Option<(Target, Inlines)>,
    /// A block of code being read: its language and its text.
    code: Option<(Option<String>, String)>,
    /// A block of HTML being read.
    html: Option<String>,
}

impl Builder {
    fn event(&mut self, event: Event) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => self.text(&text, false),
            Event::Code(text) => self.text(&text, true),
            Event::InlineMath(text) | Event::DisplayMath(text) => self.text(&text, true),
            Event::Html(html) => match &mut self.html {
                Some(block) => block.push_str(&html),
                None => {
                    self.end_loose();
                    self.push_block(Block::Html(html.trim_end().to_owned()));
                }
            },
            Event::InlineHtml(html) => {
                let tag = html.trim().to_ascii_lowercase();
                if tag.starts_with("<br") {
                    self.text("\n", false);
                }
            }
            Event::FootnoteReference(name) => self.text(&format!("[{name}]"), false),
            Event::SoftBreak => self.text(" ", false),
            Event::HardBreak => self.text("\n", false),
            Event::Rule => {
                self.end_loose();
                self.push_block(Block::Rule);
            }
            Event::TaskListMarker(checked) => {
                if self.flat == 0
                    && let Some(Container::Item(item)) = self.stack.last_mut()
                {
                    item.checked = Some(checked);
                }
            }
        }
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph => {
                self.end_loose();
                self.inlines = Some((Target::Paragraph, Inlines::default()));
            }
            Tag::Heading { level, .. } => {
                self.end_loose();
                self.inlines = Some((Target::Heading(heading_level(level)), Inlines::default()));
            }
            Tag::BlockQuote(_) => {
                self.end_loose();
                if !self.flattens() {
                    self.stack.push(Container::Blocks(Vec::new()));
                }
            }
            Tag::CodeBlock(kind) => {
                self.end_loose();
                let language = match kind {
                    CodeBlockKind::Fenced(info) => info.split_whitespace().next().map(str::to_owned),
                    CodeBlockKind::Indented => None,
                };
                self.code = Some((language, String::new()));
            }
            Tag::HtmlBlock => {
                self.end_loose();
                self.html = Some(String::new());
            }
            Tag::List(start) => {
                self.end_loose();
                if !self.flattens() {
                    self.stack.push(Container::List { start, items: Vec::new() });
                }
            }
            Tag::Item => {
                self.end_loose();
                if !self.flattens() {
                    self.stack.push(Container::Item(Item::default()));
                }
            }
            Tag::Table(alignments) => {
                self.end_loose();
                let columns = alignments.into_iter().map(align).collect();
                self.stack.push(Container::Table { columns, header: Vec::new(), rows: Vec::new(), row: Vec::new() });
            }
            Tag::TableHead | Tag::TableRow => {}
            Tag::TableCell => self.inlines = Some((Target::Cell, Inlines::default())),
            Tag::Emphasis => self.with_inlines(|inlines| inlines.emphasis += 1),
            Tag::Strong => self.with_inlines(|inlines| inlines.strong += 1),
            Tag::Strikethrough => self.with_inlines(|inlines| inlines.strikethrough += 1),
            Tag::Link { link_type, dest_url, .. } => {
                // An address in angle brackets, `<a@b.c>`, comes without its scheme.
                let url = match link_type {
                    LinkType::Email => format!("mailto:{dest_url}"),
                    _ => dest_url.into_string(),
                };
                self.with_inlines(|inlines| inlines.links.push((inlines.text.text.len(), url)))
            }
            Tag::Image { dest_url, .. } => self.with_inlines(|inlines| {
                inlines.flush_text();
                inlines.image = Some((dest_url.into_string(), String::new()));
            }),
            Tag::Superscript | Tag::Subscript => {}
            Tag::FootnoteDefinition(_)
            | Tag::DefinitionList
            | Tag::DefinitionListTitle
            | Tag::DefinitionListDefinition
            | Tag::MetadataBlock(_) => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                if let Some((_, inlines)) = self.inlines.take() {
                    self.push_block(Block::Paragraph(inlines.finish()));
                }
            }
            TagEnd::Heading(_) => {
                if let Some((Target::Heading(level), inlines)) = self.inlines.take() {
                    self.push_block(Block::Heading { level, content: inlines.finish() });
                }
            }
            TagEnd::BlockQuote(_) => {
                self.end_loose();
                if self.unflattens() {
                    return;
                }
                if let Some(Container::Blocks(blocks)) = self.stack.pop() {
                    self.push_block(Block::Quote(blocks));
                }
            }
            TagEnd::CodeBlock => {
                if let Some((language, code)) = self.code.take() {
                    let block = match language {
                        Some(language) if language.eq_ignore_ascii_case("mermaid") => Block::Mermaid(code),
                        language => Block::Code { language, code: code.strip_suffix('\n').unwrap_or(&code).to_owned() },
                    };
                    self.push_block(block);
                }
            }
            TagEnd::HtmlBlock => {
                if let Some(html) = self.html.take() {
                    self.push_block(Block::Html(html.trim_end().to_owned()));
                }
            }
            TagEnd::List(_) => {
                if self.unflattens() {
                    return;
                }
                if let Some(Container::List { start, items }) = self.stack.pop() {
                    self.push_block(Block::List { start, items });
                }
            }
            TagEnd::Item => {
                self.end_loose();
                if self.unflattens() {
                    return;
                }
                if let Some(Container::Item(item)) = self.stack.pop()
                    && let Some(Container::List { items, .. }) = self.stack.last_mut()
                {
                    items.push(item);
                }
            }
            TagEnd::TableCell => {
                if let Some((_, inlines)) = self.inlines.take()
                    && let Some(Container::Table { row, .. }) = self.stack.last_mut()
                {
                    row.push(inlines.finish());
                }
            }
            TagEnd::TableHead => {
                if let Some(Container::Table { header, row, .. }) = self.stack.last_mut() {
                    *header = std::mem::take(row);
                }
            }
            TagEnd::TableRow => {
                if let Some(Container::Table { rows, row, .. }) = self.stack.last_mut() {
                    rows.push(std::mem::take(row));
                }
            }
            TagEnd::Table => {
                if let Some(Container::Table { columns, header, rows, .. }) = self.stack.pop() {
                    self.push_block(Block::Table { columns, header, rows });
                }
            }
            TagEnd::Emphasis => self.with_inlines(|inlines| inlines.emphasis = inlines.emphasis.saturating_sub(1)),
            TagEnd::Strong => self.with_inlines(|inlines| inlines.strong = inlines.strong.saturating_sub(1)),
            TagEnd::Strikethrough => {
                self.with_inlines(|inlines| inlines.strikethrough = inlines.strikethrough.saturating_sub(1))
            }
            TagEnd::Link => self.with_inlines(Inlines::end_link),
            TagEnd::Image => self.with_inlines(|inlines| {
                if let Some((url, alt)) = inlines.image.take() {
                    let link = inlines.links.last().map(|(_, url)| url.clone());
                    inlines.inlines.push(Inline::Image { url, alt, link });
                }
            }),
            TagEnd::Superscript | TagEnd::Subscript => {}
            TagEnd::FootnoteDefinition
            | TagEnd::DefinitionList
            | TagEnd::DefinitionListTitle
            | TagEnd::DefinitionListDefinition
            | TagEnd::MetadataBlock(_) => {}
        }
    }

    /// Text: of a block of code or of HTML, or inline — in a tight list item, with no
    /// paragraph of its own, it starts one.
    fn text(&mut self, text: &str, code: bool) {
        if let Some((_, block)) = &mut self.code {
            block.push_str(text);
        } else if let Some(html) = &mut self.html {
            html.push_str(text);
        } else {
            if self.inlines.is_none() && self.in_item() {
                self.inlines = Some((Target::Loose, Inlines::default()));
            }
            if let Some((_, inlines)) = &mut self.inlines {
                inlines.push(text, code);
            }
        }
    }

    fn with_inlines(&mut self, change: impl FnOnce(&mut Inlines)) {
        if self.inlines.is_none() && self.in_item() {
            self.inlines = Some((Target::Loose, Inlines::default()));
        }
        if let Some((_, inlines)) = &mut self.inlines {
            change(inlines);
        }
    }

    /// Whether text goes into a list item, which has no paragraph of its own when the list is
    /// tight: one too deep counts, whatever the container it goes on in.
    fn in_item(&self) -> bool {
        self.flat > 0 || matches!(self.stack.last(), Some(Container::Item(_)))
    }

    /// Whether a quote, a list or an item starting now is too deep: then it is not a container of
    /// its own, and it is counted.
    fn flattens(&mut self) -> bool {
        // The document is not a level.
        let deep = self.flat > 0 || self.stack.len() > MAX_NESTING;
        if deep {
            self.flat += 1;
        }
        deep
    }

    /// Whether the quote, list or item ending now was too deep: then it is uncounted.
    fn unflattens(&mut self) -> bool {
        let deep = self.flat > 0;
        if deep {
            self.flat -= 1;
        }
        deep
    }

    /// Ends the text of a tight list item as a paragraph, before a block that follows it.
    fn end_loose(&mut self) {
        if matches!(self.inlines, Some((Target::Loose, _)))
            && let Some((_, inlines)) = self.inlines.take()
        {
            self.push_block(Block::Paragraph(inlines.finish()));
        }
    }

    /// Adds a block to the document, the quote or the list item being read.
    fn push_block(&mut self, block: Block) {
        for container in self.stack.iter_mut().rev() {
            match container {
                Container::Blocks(blocks) => return blocks.push(block),
                Container::Item(item) => return item.blocks.push(block),
                Container::List { .. } | Container::Table { .. } => {}
            }
        }
    }

    fn finish(mut self) -> Vec<Block> {
        self.end_loose();
        match self.stack.into_iter().next() {
            Some(Container::Blocks(blocks)) => blocks,
            _ => Vec::new(),
        }
    }
}

/// The file of an image or of a link, by its URL as written in a document in `folder`: a path
/// relative to the folder or an absolute one, or a `file:` URL, percent escapes decoded, `.` and
/// `..` resolved as written. `None` for one anywhere else — on the network, in the URL itself, a
/// device of Windows — which is not read.
pub fn local_file(url: &str, folder: Option<&Path>) -> Option<PathBuf> {
    let url = url.trim();
    let path = match strip_prefix_ignoring_case(url, "file:") {
        Some(rest) => {
            // `file:///a`, `file://localhost/a` or `file:/a`; `file://host/a` is of another computer.
            let rest = match rest.strip_prefix("//") {
                Some(rest) => strip_prefix_ignoring_case(rest, "localhost").unwrap_or(rest),
                None => rest,
            };
            if !rest.starts_with('/') {
                return None;
            }
            // file:///C:/a.png on Windows.
            let rest = match rest.as_bytes() {
                [b'/', drive, b':', ..] if cfg!(windows) && drive.is_ascii_alphabetic() => &rest[1..],
                _ => rest,
            };
            decode_percent(rest)?
        }
        None if has_scheme(url) => return None,
        // What follows `?` or `#` is not of the file: `logo.png?raw=true`.
        None => decode_percent(url.split(['?', '#']).next()?)?,
    };
    // A path of the network, which Windows reads over it: two separators, as `\\host\share`,
    // `//host/share` or `\/host/share`, start one.
    let separator = |c: Option<char>| matches!(c, Some('/' | '\\'));
    let mut start = path.chars();
    if path.is_empty() || (separator(start.next()) && separator(start.next())) {
        return None;
    }
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        return Some(lexical(&folder?.join(path)));
    }
    // Of Windows, a drive: not a share nor a device, as `\\?\UNC\…` or `\\.\pipe\…`.
    if let Some(Component::Prefix(prefix)) = path.components().next()
        && !matches!(prefix.kind(), Prefix::Disk(_))
    {
        return None;
    }
    Some(lexical(&path))
}

/// `text` without `prefix` at its start, in whatever case it is written.
fn strip_prefix_ignoring_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let start = text.get(..prefix.len())?;
    start.eq_ignore_ascii_case(prefix).then(|| &text[prefix.len()..])
}

/// `path` with its `.` and `..` resolved as they are written, not through links: `a/b/../c` is
/// `a/c`, as Windows resolves them; at the root `..` is the root.
fn lexical(path: &Path) -> PathBuf {
    let mut resolved = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                resolved.pop();
            }
            other => resolved.push(other),
        }
    }
    resolved
}

/// Where a link of a document leads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkTarget {
    /// A page of the web or a letter, which programs of the system open.
    Web(String),
    /// A file or a folder of this computer, named by a `file:` URL: the file manager shows it.
    Reveal(PathBuf),
    /// A file or a folder by its path, relative to the document or not.
    File(PathBuf),
    /// A heading of the document, by its anchor.
    Heading(String),
}

/// Where the link `url` of a document in `folder` leads; `None` for a link of another kind — a
/// scheme other than `http:`, `https:`, `mailto:` and `file:` could start any program — or to a
/// file of another computer.
pub fn link_target(url: &str, folder: Option<&Path>) -> Option<LinkTarget> {
    let url = url.trim();
    if let Some(anchor) = url.strip_prefix('#') {
        return Some(LinkTarget::Heading(decode_percent(anchor)?));
    }
    if !has_scheme(url) {
        return local_file(url, folder).map(LinkTarget::File);
    }
    let scheme = url.split(':').next().unwrap_or_default().to_ascii_lowercase();
    match scheme.as_str() {
        "http" | "https" | "mailto" => Some(LinkTarget::Web(url.to_owned())),
        "file" => local_file(url, None).map(LinkTarget::Reveal),
        _ => None,
    }
}

/// The anchor of a heading of the text `text`, as GitHub makes it: in lower case, its spaces
/// hyphens, without punctuation and symbols but the hyphen and the underscore; letters, digits and
/// the marks over and under letters stay.
pub fn slug(text: &str) -> String {
    static MARK: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"^\p{M}$").ok());
    let mark = |c: char| MARK.as_ref().is_some_and(|mark| mark.is_match(c.encode_utf8(&mut [0; 4])));
    text.trim()
        .to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            '-' | '_' => Some(c),
            c if c.is_alphanumeric() || mark(c) => Some(c),
            _ => None,
        })
        .collect()
}

/// The anchors of headings of the texts `texts`, in order, as GitHub makes them: a name taken
/// before gets `-1`, `-2` after it, and so does the name of a heading that is already one of those.
pub fn slugs<'a>(texts: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut taken: HashMap<String, usize> = HashMap::new();
    texts
        .into_iter()
        .map(|text| {
            let base = slug(text);
            let mut name = base.clone();
            while taken.contains_key(&name) {
                let count = taken.entry(base.clone()).or_default();
                *count += 1;
                name = format!("{base}-{count}");
            }
            taken.insert(name.clone(), 0);
            name
        })
        .collect()
}

/// The link of `text` from `start` to its end, if it has any of it.
fn close_link(text: &mut Text, start: usize, url: String) {
    if start < text.text.len() {
        text.links.push(Link { range: start..text.text.len(), url });
    }
}

/// Whether `url` starts with a scheme, as `https:` or `data:` do; a drive letter, as in `C:\`, is
/// not one.
fn has_scheme(url: &str) -> bool {
    match url.find(':') {
        Some(at) if at > 1 => {
            let scheme = &url[..at];
            scheme.starts_with(|c: char| c.is_ascii_alphabetic())
                && scheme.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
        }
        _ => false,
    }
}

/// `text` with its percent escapes decoded; `None` if they make no UTF-8.
fn decode_percent(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |at: usize| bytes.get(at).and_then(|byte| (*byte as char).to_digit(16));
        match (bytes[i], hex(i + 1), hex(i + 2)) {
            (b'%', Some(high), Some(low)) => {
                decoded.push((high * 16 + low) as u8);
                i += 3;
            }
            (byte, ..) => {
                decoded.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8(decoded).ok()
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn align(alignment: Alignment) -> Align {
    match alignment {
        Alignment::None => Align::Default,
        Alignment::Left => Align::Left,
        Alignment::Center => Align::Center,
        Alignment::Right => Align::Right,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(text: &str) -> Inline {
        Inline::Text(Text {
            text: text.to_owned(),
            runs: vec![Run { range: 0..text.len(), style: Style::default() }],
            links: Vec::new(),
        })
    }

    #[test]
    fn headings_and_paragraphs() {
        let blocks = parse("# Заголовок\n\nПервый абзац,\nв две строки.\n\n## Второй");
        assert_eq!(
            blocks,
            [
                Block::Heading { level: 1, content: vec![plain("Заголовок")] },
                Block::Paragraph(vec![plain("Первый абзац, в две строки.")]),
                Block::Heading { level: 2, content: vec![plain("Второй")] },
            ]
        );
    }

    #[test]
    fn styles_cover_the_text_whole() {
        let blocks = parse("a **b *c*** ~~d~~ `e` [f](https://g)");
        let [Block::Paragraph(content)] = &blocks[..] else { panic!("one paragraph: {blocks:?}") };
        let [Inline::Text(text)] = &content[..] else { panic!("one text: {content:?}") };
        assert_eq!(text.text, "a b c d e f");
        let styles: Vec<(&str, Style)> =
            text.runs.iter().map(|run| (&text.text[run.range.clone()], run.style)).collect();
        let strong = Style { strong: true, ..Style::default() };
        assert_eq!(
            styles,
            [
                ("a ", Style::default()),
                ("b ", strong),
                ("c", Style { emphasis: true, ..strong }),
                (" ", Style::default()),
                ("d", Style { strikethrough: true, ..Style::default() }),
                (" ", Style::default()),
                ("e", Style { code: true, ..Style::default() }),
                (" ", Style::default()),
                ("f", Style { link: true, ..Style::default() }),
            ]
        );
    }

    #[test]
    fn links_lead_where_they_are_written() {
        let blocks = parse("see [the *doc*](a.md#x), <https://example.com> or [mail](mailto:a@b)");
        let [Block::Paragraph(content)] = &blocks[..] else { panic!("one paragraph: {blocks:?}") };
        let [Inline::Text(text)] = &content[..] else { panic!("one text: {content:?}") };
        let links: Vec<(&str, &str)> =
            text.links.iter().map(|link| (&text.text[link.range.clone()], link.url.as_str())).collect();
        assert_eq!(
            links,
            [("the doc", "a.md#x"), ("https://example.com", "https://example.com"), ("mail", "mailto:a@b")]
        );
        // An address in angle brackets is a letter.
        let blocks = parse("<a@b.c>");
        let [Block::Paragraph(content)] = &blocks[..] else { panic!("one paragraph: {blocks:?}") };
        let [Inline::Text(text)] = &content[..] else { panic!("one text: {content:?}") };
        assert_eq!(text.links, [Link { range: 0..5, url: "mailto:a@b.c".to_owned() }]);
        // An image in a link leads where it does.
        let blocks = parse("[![CI](badge.svg)](https://ci)");
        let [Block::Paragraph(content)] = &blocks[..] else { panic!("one paragraph: {blocks:?}") };
        assert_eq!(
            content[..],
            [Inline::Image { url: "badge.svg".to_owned(), alt: "CI".to_owned(), link: Some("https://ci".to_owned()) }]
        );
        // An image in a link parts its text: the link goes on after it.
        let blocks = parse("[before ![i](x.png) after](https://l) out");
        let [Block::Paragraph(content)] = &blocks[..] else { panic!("one paragraph: {blocks:?}") };
        let [Inline::Text(before), Inline::Image { .. }, Inline::Text(after)] = &content[..] else {
            panic!("text, image, text: {content:?}")
        };
        assert_eq!(before.links, [Link { range: 0..7, url: "https://l".to_owned() }]);
        assert_eq!(after.text, " after out");
        assert_eq!(after.links, [Link { range: 0..6, url: "https://l".to_owned() }]);
    }

    #[test]
    fn links_lead_to_the_web_files_and_headings_only() {
        let folder = Path::new(if cfg!(windows) { r"C:\docs" } else { "/docs" });
        let target = |url| link_target(url, Some(folder));
        assert_eq!(target("#Связь-флоу"), Some(LinkTarget::Heading("Связь-флоу".to_owned())));
        assert_eq!(target("#%D1%84"), Some(LinkTarget::Heading("ф".to_owned())));
        for url in ["https://a/b?c#d", "HTTP://A", "mailto:a@b"] {
            assert_eq!(target(url), Some(LinkTarget::Web(url.to_owned())), "{url}");
        }
        assert_eq!(target("01-deposit.md"), Some(LinkTarget::File(folder.join("01-deposit.md"))));
        let parent = folder.parent().unwrap();
        assert_eq!(target("../flows/a%20b.md#top"), Some(LinkTarget::File(parent.join("flows/a b.md"))));
        for url in [
            "javascript:alert(1)",
            "zoommtg://zoom.us/join",
            "vscode://file/a",
            "data:text/html,a",
            "file:relative.txt",
            "file://host/share/a",
            "//host/share/a",
            r"\\host\share\a",
            "",
        ] {
            assert_eq!(target(url), None, "{url}");
        }
        if cfg!(windows) {
            assert_eq!(target("file:///D:/a%20b.txt"), Some(LinkTarget::Reveal(PathBuf::from("D:/a b.txt"))));
        } else {
            assert_eq!(target("file:///tmp/a%20b.txt"), Some(LinkTarget::Reveal(PathBuf::from("/tmp/a b.txt"))));
        }
    }

    #[test]
    fn anchors_of_headings_as_github_makes_them() {
        assert_eq!(slug("Связь флоу"), "связь-флоу");
        assert_eq!(slug("F-ING.1 — Общая схема"), "f-ing1--общая-схема");
        assert_eq!(slug(" Hello, World! "), "hello-world");
        assert_eq!(slug("snake_case and `code`"), "snake_case-and-code");
        // The marks of Devanagari stay, as GitHub keeps them.
        assert_eq!(slug("स्थापना"), "स्थापना");
        // A name taken gets a number, as does one already of that form.
        assert_eq!(slugs(["a", "a", "a-1", "b"]), ["a", "a-1", "a-1-1", "b"]);
    }

    #[test]
    fn breaks_and_html() {
        let blocks = parse("one  \ntwo<br>three <kbd>Ctrl</kbd>\n\n<div>\nblock\n</div>");
        assert_eq!(
            blocks,
            [Block::Paragraph(vec![plain("one\ntwo\nthree Ctrl")]), Block::Html("<div>\nblock\n</div>".to_owned())]
        );
    }

    #[test]
    fn lists_tight_loose_nested_and_tasks() {
        let blocks = parse("- [x] done\n- [ ] to do\n  1. first\n  2. second\n\n3. three\n\n   more\n");
        let item = |checked, blocks| Item { checked, blocks };
        assert_eq!(
            blocks,
            [
                Block::List {
                    start: None,
                    items: vec![
                        item(Some(true), vec![Block::Paragraph(vec![plain("done")])]),
                        item(
                            Some(false),
                            vec![
                                Block::Paragraph(vec![plain("to do")]),
                                Block::List {
                                    start: Some(1),
                                    items: vec![
                                        item(None, vec![Block::Paragraph(vec![plain("first")])]),
                                        item(None, vec![Block::Paragraph(vec![plain("second")])]),
                                    ],
                                },
                            ],
                        ),
                    ],
                },
                Block::List {
                    start: Some(3),
                    items: vec![item(
                        None,
                        vec![Block::Paragraph(vec![plain("three")]), Block::Paragraph(vec![plain("more")])]
                    )],
                },
            ]
        );
    }

    #[test]
    fn code_mermaid_quotes_and_rules() {
        let blocks = parse(
            "> quote\n> > nested\n\n```rust title\nfn main() {}\n```\n\n---\n\n```Mermaid\nflowchart LR\n  A --> B\n```\n\n    indented\n",
        );
        assert_eq!(
            blocks,
            [
                Block::Quote(vec![
                    Block::Paragraph(vec![plain("quote")]),
                    Block::Quote(vec![Block::Paragraph(vec![plain("nested")])])
                ]),
                Block::Code { language: Some("rust".to_owned()), code: "fn main() {}".to_owned() },
                Block::Rule,
                Block::Mermaid("flowchart LR\n  A --> B\n".to_owned()),
                Block::Code { language: None, code: "indented".to_owned() },
            ]
        );
    }

    #[test]
    fn tables() {
        let blocks = parse("| a | b | c |\n|:--|:-:|--:|\n| 1 | **2** |\n");
        assert_eq!(
            blocks,
            [Block::Table {
                columns: vec![Align::Left, Align::Center, Align::Right],
                header: vec![vec![plain("a")], vec![plain("b")], vec![plain("c")]],
                rows: vec![vec![
                    vec![plain("1")],
                    vec![Inline::Text(Text {
                        text: "2".to_owned(),
                        runs: vec![Run { range: 0..1, style: Style { strong: true, ..Style::default() } }],
                        links: Vec::new(),
                    })],
                    vec![],
                ]],
            }]
        );
    }

    #[test]
    fn images_among_text() {
        let blocks = parse("before ![the *alt*](img/a.png \"title\") after\n\n![](/abs.svg)");
        assert_eq!(
            blocks,
            [
                Block::Paragraph(vec![
                    plain("before "),
                    Inline::Image { url: "img/a.png".to_owned(), alt: "the alt".to_owned(), link: None },
                    plain(" after"),
                ]),
                Block::Paragraph(vec![Inline::Image { url: "/abs.svg".to_owned(), alt: String::new(), link: None }]),
            ]
        );
    }

    #[test]
    fn images_come_from_files_only() {
        let folder = Path::new(if cfg!(windows) { r"C:\docs" } else { "/docs" });
        let file = |url| local_file(url, Some(folder));
        assert_eq!(file("img/a.png"), Some(folder.join("img/a.png")));
        assert_eq!(file("./my%20logo.png?raw=true#top"), Some(folder.join("./my logo.png")));
        assert_eq!(file("%D0%BB%D0%BE%D0%B3%D0%BE.png"), Some(folder.join("лого.png")));
        assert_eq!(local_file("a.png", None), None);
        for url in [
            "https://example.com/a.png",
            "http://a/b.png",
            "data:image/png;base64,AAAA",
            "mailto:a@b",
            "",
            "//host/share/a.png",
            r"\\host\share\a.png",
            "file://host/share/a.png",
            "file:////host/share/a.png",
            // Windows reads `\/host/share` over the network too.
            "%5C/attacker/s/a.png",
            r"\/attacker/s/a.png",
            "/%5Cattacker/s/a.png",
        ] {
            assert_eq!(file(url), None, "{url}");
        }
        // `.` and `..` are resolved as written.
        assert_eq!(file("img/../logo.png"), Some(folder.join("logo.png")));
        if cfg!(windows) {
            assert_eq!(file(r"D:\pics\a.png"), Some(PathBuf::from(r"D:\pics\a.png")));
            assert_eq!(file("file:///D:/pics/a%20b.png"), Some(PathBuf::from("D:/pics/a b.png")));
        } else {
            assert_eq!(file("/pics/a.png"), Some(PathBuf::from("/pics/a.png")));
            assert_eq!(file("file:///pics/a%20b.png"), Some(PathBuf::from("/pics/a b.png")));
            assert_eq!(file("file://localhost/pics/a.png"), Some(PathBuf::from("/pics/a.png")));
            assert_eq!(file("FILE:///pics/a.png"), Some(PathBuf::from("/pics/a.png")));
            assert_eq!(file("file:/pics/a.png"), Some(PathBuf::from("/pics/a.png")));
            assert_eq!(file("/pics/../a.png"), Some(PathBuf::from("/a.png")));
        }
    }

    #[test]
    fn nesting_stops_deep_but_keeps_the_text() {
        let depth = |blocks: &[Block]| {
            fn depth(block: &Block) -> usize {
                match block {
                    Block::Quote(blocks) => 1 + blocks.iter().map(depth).max().unwrap_or(0),
                    Block::List { items, .. } => {
                        1 + items.iter().flat_map(|item| &item.blocks).map(depth).max().unwrap_or(0)
                    }
                    _ => 0,
                }
            }
            blocks.iter().map(depth).max().unwrap_or(0)
        };
        let text = |blocks: &[Block]| {
            let mut all = String::new();
            fn walk(block: &Block, all: &mut String) {
                match block {
                    Block::Paragraph(content) => content.iter().for_each(|inline| {
                        if let Inline::Text(text) = inline {
                            all.push_str(&text.text);
                            all.push('|');
                        }
                    }),
                    Block::Quote(blocks) => blocks.iter().for_each(|block| walk(block, all)),
                    Block::List { items, .. } => {
                        items.iter().flat_map(|item| &item.blocks).for_each(|block| walk(block, all))
                    }
                    _ => {}
                }
            }
            blocks.iter().for_each(|block| walk(block, &mut all));
            all
        };
        let quotes = parse(&format!("{} deep\n\nafter", ">".repeat(40)));
        assert_eq!(depth(&quotes), MAX_NESTING);
        assert_eq!(text(&quotes), "deep|after|");
        let lists: String = (0..30).map(|level| format!("{}- {level}\n", "  ".repeat(level))).collect();
        let lists = parse(&lists);
        // A list and its item make two levels.
        assert!(depth(&lists) <= MAX_NESTING, "{}", depth(&lists));
        assert_eq!(text(&lists), (0..30).map(|level| format!("{level}|")).collect::<String>());
        // Thousands of levels do not overflow the stack of whatever walks them.
        let blocks = parse(&">".repeat(20_000));
        assert!(depth(&blocks) <= MAX_NESTING);
    }

    #[test]
    fn nothing_is_lost_of_odd_input() {
        assert_eq!(parse(""), []);
        assert_eq!(parse("   \n\n"), []);
        // An unclosed fence runs to the end; a table without rows has a header.
        assert_eq!(parse("```\nopen"), [Block::Code { language: None, code: "open".to_owned() }]);
        assert!(matches!(&parse("| a |\n|---|\n")[..], [Block::Table { rows, .. }] if rows.is_empty()));
    }
}
