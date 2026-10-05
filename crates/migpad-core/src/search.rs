//! Find and replace: a phrase or a regular expression, matching case and whole words if asked.
//!
//! The search runs over the bytes of the text, so invalid UTF-8 does not get in the way; the
//! caller gives a contiguous slice (see [`Document::contiguous_text`]), from the main thread or
//! a background one.

use std::fmt;
use std::ops::Range;
use std::time::Instant;

use regex::bytes::{Regex, RegexBuilder};

use crate::document::{Document, TooLong};
use crate::history::{EditKind, Selection};
use crate::text::TextStore;

/// What to look for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    pub text: String,
    pub match_case: bool,
    /// The match must not continue a word on either side.
    pub whole_word: bool,
    /// `text` is a regular expression rather than a phrase to find as it is.
    pub regex: bool,
}

/// Why a query cannot be searched for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueryError {
    Empty,
    /// The regular expression is not valid; the message says why.
    Invalid(String),
}

impl fmt::Display for QueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QueryError::Empty => f.write_str("nothing to find"),
            QueryError::Invalid(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for QueryError {}

/// A query ready to search with.
#[derive(Clone, Debug)]
pub struct Search {
    regex: Regex,
    /// Replacements expand groups and escapes.
    expand: bool,
}

impl Search {
    /// In regular expressions, `.` does not match line breaks, and `^` and `$` match at the
    /// start and end of lines; LF, CR and CRLF all break lines.
    pub fn new(query: &Query) -> Result<Search, QueryError> {
        if query.text.is_empty() {
            return Err(QueryError::Empty);
        }
        let pattern = if query.regex { query.text.clone() } else { regex::escape(&query.text) };
        // Half boundaries only look outside the match, so a phrase may start or end with any
        // character: `-x` as a whole word matches in "a -x b" but not in "a-xb".
        let pattern = if query.whole_word { format!(r"\b{{start-half}}(?:{pattern})\b{{end-half}}") } else { pattern };
        let regex = RegexBuilder::new(&pattern)
            .case_insensitive(!query.match_case)
            .multi_line(true)
            .crlf(true)
            .build()
            .map_err(|error| QueryError::Invalid(error.to_string()))?;
        Ok(Search { regex, expand: query.regex })
    }

    /// The first match after `selection`, wrapping around to the start. The match that is
    /// `selection` itself — the one found last — is passed over, even if it is empty.
    pub fn find_next(&self, text: &[u8], selection: Range<usize>) -> Option<Range<usize>> {
        let mut from = selection.end.min(text.len());
        loop {
            match self.regex.find_at(text, from).map(|m| m.range()) {
                Some(found) if found == selection => {
                    if from >= text.len() {
                        break;
                    }
                    from = next_char(text, from);
                }
                Some(found) => return Some(found),
                None => break,
            }
        }
        // Around the end: the first match, which may be the selection itself if it is the only one.
        self.regex.find(text).map(|m| m.range())
    }

    /// The last match before `selection`, wrapping around to the end.
    pub fn find_prev(&self, text: &[u8], selection: Range<usize>) -> Option<Range<usize>> {
        let mut last_before = None;
        let mut last = None;
        for found in self.regex.find_iter(text).map(|m| m.range()) {
            if found.start < selection.start && found != selection {
                last_before = Some(found.clone());
            }
            last = Some(found);
        }
        last_before.or(last)
    }

    /// The number of matches.
    pub fn count(&self, text: &[u8]) -> usize {
        self.regex.find_iter(text).count()
    }

    /// What `range` is replaced with, if it is exactly a match: `replacement` as it is for a
    /// phrase; for a regular expression with groups (`$1`, `${name}`, `$$` for `$`) and escapes
    /// (`\n` for `newline`, `\t`, `\\`) expanded.
    pub fn replacement(&self, text: &[u8], range: Range<usize>, replacement: &str, newline: &[u8]) -> Option<Vec<u8>> {
        let captures = self.regex.captures_at(text, range.start)?;
        if captures.get(0)?.range() != range {
            return None;
        }
        Some(self.expand(&captures, &self.template(replacement, newline)))
    }

    /// The replacements of all matches, last first, so that each range still holds when the ones
    /// before it in the list have been replaced.
    pub fn replace_all(&self, text: &[u8], replacement: &str, newline: &[u8]) -> Vec<(Range<usize>, Vec<u8>)> {
        let template = self.template(replacement, newline);
        let mut replacements: Vec<_> = self
            .regex
            .captures_iter(text)
            .map(|captures| (captures.get(0).expect("the whole match").range(), self.expand(&captures, &template)))
            .collect();
        replacements.reverse();
        replacements
    }

    fn template(&self, replacement: &str, newline: &[u8]) -> Vec<u8> {
        if self.expand { unescape(replacement, newline) } else { replacement.as_bytes().to_vec() }
    }

    fn expand(&self, captures: &regex::bytes::Captures, template: &[u8]) -> Vec<u8> {
        if !self.expand {
            return template.to_vec();
        }
        let mut out = Vec::with_capacity(template.len());
        captures.expand(template, &mut out);
        out
    }
}

impl Document {
    /// Replaces every match as one undo step; `\n` in the replacement of a regular expression
    /// inserts the document's line break. Returns the number of replacements.
    pub fn replace_all(
        &mut self,
        search: &Search,
        replacement: &str,
        selection: Selection,
        now: Instant,
    ) -> Result<usize, TooLong> {
        let newline = self.format.line_ending.as_bytes();
        let replacements = search.replace_all(self.contiguous_text(), replacement, newline);
        if replacements.is_empty() {
            return Ok(0);
        }
        let edits: Vec<(Range<usize>, &[u8])> =
            replacements.iter().map(|(range, text)| (range.clone(), text.as_slice())).collect();
        let len_after =
            replacements.iter().fold(self.text().len(), |len, (range, text)| len - range.len() + text.len());
        let clamp = |pos: usize| pos.min(len_after);
        let after = Selection { anchor: clamp(selection.anchor), head: clamp(selection.head) };
        self.edit(&edits, selection, after, EditKind::Other, now)?;
        Ok(replacements.len())
    }
}

/// The position after the character at `pos`.
fn next_char(text: &[u8], pos: usize) -> usize {
    let mut next = pos + 1;
    while next < text.len() && text[next] & 0xC0 == 0x80 {
        next += 1;
    }
    next
}

/// Turns `\n`, `\t` and `\\` of a replacement into `newline`, a tab and a backslash; any other
/// backslash stays.
fn unescape(replacement: &str, newline: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(replacement.len());
    let mut chars = replacement.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
            continue;
        }
        match chars.clone().next() {
            Some('n') => out.extend_from_slice(newline),
            Some('t') => out.push(b'\t'),
            Some('\\') => out.push(b'\\'),
            _ => {
                out.push(b'\\');
                continue;
            }
        }
        chars.next();
    }
    out
}

#[cfg(test)]
mod tests {
    // Expected matches are lists of ranges, even when there is one.
    #![allow(clippy::single_range_in_vec_init)]

    use super::*;
    use crate::document::Format;
    use crate::line_ending::LineEnding;

    fn phrase(text: &str) -> Query {
        Query { text: text.into(), ..Query::default() }
    }

    fn regex(text: &str) -> Query {
        Query { text: text.into(), regex: true, ..Query::default() }
    }

    fn matches(query: Query, text: &str) -> Vec<Range<usize>> {
        Search::new(&query).unwrap().regex.find_iter(text.as_bytes()).map(|m| m.range()).collect()
    }

    fn document(text: &str) -> Document {
        let mut doc = Document::new();
        doc.edit(
            &[(0..0, text.as_bytes())],
            Selection::default(),
            Selection::default(),
            EditKind::Other,
            Instant::now(),
        )
        .unwrap();
        doc
    }

    fn text(doc: &Document) -> String {
        String::from_utf8(doc.text().to_vec(0..doc.text().len())).unwrap()
    }

    #[test]
    fn phrases_are_found_as_they_are() {
        assert_eq!(matches(phrase("a.b*"), "a.b* axb"), [0..4]);
        assert_eq!(matches(phrase("("), "f(x)"), [1..2]);
    }

    #[test]
    fn case_is_ignored_unless_asked() {
        assert_eq!(matches(phrase("привет"), "ПРИВЕТ привет"), [0..12, 13..25]);
        assert_eq!(matches(Query { match_case: true, ..phrase("привет") }, "ПРИВЕТ привет"), [13..25]);
    }

    #[test]
    fn whole_words_do_not_continue_words() {
        let whole = |text: &str| Query { whole_word: true, ..phrase(text) };
        assert_eq!(matches(whole("cat"), "cat concat cats cat."), [0..3, 16..19]);
        assert_eq!(matches(whole("-x"), "a -x b a-xb"), [2..4]);
        assert_eq!(matches(whole("кот"), "кот котёнок кот."), [0..6, 22..28]);
    }

    #[test]
    fn line_breaks_end_lines_in_regular_expressions() {
        assert_eq!(matches(regex("a.b"), "a\nb a\rb a\r\nb axb"), [13..16]);
        assert_eq!(matches(regex("^x"), "x\nx\rx\r\nx"), [0..1, 2..3, 4..5, 7..8]);
        assert_eq!(matches(regex("x$"), "x\nx\rx\r\nx"), [0..1, 2..3, 4..5, 7..8]);
    }

    #[test]
    fn invalid_utf8_does_not_get_in_the_way() {
        let search = Search::new(&phrase("abc")).unwrap();
        assert_eq!(search.find_next(b"\xFFabc\xFE", 0..0), Some(1..4));
        assert_eq!(Search::new(&regex(".")).unwrap().count(b"\xFFabc\xFE"), 3);
    }

    #[test]
    fn finding_wraps_around_both_ends() {
        let search = Search::new(&phrase("ab")).unwrap();
        let text = b"ab ab ab";
        assert_eq!(search.find_next(text, 0..0), Some(0..2));
        assert_eq!(search.find_next(text, 0..2), Some(3..5));
        assert_eq!(search.find_next(text, 6..8), Some(0..2));
        assert_eq!(search.find_prev(text, 3..5), Some(0..2));
        assert_eq!(search.find_prev(text, 0..2), Some(6..8));
        assert_eq!(search.find_next(b"xyz", 0..0), None);
        assert_eq!(search.count(text), 3);
    }

    #[test]
    fn empty_matches_do_not_stall() {
        let search = Search::new(&regex("^")).unwrap();
        let text = b"a\nb";
        assert_eq!(search.find_next(text, 0..0), Some(2..2));
        assert_eq!(search.find_next(text, 2..2), Some(0..0));
        assert_eq!(search.find_prev(text, 2..2), Some(0..0));
    }

    #[test]
    fn replace_all_is_one_undo_step() {
        let mut doc = document("one two one");
        let search = Search::new(&phrase("one")).unwrap();
        assert_eq!(doc.replace_all(&search, "1", Selection::caret(11), Instant::now()), Ok(2));
        assert_eq!(text(&doc), "1 two 1");
        assert_eq!(doc.undo(), Some(Selection::caret(11)));
        assert_eq!(text(&doc), "one two one");
        assert_eq!(
            doc.replace_all(&Search::new(&phrase("none")).unwrap(), "x", Selection::default(), Instant::now()),
            Ok(0)
        );
    }

    #[test]
    fn regular_expressions_expand_groups_and_escapes() {
        let mut doc = document("a=1 b=2");
        doc.format = Format { line_ending: LineEnding::CrLf, ..doc.format };
        let search = Search::new(&regex(r"(\w+)=(?<value>\w+)")).unwrap();
        doc.replace_all(&search, r"${value}:$1\n\t\\$$", Selection::default(), Instant::now()).unwrap();
        assert_eq!(text(&doc), "1:a\r\n\t\\$ 2:b\r\n\t\\$");
        // A phrase replaces literally.
        let mut doc = document("x");
        let search = Search::new(&phrase("x")).unwrap();
        doc.replace_all(&search, r"$1\n", Selection::default(), Instant::now()).unwrap();
        assert_eq!(text(&doc), r"$1\n");
    }

    #[test]
    fn only_an_exact_match_is_replaced() {
        let search = Search::new(&regex(r"\d+")).unwrap();
        let text = b"a 12 b";
        assert_eq!(search.replacement(text, 2..4, "<$0>", b"\n"), Some(b"<12>".to_vec()));
        assert_eq!(search.replacement(text, 2..3, "<$0>", b"\n"), None);
        assert_eq!(search.replacement(text, 0..1, "<$0>", b"\n"), None);
    }

    #[test]
    fn bad_queries_explain_themselves() {
        assert_eq!(Search::new(&phrase("")).unwrap_err(), QueryError::Empty);
        let QueryError::Invalid(message) = Search::new(&regex("(")).unwrap_err() else { panic!() };
        assert!(message.contains("unclosed"), "{message}");
    }
}
