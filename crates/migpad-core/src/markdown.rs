//! Markdown for the preview: the blocks of a document and their text with its styles —
//! CommonMark with the extensions of GitHub: tables, strikethrough, task lists. A block of code in
//! the language `mermaid` is a diagram, which [`mermaid`] draws. HTML is not interpreted: a block
//! of it shows as it is written; inline tags go, the text between them stays.

pub mod mermaid;

use std::ops::Range;
use std::path::{Path, PathBuf};

use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

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
    /// An image: where it is, as written, and the text for it.
    Image {
        url: String,
        alt: String,
    },
}

/// Text and the styles of its parts: the runs follow each other and cover it whole.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Text {
    pub text: String,
    pub runs: Vec<Run>,
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
    /// The text of a link, which is not followed.
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
    link: usize,
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
            link: self.link > 0,
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
            self.inlines.push(Inline::Text(std::mem::take(&mut self.text)));
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
                if let Some(Container::Item(item)) = self.stack.last_mut() {
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
                self.stack.push(Container::Blocks(Vec::new()));
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
                self.stack.push(Container::List { start, items: Vec::new() });
            }
            Tag::Item => self.stack.push(Container::Item(Item::default())),
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
            Tag::Link { .. } => self.with_inlines(|inlines| inlines.link += 1),
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
                if let Some(Container::List { start, items }) = self.stack.pop() {
                    self.push_block(Block::List { start, items });
                }
            }
            TagEnd::Item => {
                self.end_loose();
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
            TagEnd::Link => self.with_inlines(|inlines| inlines.link = inlines.link.saturating_sub(1)),
            TagEnd::Image => self.with_inlines(|inlines| {
                if let Some((url, alt)) = inlines.image.take() {
                    inlines.inlines.push(Inline::Image { url, alt });
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
            if self.inlines.is_none() && matches!(self.stack.last(), Some(Container::Item(_))) {
                self.inlines = Some((Target::Loose, Inlines::default()));
            }
            if let Some((_, inlines)) = &mut self.inlines {
                inlines.push(text, code);
            }
        }
    }

    fn with_inlines(&mut self, change: impl FnOnce(&mut Inlines)) {
        if self.inlines.is_none() && matches!(self.stack.last(), Some(Container::Item(_))) {
            self.inlines = Some((Target::Loose, Inlines::default()));
        }
        if let Some((_, inlines)) = &mut self.inlines {
            change(inlines);
        }
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

/// The file of an image, by its URL as written in a document in `folder`: a path relative to the
/// folder or an absolute one, or a `file:` URL, percent escapes decoded. `None` for an image
/// anywhere else — on the network, in the URL itself — which is not loaded.
pub fn image_file(url: &str, folder: Option<&Path>) -> Option<PathBuf> {
    let url = url.trim();
    let path = match url.strip_prefix("file://") {
        Some(rest) => {
            let rest = rest.strip_prefix("localhost").unwrap_or(rest);
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
    if path.is_empty() {
        return None;
    }
    let path = PathBuf::from(path);
    if path.is_absolute() { Some(path) } else { Some(folder?.join(path)) }
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
        Inline::Text(Text { text: text.to_owned(), runs: vec![Run { range: 0..text.len(), style: Style::default() }] })
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
                        runs: vec![Run { range: 0..1, style: Style { strong: true, ..Style::default() } }]
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
                    Inline::Image { url: "img/a.png".to_owned(), alt: "the alt".to_owned() },
                    plain(" after"),
                ]),
                Block::Paragraph(vec![Inline::Image { url: "/abs.svg".to_owned(), alt: String::new() }]),
            ]
        );
    }

    #[test]
    fn images_come_from_files_only() {
        let folder = Path::new(if cfg!(windows) { r"C:\docs" } else { "/docs" });
        let file = |url| image_file(url, Some(folder));
        assert_eq!(file("img/a.png"), Some(folder.join("img/a.png")));
        assert_eq!(file("./my%20logo.png?raw=true#top"), Some(folder.join("./my logo.png")));
        assert_eq!(file("%D0%BB%D0%BE%D0%B3%D0%BE.png"), Some(folder.join("лого.png")));
        assert_eq!(image_file("a.png", None), None);
        for url in ["https://example.com/a.png", "http://a/b.png", "data:image/png;base64,AAAA", "mailto:a@b", ""] {
            assert_eq!(file(url), None, "{url}");
        }
        if cfg!(windows) {
            assert_eq!(file(r"D:\pics\a.png"), Some(PathBuf::from(r"D:\pics\a.png")));
            assert_eq!(file("file:///D:/pics/a%20b.png"), Some(PathBuf::from("D:/pics/a b.png")));
        } else {
            assert_eq!(file("/pics/a.png"), Some(PathBuf::from("/pics/a.png")));
            assert_eq!(file("file:///pics/a%20b.png"), Some(PathBuf::from("/pics/a b.png")));
            assert_eq!(file("file://localhost/pics/a.png"), Some(PathBuf::from("/pics/a.png")));
        }
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
