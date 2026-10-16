//! Addresses in text as links, as GitHub makes them: the autolinks extension of GitHub Flavored
//! Markdown. `https://…`, `http://…` and `www.…` lead to the web, an address of mail to `mailto:`.
//!
//! GitHub finds a web address as it reads the text, before emphasis and code: in
//! `https://a.b/__init__.py` the underscores are a part of the address. When the events of
//! pulldown-cmark come, `__init__` is bold already; so web addresses are looked for in the source,
//! and the events they cover give way to a link. Addresses of mail GitHub finds in the text once
//! it is read, and so does [`add_mail_links`].

use std::ops::Range;

use pulldown_cmark::{CowStr, Event, LinkType, Tag, TagEnd};

use super::{Link, Run, Style, Text};

/// The events of the document `source` with its web addresses as links. Not in code, links,
/// images and HTML, nor after a `[` that may yet start a link.
pub(super) fn add_web_links<'a>(source: &'a str, events: Vec<(Event<'a>, Range<usize>)>) -> Vec<Event<'a>> {
    let mut out = Vec::with_capacity(events.len());
    // Code blocks, HTML blocks, links and images open: their text has no addresses.
    let mut inside = 0usize;
    // The brackets open in the text of the paragraph, heading, cell or list item.
    let mut brackets = 0usize;
    // Whether the next text starts its block or a line: then nothing stands before it.
    let mut line_start = true;
    // Where the rest of the text at `i` starts, when an address has taken its beginning.
    let mut from = None;
    let mut i = 0;
    while i < events.len() {
        let (event, range) = &events[i];
        if let Event::Text(text) = event
            && inside == 0
            && is_raw(source, text, range)
        {
            let start = from.take().unwrap_or(range.start);
            let first = (start == range.start && line_start).then_some(start);
            let Some(link) = next_web_link(source, start..range.end, first, &events[i + 1..], &mut brackets) else {
                if start < range.end {
                    out.push(Event::Text(CowStr::Borrowed(&source[start..range.end])));
                }
                line_start = false;
                i += 1;
                continue;
            };
            if start < link.start {
                out.push(Event::Text(CowStr::Borrowed(&source[start..link.start])));
            }
            let (written, rest) = link.text.split_at(link.end);
            let url = if written.starts_with("www.") { format!("http://{written}") } else { written.to_owned() };
            let tag =
                Tag::Link { link_type: LinkType::Autolink, dest_url: url.into(), title: "".into(), id: "".into() };
            out.push(Event::Start(tag));
            out.push(Event::Text(written.to_owned().into()));
            out.push(Event::End(TagEnd::Link));
            line_start = false;
            from = match link.after {
                // The punctuation after it, if any, stays in the text, which goes on.
                After::Here => Some(link.start + link.end),
                after => {
                    if !rest.is_empty() {
                        out.push(Event::Text(rest.to_owned().into()));
                    }
                    i += 1 + link.passed;
                    match after {
                        After::Text(at) => Some(at),
                        _ => None,
                    }
                }
            };
            continue;
        }
        match event {
            Event::Start(Tag::CodeBlock(_) | Tag::HtmlBlock | Tag::Link { .. } | Tag::Image { .. }) => inside += 1,
            Event::End(TagEnd::CodeBlock | TagEnd::HtmlBlock | TagEnd::Link | TagEnd::Image) => {
                inside = inside.saturating_sub(1)
            }
            _ => {}
        }
        if let Event::Start(Tag::Paragraph | Tag::Heading { .. } | Tag::TableCell | Tag::Item) = event {
            brackets = 0;
        }
        line_start = matches!(
            event,
            Event::Start(Tag::Paragraph | Tag::Heading { .. } | Tag::TableCell | Tag::Item)
                | Event::TaskListMarker(_)
                | Event::SoftBreak
                | Event::HardBreak
        );
        out.push(event.clone());
        i += 1;
    }
    out
}

/// Whether `text` is as it is written at `range` of the source: not an entity, not an escape.
fn is_raw(source: &str, text: &str, range: &Range<usize>) -> bool {
    source.get(range.clone()) == Some(text)
}

/// A web address found in the source.
struct WebLink {
    /// Where it starts in the source.
    start: usize,
    /// The text from its start to where it may reach, as it shows: its source, the entities and
    /// escapes in it as they read.
    text: String,
    /// The length of the address in `text`; the rest of it is punctuation after the address.
    end: usize,
    /// How many of the events after its text it covers.
    passed: usize,
    /// What comes after what it covers.
    after: After,
}

/// What comes after the text a web address may reach.
enum After {
    /// The rest of the text it starts in.
    Here,
    /// The rest of the text after the events it covers, from this place of the source.
    Text(usize),
    /// The event after those it covers.
    Event,
}

/// The first web address that starts within the text at `text` of the source; `first` is the
/// start of the text if nothing stands before it. `brackets` counts the brackets open.
fn next_web_link(
    source: &str,
    text: Range<usize>,
    first: Option<usize>,
    events: &[(Event, Range<usize>)],
    brackets: &mut usize,
) -> Option<WebLink> {
    for (offset, c) in source[text.clone()].char_indices() {
        let at = text.start + offset;
        match c {
            // An escaped bracket comes as a text of its own, after the backslash.
            '[' if !(at == text.start && source[..at].ends_with('\\')) => *brackets += 1,
            ']' => *brackets = brackets.saturating_sub(1),
            'h' | 'H' | 'w' if *brackets == 0 => {
                let before = if Some(at) == first { None } else { source[..at].chars().next_back() };
                if let Some(domain) = web_domain(&source[at..], before) {
                    let (text, passed, after) = reach(source, at, text.end, events);
                    let end = trim_end(&text);
                    if end > domain {
                        return Some(WebLink { start: at, text, end, passed, after });
                    }
                }
            }
            _ => {}
        }
    }
    None
}

/// Where the domain of the web address at the start of `text` starts, after `before`, if it has
/// one: `http://` or `https://` after anything but a Latin letter, `www.` at the start or after a
/// space, `*`, `_`, `~` or `(`; then a domain.
fn web_domain(text: &str, before: Option<char>) -> Option<usize> {
    let scheme = ["https://", "http://"].into_iter().find(|scheme| starts_with_ignoring_case(text, scheme));
    let first = match scheme {
        Some(scheme) if !before.is_some_and(|c| c.is_ascii_alphabetic()) => scheme.len(),
        None if text.starts_with("www.")
            && !before.is_some_and(|c| !c.is_whitespace() && !matches!(c, '*' | '_' | '~' | '(')) =>
        {
            "www.".len()
        }
        _ => return None,
    };
    // After the scheme, a letter or a digit; after `www.`, any of the domain.
    let next = text[first..].chars().next()?;
    if !(next.is_alphanumeric() || (scheme.is_none() && matches!(next, '-' | '_'))) {
        return None;
    }
    let domain = if scheme.is_some() { first } else { 0 };
    domain_len(&text[domain..])?;
    Some(first)
}

/// The text from `at` of the source to where an address there may reach — a space or `<`, or an
/// event it cannot cover: the end of a paragraph or a cell, the end of emphasis that started
/// before it, an element with a space — with how many events after the text it covers, and what
/// comes after them. Emphasis, code and links within it are its text as they are written.
fn reach(source: &str, at: usize, text_end: usize, events: &[(Event, Range<usize>)]) -> (String, usize, After) {
    let stops = |c: char| c.is_whitespace() || c == '<';
    if let Some(stop) = source[at..text_end].find(stops) {
        return (source[at..at + stop].to_owned(), 0, After::Here);
    }
    let mut text = source[at..text_end].to_owned();
    let mut end = text_end;
    let mut j = 0;
    while let Some((event, range)) = events.get(j) {
        // Between them, a backslash that escapes what follows.
        if range.start < end || source[end..range.start].contains(stops) {
            break;
        }
        match event {
            Event::Text(written) => match written.find(stops) {
                Some(stop) if is_raw(source, written, range) => {
                    text.push_str(&written[..stop]);
                    return (text, j, After::Text(range.start + stop));
                }
                Some(_) => break,
                None => {
                    text.push_str(written);
                    end = range.end;
                    j += 1;
                }
            },
            Event::Code(_)
            | Event::Start(Tag::Emphasis | Tag::Strong | Tag::Strikethrough | Tag::Link { .. } | Tag::Image { .. }) => {
                if source[range.clone()].contains(stops) {
                    break;
                }
                text.push_str(&source[range.clone()]);
                end = range.end;
                j += if let Event::Code(_) = event { 1 } else { past_end(&events[j..]) };
            }
            _ => break,
        }
    }
    (text, j, After::Event)
}

/// How many events the element that starts the `events` takes, its end included.
fn past_end(events: &[(Event, Range<usize>)]) -> usize {
    let mut depth = 0usize;
    for (taken, (event, _)) in events.iter().enumerate() {
        match event {
            Event::Start(_) => depth += 1,
            Event::End(_) => {
                depth -= 1;
                if depth == 0 {
                    return taken + 1;
                }
            }
            _ => {}
        }
    }
    events.len()
}

/// The length of the domain at the start of `text`: letters, digits, `-` and `_` in parts that
/// `.` separates. `None` if its last two parts have `_`, as host names do not, or if it is longer
/// than a name can be: the text after each `_www.` of `_www.a_www.a_…` is not read to its end.
fn domain_len(text: &str) -> Option<usize> {
    const MAX: usize = 253;
    let (mut len, mut underscore, mut underscore_before) = (0, false, false);
    for (at, c) in text.char_indices() {
        if at >= MAX {
            return None;
        }
        match c {
            '.' => (underscore_before, underscore) = (underscore, false),
            '_' => underscore = true,
            '-' => {}
            c if c.is_alphanumeric() => {}
            _ => break,
        }
        len = at + c.len_utf8();
    }
    (len > 0 && !underscore && !underscore_before).then_some(len)
}

/// The length of `link` without the punctuation at its end that is not a part of it: `?`, `!`,
/// `.`, `,`, `:`, `*`, `_`, `~`, quotes, closing brackets without a pair, and what looks like an
/// entity, `&hl;`.
fn trim_end(link: &str) -> usize {
    let opening = link.matches('(').count();
    let mut closing = link.matches(')').count();
    let mut end = link.len();
    while let Some(c) = link[..end].chars().next_back() {
        match c {
            ')' if closing > opening => closing -= 1,
            '?' | '!' | '.' | ',' | ':' | '*' | '_' | '~' | '\'' | '"' => {}
            ';' => {
                let before = &link[..end - 1];
                let word = before.trim_end_matches(|c: char| c.is_ascii_alphanumeric());
                if word.len() < before.len() && word.ends_with('&') {
                    end = word.len() - 1;
                    continue;
                }
            }
            _ => break,
        }
        end -= 1;
    }
    end
}

/// Makes links of the addresses of mail in the parts of `text` that are neither code nor links.
pub(super) fn add_mail_links(text: &mut Text) {
    let mut runs = Vec::with_capacity(text.runs.len());
    let mut links = Vec::new();
    for run in std::mem::take(&mut text.runs) {
        if run.style.code || run.style.link {
            runs.push(run);
            continue;
        }
        let mut at = run.range.start;
        for found in find_mail(&text.text[run.range.clone()]) {
            let range = run.range.start + found.start..run.range.start + found.end;
            if at < range.start {
                runs.push(Run { range: at..range.start, style: run.style });
            }
            runs.push(Run { range: range.clone(), style: Style { link: true, ..run.style } });
            at = range.end;
            let written = &text.text[range.clone()];
            let url = if starts_with_ignoring_case(written, "mailto:") {
                written.to_owned()
            } else {
                format!("mailto:{written}")
            };
            links.push(Link { range, url });
        }
        if at < run.range.end {
            runs.push(Run { range: at..run.range.end, style: run.style });
        }
    }
    text.runs = runs;
    if !links.is_empty() {
        text.links.extend(links);
        text.links.sort_by_key(|link| link.range.start);
    }
}

/// The addresses of mail in `text`, as byte ranges.
fn find_mail(text: &str) -> Vec<Range<usize>> {
    let mut found = Vec::new();
    // Where the last address ends: the next one starts after it.
    let mut floor = 0;
    for (at, _) in text.match_indices('@') {
        if at < floor {
            continue;
        }
        if let Some(range) = mail(text, at, floor) {
            floor = range.end;
            found.push(range);
        }
    }
    found
}

/// The address of mail around the `@` at `at` in `text`, from no earlier than `floor`: Latin
/// letters, digits, `.`, `-`, `_` and `+` before it; letters, digits, `-` and `_` in parts that `.`
/// separates after it, at least two parts, the last not ending with `-` or `_`. `mailto:` before
/// it goes with it.
fn mail(text: &str, at: usize, floor: usize) -> Option<Range<usize>> {
    let before = text[floor..at].trim_end_matches(|c: char| c.is_ascii_alphanumeric() || ".-_+".contains(c));
    let start = floor + before.len();
    // In a path, as `a/b@c.d`, it is not an address.
    if start == at || before.ends_with('/') {
        return None;
    }
    let bytes = text.as_bytes();
    let (mut end, mut dots) = (at + 1, 0);
    while end < bytes.len() {
        match bytes[end] {
            b if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' => {}
            b'.' if bytes.get(end + 1).is_some_and(u8::is_ascii_alphanumeric) => dots += 1,
            _ => break,
        }
        end += 1;
    }
    if dots == 0 || matches!(bytes[end - 1], b'-' | b'_') || bytes.get(end) == Some(&b'@') {
        return None;
    }
    let mailto = before.len().checked_sub("mailto:".len()).filter(|&at| before.is_char_boundary(at));
    let start = match mailto {
        Some(mailto) if starts_with_ignoring_case(&before[mailto..], "mailto:") => floor + mailto,
        _ => start,
    };
    Some(start..end)
}

fn starts_with_ignoring_case(text: &str, prefix: &str) -> bool {
    text.get(..prefix.len()).is_some_and(|start| start.eq_ignore_ascii_case(prefix))
}

#[cfg(test)]
mod tests {
    use super::super::{Block, Inline, parse};

    /// The links of the Markdown document `markdown`: their text and where they lead.
    fn links(markdown: &str) -> Vec<(String, String)> {
        fn walk(blocks: &[Block], all: &mut Vec<(String, String)>) {
            for block in blocks {
                match block {
                    Block::Paragraph(content) | Block::Heading { content, .. } => inlines(content, all),
                    Block::Quote(blocks) => walk(blocks, all),
                    Block::List { items, .. } => items.iter().for_each(|item| walk(&item.blocks, all)),
                    Block::Table { header, rows, .. } => {
                        header.iter().chain(rows.iter().flatten()).for_each(|cell| inlines(cell, all))
                    }
                    _ => {}
                }
            }
        }
        fn inlines(content: &[Inline], all: &mut Vec<(String, String)>) {
            for inline in content {
                if let Inline::Text(text) = inline {
                    all.extend(
                        text.links.iter().map(|link| (text.text[link.range.clone()].to_owned(), link.url.clone())),
                    );
                }
            }
        }
        let mut all = Vec::new();
        walk(&parse(markdown), &mut all);
        all
    }

    fn web(written: &str) -> (String, String) {
        let url = if written.starts_with("www.") { format!("http://{written}") } else { written.to_owned() };
        (written.to_owned(), url)
    }

    fn mail(written: &str) -> (String, String) {
        (written.to_owned(), format!("mailto:{written}"))
    }

    fn the_text(markdown: &str) -> super::Text {
        let blocks = parse(markdown);
        let [Block::Paragraph(content)] = &blocks[..] else { panic!("one paragraph: {blocks:?}") };
        let [Inline::Text(text)] = &content[..] else { panic!("one text: {content:?}") };
        text.clone()
    }

    #[test]
    fn www_and_http_addresses_are_links() {
        assert_eq!(links("www.commonmark.org"), [web("www.commonmark.org")]);
        assert_eq!(links("Visit www.commonmark.org/help for more information."), [web("www.commonmark.org/help")]);
        assert_eq!(links("http://commonmark.org"), [web("http://commonmark.org")]);
        assert_eq!(links("HTTPS://Example.COM/A"), [web("HTTPS://Example.COM/A")]);
        assert_eq!(links("http://localhost:8080/x"), [web("http://localhost:8080/x")]);
        assert_eq!(links("Сайт: https://пример.рф/путь."), [web("https://пример.рф/путь")]);
        assert_eq!(links("two https://a.b and www.c.d"), [web("https://a.b"), web("www.c.d")]);
    }

    #[test]
    fn punctuation_at_the_end_is_not_a_part_of_an_address() {
        assert_eq!(links("Visit www.commonmark.org."), [web("www.commonmark.org")]);
        assert_eq!(links("Visit www.commonmark.org/a.b."), [web("www.commonmark.org/a.b")]);
        assert_eq!(links("see https://a.b/c?, or https://a.b/d!:"), [web("https://a.b/c"), web("https://a.b/d")]);
        assert_eq!(links("'https://a.b/c' \"www.d.e\""), [web("https://a.b/c")]);
    }

    #[test]
    fn closing_brackets_need_a_pair() {
        let url = "www.google.com/search?q=Markup+(business)";
        assert_eq!(links(url), [web(url)]);
        assert_eq!(links(&format!("{url}))")), [web(url)]);
        assert_eq!(links(&format!("({url})")), [web(url)]);
        assert_eq!(links(&format!("({url}")), [web(url)]);
        let within = "www.google.com/search?q=(business))+ok";
        assert_eq!(links(within), [web(within)]);
        let https = "https://encrypted.google.com/search?q=Markup+(business)";
        assert_eq!(links(&format!("(Visit {https})")), [web(https)]);
    }

    #[test]
    fn what_looks_like_an_entity_at_the_end_is_not_a_part_of_an_address() {
        let url = "www.google.com/search?q=commonmark&hl=en";
        assert_eq!(links(url), [web(url)]);
        assert_eq!(links("www.google.com/search?q=commonmark&hl;"), [web("www.google.com/search?q=commonmark")]);
        assert_eq!(links("www.a.b/c;"), [web("www.a.b/c")]);
        // An entity that is one reads as its character.
        assert_eq!(links("www.a.b/?q=1&amp;hl=en"), [web("www.a.b/?q=1&hl=en")]);
    }

    #[test]
    fn an_address_ends_before_a_space_or_an_angle_bracket() {
        assert_eq!(links("www.commonmark.org/he<lp"), [web("www.commonmark.org/he")]);
        assert_eq!(links("https://a.b/c\td"), [web("https://a.b/c")]);
    }

    #[test]
    fn where_an_address_starts() {
        assert_eq!(links("(www.a.b) *www.c.d* ~www.g.h~"), [web("www.a.b"), web("www.c.d"), web("www.g.h")]);
        // The closing underscore is in the last part of its domain, as GitHub reads it.
        assert_eq!(links("_www.e.f_"), []);
        assert_eq!(links("xwww.a.b 1www.a.b .www.a.b"), []);
        assert_eq!(links("xhttps://a.b"), []);
        assert_eq!(links("1https://a.b"), [web("https://a.b")]);
        assert_eq!(links("WWW.a.b"), [], "www is lower case");
        // What stands before an address is in the source: a code span, a link.
        assert_eq!(links("x`c`www.a.b [x](y)www.c.d"), [("x".to_owned(), "y".to_owned())]);
        // At the start of a line of a quote and of a cell nothing stands before it.
        assert_eq!(links(">www.a.b\n>www.c.d"), [web("www.a.b"), web("www.c.d")]);
        assert_eq!(links("| a | b |\n|---|---|\n|www.a.b|www.c.d|"), [web("www.a.b"), web("www.c.d")]);
    }

    #[test]
    fn an_address_goes_on_through_emphasis_and_code() {
        let url = "https://github.com/a/b/blob/main/__init__.py";
        assert_eq!(links(&format!("See {url} and more")), [web(url)]);
        assert_eq!(links("www.a.b/*x*/c and `www.c.d/`x`/e`"), [web("www.a.b/*x*/c")]);
        assert_eq!(links("www.a.b/`x`/c"), [web("www.a.b/`x`/c")]);
        // What is trimmed of it stays text, with no emphasis: its delimiter went with the address.
        let text = the_text("https://a.b/*x* and");
        assert_eq!(text.text, "https://a.b/*x* and");
        assert_eq!(text.links[0].range, 0.."https://a.b/*x".len());
        assert!(text.runs.iter().all(|run| !run.style.emphasis));
        // Emphasis that started before it ends after it.
        let text = the_text("**see www.a.b**, or *www.c.d/*");
        assert_eq!(text.text, "see www.a.b, or www.c.d/");
        let styles: Vec<(&str, bool, bool)> = text
            .runs
            .iter()
            .map(|run| (&text.text[run.range.clone()], run.style.link, run.style.strong || run.style.emphasis))
            .collect();
        assert_eq!(
            styles,
            [("see ", false, true), ("www.a.b", true, true), (", or ", false, false), ("www.c.d/", true, true)]
        );
        // An element with a space ends it.
        assert_eq!(links("https://a.b/__x y__"), [web("https://a.b/")]);
    }

    #[test]
    fn no_address_after_an_open_bracket() {
        assert_eq!(links("[https://a.b/c] [see www.c.d] [x] www.e.f"), [web("www.e.f")]);
        assert_eq!(links("[[www.a.b]] [unclosed www.c.d"), []);
        assert_eq!(
            links("[x](https://a.b) https://c.d"),
            [("x".to_owned(), "https://a.b".to_owned()), web("https://c.d")]
        );
        assert_eq!(links("\\[ www.a.b"), [web("www.a.b")], "an escaped bracket is text");
    }

    #[test]
    fn a_domain_is_needed() {
        assert_eq!(links("https:// www. www.. http://. https://-a www._a.b"), []);
        // Not a host name: an underscore in one of its last two parts.
        assert_eq!(links("www.xxx.yyy._zzz www.xxx._yyy.zzz"), []);
        assert_eq!(links("www._xxx.yyy.zzz"), [web("www._xxx.yyy.zzz")]);
    }

    #[test]
    fn addresses_of_mail_lead_to_mailto() {
        assert_eq!(links("foo@bar.baz"), [mail("foo@bar.baz")]);
        assert_eq!(
            links("hello@mail+xyz.example isn't valid, but hello+xyz@mail.example is."),
            [mail("hello+xyz@mail.example")]
        );
        assert_eq!(links("a.b-c_d@a.b"), [mail("a.b-c_d@a.b")]);
        assert_eq!(links("a.b-c_d@a.b."), [mail("a.b-c_d@a.b")]);
        assert_eq!(links("a.b-c_d@a.b- a.b-c_d@a.b_"), []);
        assert_eq!(links("(write to me@example.com)"), [mail("me@example.com")]);
        assert_eq!(links("Почта:me@example.com"), [mail("me@example.com")]);
        assert_eq!(links("@a.b a@b a/b@c.d"), []);
        // Two `@`: not the first address, as GitHub has it.
        assert_eq!(links("a@b.c@d.e"), [mail("b.c@d.e")]);
        assert_eq!(links("MAILTO:me@a.b"), [("MAILTO:me@a.b".to_owned(), "MAILTO:me@a.b".to_owned())]);
        assert_eq!(links("`me@a.b` [me@c.d](https://e.f)"), [("me@c.d".to_owned(), "https://e.f".to_owned())]);
    }

    #[test]
    fn an_address_of_mail_in_a_web_one_is_a_part_of_it() {
        assert_eq!(links("https://user@a.b/c and x@y.z"), [web("https://user@a.b/c"), mail("x@y.z")]);
    }

    #[test]
    fn long_text_is_read_once() {
        let text = "_www.a".repeat(50_000);
        assert_eq!(links(&text), [web("www.a")], "the last one, with a short domain");
        let text = "w".repeat(100_000) + &"a@".repeat(50_000) + &" http://a.b".repeat(20_000);
        assert_eq!(links(&text).len(), 20_000);
        // One address through all the emphasis.
        let text = "https://a.b/".to_owned() + &"*x*".repeat(20_000);
        assert_eq!(links(&text).len(), 1);
    }
}
