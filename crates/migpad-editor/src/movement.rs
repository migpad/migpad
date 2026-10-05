//! Where the caret goes in the text: by characters, words and lines. Positions are byte offsets
//! at character boundaries, never inside a line break; the screen is not involved.

use std::ops::Range;

use migpad_core::text::{LineIndex, TextStore};

/// Where moving right by a word stops, as the text fields of each system do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WordStop {
    /// At the end of the word: macOS and Linux.
    End,
    /// At the start of the next word: Windows.
    NextStart,
}

impl WordStop {
    pub const PLATFORM: WordStop = if cfg!(target_os = "windows") { WordStop::NextStart } else { WordStop::End };
}

/// Kinds of characters; a word is a run of characters of one kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// Letters, digits and `_`.
    Word,
    Space,
    /// Punctuation and everything else, invalid UTF-8 included.
    Other,
}

fn kind(c: char) -> Kind {
    if c.is_alphanumeric() || c == '_' {
        Kind::Word
    } else if c.is_whitespace() {
        Kind::Space
    } else {
        Kind::Other
    }
}

/// The character from `pos` to the next boundary, not past `end`; an invalid part of UTF-8 is
/// U+FFFD.
fn char_from<S: TextStore>(text: &S, pos: usize, end: usize) -> (char, usize) {
    let next = text.next_char_boundary(pos, end);
    (decode(text, pos..next), next)
}

/// The character before `pos`, not before `start`, and where it starts.
fn char_before<S: TextStore>(text: &S, pos: usize, start: usize) -> (char, usize) {
    let prev = text.prev_char_boundary(pos, start);
    (decode(text, prev..pos), prev)
}

fn decode<S: TextStore>(text: &S, range: Range<usize>) -> char {
    let mut bytes = [0u8; 4];
    let len = range.len().min(4);
    for (i, byte) in bytes[..len].iter_mut().enumerate() {
        *byte = text.byte(range.start + i);
    }
    std::str::from_utf8(&bytes[..len]).ok().and_then(|s| s.chars().next()).unwrap_or(char::REPLACEMENT_CHARACTER)
}

/// Moves right from `pos` over characters of kind `kind`, up to `end`.
fn skip_right<S: TextStore>(text: &S, mut pos: usize, end: usize, kind_of: Kind) -> usize {
    while pos < end {
        let (c, next) = char_from(text, pos, end);
        if kind(c) != kind_of {
            break;
        }
        pos = next;
    }
    pos
}

/// Moves left from `pos` over characters of kind `kind`, down to `start`.
fn skip_left<S: TextStore>(text: &S, mut pos: usize, start: usize, kind_of: Kind) -> usize {
    while pos > start {
        let (c, prev) = char_before(text, pos, start);
        if kind(c) != kind_of {
            break;
        }
        pos = prev;
    }
    pos
}

/// The line of `pos`: its bytes without the line break, and the length of the break.
fn line_at<S: TextStore>(text: &S, lines: &LineIndex, pos: usize) -> (usize, Range<usize>, usize) {
    let line = lines.line_of(pos);
    let (range, eol) = lines.line_range(text, line);
    (line, range, eol)
}

/// The nearest place at or before `pos` where the caret can be: within the text, at the start of
/// a character, not inside a line break.
pub fn snap<S: TextStore>(text: &S, lines: &LineIndex, pos: usize) -> usize {
    let pos = pos.min(text.len());
    let (_, range, _) = line_at(text, lines, pos);
    if pos >= range.end { range.end } else { text.floor_char_boundary(pos, range.start) }
}

/// One character to the right; a line break, CRLF included, is one step.
pub fn next_char<S: TextStore>(text: &S, lines: &LineIndex, pos: usize) -> usize {
    let (_, range, eol) = line_at(text, lines, pos);
    if pos < range.end { text.next_char_boundary(pos, range.end) } else { range.end + eol }
}

/// One character to the left; a line break, CRLF included, is one step.
pub fn prev_char<S: TextStore>(text: &S, lines: &LineIndex, pos: usize) -> usize {
    let (line, range, _) = line_at(text, lines, pos);
    if pos > range.start {
        text.prev_char_boundary(pos.min(range.end), range.start)
    } else if line > 0 {
        lines.line_range(text, line - 1).0.end
    } else {
        0
    }
}

/// The end of the line of `pos`, before its line break.
pub fn line_end<S: TextStore>(text: &S, lines: &LineIndex, pos: usize) -> usize {
    line_at(text, lines, pos).1.end
}

/// `line` with its line break, for selecting whole lines.
pub fn line_with_break<S: TextStore>(text: &S, lines: &LineIndex, line: usize) -> Range<usize> {
    let (range, eol) = lines.line_range(text, line);
    range.start..range.end + eol
}

/// One word to the right: over spaces, then over a word; at the end of a line, over its break.
/// [`WordStop::NextStart`] goes over the word first and stops after the spaces that follow it.
pub fn word_right<S: TextStore>(text: &S, lines: &LineIndex, pos: usize, stop: WordStop) -> usize {
    let (_, range, eol) = line_at(text, lines, pos);
    if pos >= range.end {
        return range.end + eol;
    }
    let end = range.end;
    match stop {
        WordStop::End => {
            let pos = skip_right(text, pos, end, Kind::Space);
            if pos == end {
                return pos;
            }
            skip_right(text, pos, end, kind(char_from(text, pos, end).0))
        }
        WordStop::NextStart => {
            let first = kind(char_from(text, pos, end).0);
            let pos = if first == Kind::Space { pos } else { skip_right(text, pos, end, first) };
            skip_right(text, pos, end, Kind::Space)
        }
    }
}

/// One word to the left: over spaces, then to the start of a word; at the start of a line, over
/// the break before it.
pub fn word_left<S: TextStore>(text: &S, lines: &LineIndex, pos: usize) -> usize {
    let (line, range, _) = line_at(text, lines, pos);
    if pos <= range.start {
        return if line > 0 { lines.line_range(text, line - 1).0.end } else { 0 };
    }
    let pos = skip_left(text, pos.min(range.end), range.start, Kind::Space);
    if pos == range.start {
        return pos;
    }
    skip_left(text, pos, range.start, kind(char_before(text, pos, range.start).0))
}

/// The word, run of spaces or run of other characters that the character at `pos` belongs to,
/// for a double click; at the end of a line, the one before it.
pub fn word_at<S: TextStore>(text: &S, lines: &LineIndex, pos: usize) -> Range<usize> {
    let (_, range, _) = line_at(text, lines, pos);
    let pos = pos.min(range.end);
    let kind_of = if pos < range.end {
        kind(char_from(text, pos, range.end).0)
    } else if pos > range.start {
        kind(char_before(text, pos, range.start).0)
    } else {
        return pos..pos;
    };
    skip_left(text, pos, range.start, kind_of)..skip_right(text, pos, range.end, kind_of)
}

#[cfg(test)]
mod tests {
    use migpad_core::text::{GapBuffer, Indexer};

    use super::*;

    fn text(s: &[u8]) -> (GapBuffer, LineIndex) {
        let text = GapBuffer::from_vec(s.to_vec());
        let lines = Indexer::index(&text).lines;
        (text, lines)
    }

    /// Every stop from the start to the end of the text, moving right with `step`.
    fn stops(s: &[u8], step: impl Fn(&GapBuffer, &LineIndex, usize) -> usize) -> Vec<usize> {
        let (text, lines) = text(s);
        let mut stops = vec![0];
        while *stops.last().unwrap() < text.len() {
            let next = step(&text, &lines, *stops.last().unwrap());
            assert!(next > *stops.last().unwrap(), "stuck at {}", stops.last().unwrap());
            stops.push(next);
        }
        stops
    }

    /// Every stop from the end to the start of the text, moving left with `step`, in ascending order.
    fn stops_back(s: &[u8], step: impl Fn(&GapBuffer, &LineIndex, usize) -> usize) -> Vec<usize> {
        let (text, lines) = text(s);
        let mut stops = vec![text.len()];
        while *stops.last().unwrap() > 0 {
            let prev = step(&text, &lines, *stops.last().unwrap());
            assert!(prev < *stops.last().unwrap(), "stuck at {}", stops.last().unwrap());
            stops.push(prev);
        }
        stops.reverse();
        stops
    }

    #[test]
    fn characters_step_over_line_breaks_at_once() {
        // a, CRLF, ж (2 bytes), LF, CR, then b.
        let s = "a\r\nж\n\rb".as_bytes();
        let want = vec![0, 1, 3, 5, 6, 7, 8];
        assert_eq!(stops(s, next_char), want);
        assert_eq!(stops_back(s, prev_char), want);
    }

    #[test]
    fn characters_step_over_invalid_utf8_parts() {
        // E2 82 is one invalid part, FF another.
        let s = b"x\xE2\x82\xFFy";
        let want = vec![0, 1, 3, 4, 5];
        assert_eq!(stops(s, next_char), want);
        assert_eq!(stops_back(s, prev_char), want);
    }

    #[test]
    fn words_to_the_right_stop_at_their_ends() {
        let s = "foo, bar_1  ёлка\n  x".as_bytes();
        let (text, lines) = text(s);
        let right = |pos| word_right(&text, &lines, pos, WordStop::End);
        // foo | , | bar_1 | ёлка (8 bytes) | line break | indent and x.
        assert_eq!(right(0), 3);
        assert_eq!(right(3), 4);
        assert_eq!(right(4), 10);
        assert_eq!(right(10), 20);
        assert_eq!(right(20), 21, "over the line break");
        assert_eq!(right(21), 24);
        assert_eq!(right(24), 24, "the end of the text");
    }

    #[test]
    fn words_to_the_right_stop_at_the_next_start_on_windows() {
        let s = b"foo, bar  baz\n  x";
        let want = vec![0, 3, 5, 10, 13, 14, 16, 17];
        assert_eq!(stops(s, |text, lines, pos| word_right(text, lines, pos, WordStop::NextStart)), want);
    }

    #[test]
    fn words_to_the_left_stop_at_their_starts() {
        let s = b"foo, bar  baz\n  x";
        let want = vec![0, 3, 5, 10, 13, 14, 16, 17];
        assert_eq!(stops_back(s, word_left), want);
    }

    #[test]
    fn words_end_at_trailing_spaces_and_line_breaks() {
        let s = b"ab   \r\ncd";
        let (text, lines) = text(s);
        assert_eq!(word_right(&text, &lines, 2, WordStop::End), 5, "spaces up to the line end");
        assert_eq!(word_right(&text, &lines, 5, WordStop::End), 7, "over CRLF at once");
        assert_eq!(word_left(&text, &lines, 7), 5);
        assert_eq!(word_left(&text, &lines, 5), 0);
    }

    #[test]
    fn the_word_at_a_position() {
        let s = "say: «привет_1»  ok\n".as_bytes();
        let (text, lines) = text(s);
        let word = |pos| {
            let range = word_at(&text, &lines, pos);
            std::str::from_utf8(&s[range]).unwrap()
        };
        assert_eq!(word(0), "say");
        assert_eq!(word(2), "say");
        assert_eq!(word(3), ":");
        assert_eq!(word(4), " ");
        // « and » are punctuation, each a run of its own here: the word is between them.
        assert_eq!(word(5), "«");
        assert_eq!(word(7), "привет_1");
        assert_eq!(word(21), "»");
        assert_eq!(word(24), "  ");
        assert_eq!(word(27), "ok", "at the end of the line: the word before it");
        assert_eq!(word_at(&text, &lines, 28), 28..28, "an empty line");
    }

    #[test]
    fn positions_snap_to_where_the_caret_can_be() {
        let s = "aж\r\nb".as_bytes();
        let (text, lines) = text(s);
        assert_eq!(snap(&text, &lines, 2), 1, "inside ж: its start");
        assert_eq!(snap(&text, &lines, 4), 3, "inside CRLF: the end of the line");
        assert_eq!(snap(&text, &lines, 5), 5);
        assert_eq!(snap(&text, &lines, 99), 6, "past the end: the end");
    }

    #[test]
    fn lines_with_their_breaks() {
        let (text, lines) = text(b"one\r\ntwo");
        assert_eq!(line_with_break(&text, &lines, 0), 0..5);
        assert_eq!(line_with_break(&text, &lines, 1), 5..8);
        assert_eq!(line_end(&text, &lines, 1), 3);
    }
}
