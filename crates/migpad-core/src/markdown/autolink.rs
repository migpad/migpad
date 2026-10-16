//! Addresses in text as links, as GitHub makes them: the autolinks extension of GitHub Flavored
//! Markdown. `https://…`, `http://…` and `www.…` lead to the web, an address of mail to `mailto:`.

use std::ops::Range;

use super::{Link, Run, Style, Text};

/// Makes links of the addresses in the parts of `text` that are neither code nor links already.
pub(super) fn add_autolinks(text: &mut Text) {
    let mut runs = Vec::with_capacity(text.runs.len());
    let mut links = Vec::new();
    for run in std::mem::take(&mut text.runs) {
        if run.style.code || run.style.link {
            runs.push(run);
            continue;
        }
        let mut at = run.range.start;
        for (found, url) in find(&text.text[run.range.clone()]) {
            let range = run.range.start + found.start..run.range.start + found.end;
            if at < range.start {
                runs.push(Run { range: at..range.start, style: run.style });
            }
            runs.push(Run { range: range.clone(), style: Style { link: true, ..run.style } });
            at = range.end;
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

/// The addresses in `text`, as byte ranges, and where they lead.
fn find(text: &str) -> Vec<(Range<usize>, String)> {
    let mut found = Vec::new();
    // Where the last address ends: an address of mail starts after it.
    let mut floor = 0;
    let mut at = 0;
    while let Some(c) = text[at..].chars().next() {
        let link = match c {
            'h' | 'H' | 'w' => web(text, at).map(|end| (at, end)),
            '@' => mail(text, at, floor),
            _ => None,
        };
        match link {
            Some((start, end)) => {
                let written = &text[start..end];
                let url = match c {
                    'w' => format!("http://{written}"),
                    '@' if !starts_with_ignoring_case(written, "mailto:") => format!("mailto:{written}"),
                    _ => written.to_owned(),
                };
                found.push((start..end, url));
                floor = end;
                at = end;
            }
            None => at += c.len_utf8(),
        }
    }
    found
}

/// The end of the web address at `at` in `text`: `http://` or `https://` after anything but a
/// Latin letter, `www.` at the start or after a space, `*`, `_`, `~` or `(`; then a domain and
/// anything up to a space or `<`, without the punctuation at its end.
fn web(text: &str, at: usize) -> Option<usize> {
    let rest = &text[at..];
    let before = text[..at].chars().next_back();
    let domain = if let Some(scheme) = ["https://", "http://"].into_iter().find(|s| starts_with_ignoring_case(rest, s))
    {
        if before.is_some_and(|c| c.is_ascii_alphabetic()) {
            return None;
        }
        scheme.len()
    } else if rest.starts_with("www.") {
        if before.is_some_and(|c| !c.is_whitespace() && !matches!(c, '*' | '_' | '~' | '(')) {
            return None;
        }
        0
    } else {
        return None;
    };
    // After the scheme, a letter or a digit; after `www.`, any of the domain.
    let first = if domain == 0 { "www.".len() } else { domain };
    let next = rest[first..].chars().next();
    if !next.is_some_and(|c| c.is_alphanumeric() || (domain == 0 && matches!(c, '-' | '_'))) {
        return None;
    }
    let end = domain + domain_len(&rest[domain..])?;
    let end = end + rest[end..].find(|c: char| c.is_whitespace() || c == '<').unwrap_or(rest.len() - end);
    let end = trim_end(&rest[..end]);
    (end > first).then_some(at + end)
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

/// The address of mail around the `@` at `at` in `text`, from no earlier than `floor`: Latin
/// letters, digits, `.`, `-`, `_` and `+` before it; letters, digits, `-` and `_` in parts that `.`
/// separates after it, at least two parts, the last not ending with `-` or `_`. `mailto:` before
/// it goes with it.
fn mail(text: &str, at: usize, floor: usize) -> Option<(usize, usize)> {
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
    Some((start, end))
}

fn starts_with_ignoring_case(text: &str, prefix: &str) -> bool {
    text.get(..prefix.len()).is_some_and(|start| start.eq_ignore_ascii_case(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The addresses in `text` as written, and where they lead.
    fn links(text: &str) -> Vec<(&str, String)> {
        find(text).into_iter().map(|(range, url)| (&text[range], url)).collect()
    }

    fn web(written: &str) -> (&str, String) {
        let url = if written.starts_with("www.") { format!("http://{written}") } else { written.to_owned() };
        (written, url)
    }

    fn mail(written: &str) -> (&str, String) {
        (written, format!("mailto:{written}"))
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
        assert_eq!(links("see https://a.b/c?, or https://a.b/d!:*_~"), [web("https://a.b/c"), web("https://a.b/d")]);
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
    }

    #[test]
    fn an_address_ends_before_a_space_or_an_angle_bracket() {
        assert_eq!(links("www.commonmark.org/he<lp"), [web("www.commonmark.org/he")]);
        assert_eq!(links("https://a.b/c\td"), [web("https://a.b/c")]);
    }

    #[test]
    fn where_an_address_starts() {
        // `_www.e.f_` would be emphasis, its text without the underscores.
        assert_eq!(
            links("(www.a.b) *www.c.d* _www.e.f ~www.g.h~"),
            [web("www.a.b"), web("www.c.d"), web("www.e.f"), web("www.g.h")]
        );
        assert_eq!(links("xwww.a.b 1www.a.b .www.a.b"), []);
        assert_eq!(links("xhttps://a.b"), []);
        assert_eq!(links("1https://a.b"), [web("https://a.b")]);
        assert_eq!(links("WWW.a.b"), [], "www is lower case");
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
        assert_eq!(links("MAILTO:me@a.b"), [("MAILTO:me@a.b", "MAILTO:me@a.b".to_owned())]);
    }

    #[test]
    fn an_address_of_mail_in_a_web_one_is_a_part_of_it() {
        assert_eq!(
            links("https://user@a.b/c and x@y.z"),
            [web("https://user@a.b/c"), ("x@y.z", "mailto:x@y.z".to_owned())]
        );
    }

    #[test]
    fn long_text_is_read_once() {
        let text = "_www.a".repeat(50_000);
        assert_eq!(links(&text), [web("www.a")], "the last one, with a short domain");
        let text = "w".repeat(100_000) + &"a@".repeat(50_000) + &" http://a.b".repeat(20_000);
        assert_eq!(links(&text).len(), 20_000);
    }
}
