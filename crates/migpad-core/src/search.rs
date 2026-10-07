//! Find and replace: a phrase or a regular expression, matching case and whole words if asked.
//!
//! The search runs over the bytes of the text, so invalid UTF-8 does not get in the way; the
//! caller gives a contiguous slice (see [`Document::contiguous_text`]), from the main thread or
//! a background one. Counting the matches of a large text or collecting their replacements can
//! go a part at a time, see [`MatchWalk`].

use std::fmt;
use std::ops::Range;
use std::time::Instant;

use regex::bytes::{Regex, RegexBuilder};

use crate::document::{Document, EditError};
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
        let mut walk = MatchWalk::new(0);
        std::iter::from_fn(|| walk.next(self, text)).count()
    }

    /// What matches are replaced with: `replacement` as it is for a phrase; for a regular
    /// expression its escapes (`\n` for `newline`, `\t`, `\\`) are turned into bytes once here,
    /// and its groups (`$1`, `${name}`, `$$` for `$`) are expanded for each match.
    pub fn template(&self, replacement: &str, newline: &[u8]) -> Template {
        Template(if self.expand { unescape(replacement, newline) } else { replacement.as_bytes().to_vec() })
    }

    /// What `range` is replaced with by `template`, if it is exactly a match.
    pub fn replacement(&self, text: &[u8], range: Range<usize>, template: &Template) -> Option<Vec<u8>> {
        if !self.expand {
            let found = self.regex.find_at(text, range.start)?;
            return (found.range() == range).then(|| template.0.clone());
        }
        let captures = self.regex.captures_at(text, range.start)?;
        if captures.get(0)?.range() != range {
            return None;
        }
        let mut out = Vec::with_capacity(template.0.len());
        captures.expand(&template.0, &mut out);
        Some(out)
    }

    /// The replacements of all matches, in the order of the text.
    pub fn replace_all(&self, text: &[u8], template: &Template) -> Vec<(Range<usize>, Vec<u8>)> {
        let mut walk = MatchWalk::new(0);
        std::iter::from_fn(|| walk.next(self, text))
            .map(|found| {
                let replacement = self.replacement(text, found.clone(), template).expect("a match just found");
                (found, replacement)
            })
            .collect()
    }
}

/// What the matches of a search are replaced with, see [`Search::template`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Template(Vec<u8>);

/// A walk over the matches of a search, which can stop and go on later over the same text: a
/// large text is counted, or its replacements collected, a part a frame. Matches do not overlap;
/// an empty match right where the previous one ended is passed over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MatchWalk {
    /// Where the next search starts.
    pos: usize,
    /// Where the last match ended.
    last_end: Option<usize>,
}

impl MatchWalk {
    /// A walk from `from` on.
    pub fn new(from: usize) -> Self {
        MatchWalk { pos: from, last_end: None }
    }

    /// Where the next search starts: how far the walk has come.
    pub fn position(&self) -> usize {
        self.pos
    }

    /// The next match of `search` in `text`, or `None` past the last one.
    pub fn next(&mut self, search: &Search, text: &[u8]) -> Option<Range<usize>> {
        loop {
            if self.pos > text.len() {
                return None;
            }
            let Some(found) = search.regex.find_at(text, self.pos).map(|m| m.range()) else {
                self.pos = text.len() + 1;
                return None;
            };
            if found.is_empty() && Some(found.end) == self.last_end {
                self.pos = next_char(text, found.end);
                continue;
            }
            self.pos = if found.is_empty() { next_char(text, found.end) } else { found.end };
            self.last_end = Some(found.end);
            return Some(found);
        }
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
    ) -> Result<usize, EditError> {
        let template = search.template(replacement, self.format.line_ending.as_bytes());
        let replacements = search.replace_all(self.contiguous_text(), &template);
        self.apply_replacements(&replacements, selection, now)
    }

    /// Applies `replacements` — ranges of the text as it is, in its order, apart from each other —
    /// as one undo step that starts from `selection`, which stays where it is as far as the text
    /// allows. Returns the number of replacements.
    pub fn apply_replacements(
        &mut self,
        replacements: &[(Range<usize>, Vec<u8>)],
        selection: Selection,
        now: Instant,
    ) -> Result<usize, EditError> {
        if replacements.is_empty() {
            return Ok(0);
        }
        // The last first: each range still holds when the ones after it are replaced.
        let edits: Vec<(Range<usize>, &[u8])> =
            replacements.iter().rev().map(|(range, text)| (range.clone(), text.as_slice())).collect();
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
        let template = search.template("<$0>", b"\n");
        assert_eq!(search.replacement(text, 2..4, &template), Some(b"<12>".to_vec()));
        assert_eq!(search.replacement(text, 2..3, &template), None);
        assert_eq!(search.replacement(text, 0..1, &template), None);
        let search = Search::new(&phrase("12")).unwrap();
        let template = search.template("$1", b"\n");
        assert_eq!(search.replacement(text, 2..4, &template), Some(b"$1".to_vec()));
        assert_eq!(search.replacement(text, 1..3, &template), None);
    }

    #[test]
    fn a_walk_goes_on_where_it_stopped() {
        let search = Search::new(&phrase("ab")).unwrap();
        let text = b"ab xab abab";
        let mut walk = MatchWalk::new(0);
        assert_eq!(walk.next(&search, text), Some(0..2));
        assert_eq!(walk.position(), 2);
        // The text may be borrowed anew for each part.
        let again = text.to_vec();
        assert_eq!(walk.next(&search, &again), Some(4..6));
        assert_eq!(walk.next(&search, text), Some(7..9));
        assert_eq!(walk.next(&search, text), Some(9..11));
        assert_eq!(walk.next(&search, text), None);
        assert_eq!(walk.next(&search, text), None);
        // From the middle.
        assert_eq!(MatchWalk::new(5).next(&search, text), Some(7..9));
    }

    #[test]
    fn a_walk_passes_empty_matches_as_find_iter_does() {
        let walk = |pattern: &str, text: &str| {
            let search = Search::new(&regex(pattern)).unwrap();
            let mut walk = MatchWalk::new(0);
            std::iter::from_fn(|| walk.next(&search, text.as_bytes())).collect::<Vec<_>>()
        };
        assert_eq!(walk("^", "a\nb\n"), [0..0, 2..2, 4..4]);
        assert_eq!(walk("a*", "baab"), [0..0, 1..3, 4..4]);
        // Empty matches stay on character boundaries.
        assert_eq!(walk("x*", "жx"), [0..0, 2..3]);
        assert_eq!(Search::new(&regex("a*")).unwrap().count(b"baab"), 3);
    }

    #[test]
    fn collected_replacements_apply_as_one_step() {
        let mut doc = document("one two one");
        let search = Search::new(&phrase("one")).unwrap();
        let template = search.template("1", b"\n");
        let replacements = search.replace_all(doc.contiguous_text(), &template);
        assert_eq!(replacements, [(0..3, b"1".to_vec()), (8..11, b"1".to_vec())]);
        assert_eq!(doc.apply_replacements(&replacements, Selection::caret(11), Instant::now()), Ok(2));
        assert_eq!(text(&doc), "1 two 1");
        assert_eq!(doc.apply_replacements(&[], Selection::caret(0), Instant::now()), Ok(0));
        assert_eq!(doc.undo(), Some(Selection::caret(11)));
        assert_eq!(text(&doc), "one two one");
    }

    #[test]
    fn bad_queries_explain_themselves() {
        assert_eq!(Search::new(&phrase("")).unwrap_err(), QueryError::Empty);
        let QueryError::Invalid(message) = Search::new(&regex("(")).unwrap_err() else { panic!() };
        assert!(message.contains("unclosed"), "{message}");
    }
}
