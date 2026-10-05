//! A line of the text as it is shown: tabs expanded, invalid UTF-8 as U+FFFD, invisible
//! characters marked, and where each character of the line ends up on screen.

use std::ops::Range;

/// Lines longer than this many bytes are shaped only in a window around the visible part.
pub const MAX_SHAPED: usize = 4096;

/// Marks of whitespace, when it is shown: a space, a no-break space, the start of a tab.
const SPACE_MARK: char = '·';
const NO_BREAK_SPACE_MARK: char = '°';
const TAB_MARK: char = '→';

/// The text of (part of) a line as it is shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayText {
    pub text: String,
    /// Where each character of the line part is in `text`.
    pub map: OffsetMap,
    /// Ranges of `text` that mark what is otherwise invisible: whitespace when it is shown, and
    /// control characters always. They are drawn faintly.
    pub marks: Vec<Range<usize>>,
}

/// Character boundaries of a line part and of the text it is shown as.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OffsetMap {
    /// Pairs of a byte offset in the line part and a byte offset in the shown text, ascending and
    /// ending with both lengths; `None` when the two are the same.
    pairs: Option<Vec<(u32, u32)>>,
    /// The length of the shown text.
    len: usize,
}

/// Control Pictures show control characters: U+2400 and on for C0, U+2421 for DEL.
fn control_picture(c: char) -> Option<char> {
    match c {
        '\0'..='\x1F' => char::from_u32(0x2400 + c as u32),
        '\x7F' => Some('\u{2421}'),
        _ => None,
    }
}

impl DisplayText {
    /// Builds the text shown for `bytes`, a part of a line that starts at display column
    /// `column`: tab stops depend on it. With `whitespace`, spaces and tabs are marked.
    pub fn new(bytes: &[u8], column: usize, tab_width: usize, whitespace: bool) -> Self {
        let marked = bytes.iter().any(|&b| b < 0x20 || b == 0x7F || (whitespace && b == b' '))
            || (whitespace && memchr::memmem::find(bytes, "\u{A0}".as_bytes()).is_some());
        if !marked && let Ok(text) = std::str::from_utf8(bytes) {
            let map = OffsetMap { pairs: None, len: bytes.len() };
            return DisplayText { text: text.to_owned(), map, marks: Vec::new() };
        }
        let mut text = String::with_capacity(bytes.len() + 16);
        let mut pairs = Vec::with_capacity(bytes.len() + 1);
        let mut marks = Vec::new();
        let mut column = column;
        let mut offset = 0;
        for chunk in bytes.utf8_chunks() {
            for c in chunk.valid().chars() {
                pairs.push((offset as u32, text.len() as u32));
                let at = text.len();
                match c {
                    '\t' => {
                        let spaces = tab_width - column % tab_width;
                        if whitespace {
                            text.push(TAB_MARK);
                            marks.push(at..text.len());
                        }
                        text.extend(std::iter::repeat_n(' ', spaces - usize::from(whitespace)));
                        column += spaces;
                    }
                    ' ' if whitespace => {
                        text.push(SPACE_MARK);
                        marks.push(at..text.len());
                        column += 1;
                    }
                    '\u{A0}' if whitespace => {
                        text.push(NO_BREAK_SPACE_MARK);
                        marks.push(at..text.len());
                        column += 1;
                    }
                    _ => {
                        match control_picture(c) {
                            Some(picture) => {
                                text.push(picture);
                                marks.push(at..text.len());
                            }
                            None => text.push(c),
                        }
                        column += 1;
                    }
                }
                offset += c.len_utf8();
            }
            // One replacement character for the whole invalid part, as the core moves over it.
            if !chunk.invalid().is_empty() {
                pairs.push((offset as u32, text.len() as u32));
                text.push(char::REPLACEMENT_CHARACTER);
                offset += chunk.invalid().len();
                column += 1;
            }
        }
        pairs.push((offset as u32, text.len() as u32));
        let len = text.len();
        DisplayText { text, map: OffsetMap { pairs: Some(pairs), len }, marks }
    }
}

impl OffsetMap {
    /// Where the character at byte `offset` of the line part is in the shown text; an offset
    /// inside a character counts as its start.
    pub fn display_offset(&self, offset: usize) -> usize {
        match &self.pairs {
            None => offset.min(self.len),
            Some(pairs) => {
                let i = pairs.partition_point(|&(byte, _)| byte as usize <= offset).saturating_sub(1);
                pairs[i].1 as usize
            }
        }
    }

    /// The byte offset in the line part of the character boundary nearest to `display`, a
    /// character boundary of the shown text; inside an expanded tab, the nearer edge of the tab.
    pub fn byte_offset(&self, display: usize) -> usize {
        match &self.pairs {
            None => display.min(self.len),
            Some(pairs) => {
                let i = pairs.partition_point(|&(_, shown)| (shown as usize) < display).min(pairs.len() - 1);
                let nearer_before = i > 0 && display - pairs[i - 1].1 as usize <= pairs[i].1 as usize - display;
                pairs[if nearer_before { i - 1 } else { i }].0 as usize
            }
        }
    }

    /// The byte offset in the line part of the character shown at `display`, a character
    /// boundary of the shown text: inside an expanded tab, the tab.
    pub fn byte_at(&self, display: usize) -> usize {
        match &self.pairs {
            None => display.min(self.len),
            Some(pairs) => {
                let i = pairs.partition_point(|&(_, shown)| shown as usize <= display).saturating_sub(1);
                pairs[i].0 as usize
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_is_shown_as_it_is() {
        let shown = DisplayText::new("Привет, мир".as_bytes(), 0, 8, false);
        assert_eq!((shown.text.as_str(), shown.map.pairs.is_none()), ("Привет, мир", true));
        assert_eq!(shown.map.display_offset(4), 4);
        assert_eq!(shown.map.byte_offset(100), shown.text.len());
        assert_eq!(shown.map.byte_at(2), 2);
    }

    #[test]
    fn tabs_go_to_the_next_stop() {
        assert_eq!(DisplayText::new(b"\tx", 0, 8, false).text, "        x");
        assert_eq!(DisplayText::new(b"ab\tx", 0, 8, false).text, "ab      x");
        assert_eq!(DisplayText::new(b"ab\tx", 0, 4, false).text, "ab  x");
        // Columns count characters, not bytes.
        assert_eq!(DisplayText::new("жж\tx".as_bytes(), 0, 4, false).text, "жж  x");
        // A part of a line continues the columns of what came before it.
        assert_eq!(DisplayText::new(b"\tx", 6, 8, false).text, "  x");
    }

    #[test]
    fn invalid_utf8_shows_one_replacement_per_invalid_part() {
        let shown = DisplayText::new(b"a\xE2\x82b\xFF\xFE", 0, 8, false);
        assert_eq!(shown.text, "a\u{FFFD}b\u{FFFD}\u{FFFD}");
        // E2 82 is one part: the core moves over it as one character.
        assert_eq!(shown.map.display_offset(1), 1);
        assert_eq!(shown.map.display_offset(3), 4);
        assert_eq!(shown.map.byte_offset(4), 3);
        assert_eq!(shown.map.byte_at(1), 1);
    }

    #[test]
    fn offsets_map_both_ways() {
        let shown = DisplayText::new("a\tж".as_bytes(), 0, 4, false);
        assert_eq!(shown.text, "a   ж");
        let map = &shown.map;
        // Bytes: a at 0, the tab at 1 shown as 1..4, ж at 2..4 shown at 4, the end at 4 shown at 6.
        assert_eq!([0, 1, 2, 4].map(|byte| map.display_offset(byte)), [0, 1, 4, 6]);
        assert_eq!(map.display_offset(3), 4, "inside ж: its start");
        assert_eq!([0, 1, 2, 3, 4, 6].map(|display| map.byte_offset(display)), [0, 1, 1, 2, 2, 4]);
        // The character under a display offset: inside the tab, the tab.
        assert_eq!([0, 1, 2, 3, 4, 6].map(|display| map.byte_at(display)), [0, 1, 1, 1, 2, 4]);
    }

    #[test]
    fn whitespace_is_marked_when_shown() {
        let shown = DisplayText::new("a b\tc\u{A0}d".as_bytes(), 0, 8, true);
        assert_eq!(shown.text, "a·b→    c°d");
        let marked: Vec<&str> = shown.marks.iter().map(|range| &shown.text[range.clone()]).collect();
        assert_eq!(marked, ["·", "→", "°"]);
        // The marks keep the columns: after the tab, c is at the stop at 8.
        assert_eq!(shown.map.display_offset(4), "a·b→    ".len());
        assert_eq!(shown.map.byte_offset("a·".len()), 2);
        // Without whitespace shown, the same text has no marks.
        assert!(DisplayText::new(b"a b", 0, 4, false).marks.is_empty());
    }

    #[test]
    fn control_characters_are_always_marked() {
        let shown = DisplayText::new(b"a\x00b\x1Bc\x7F", 0, 8, false);
        assert_eq!(shown.text, "a\u{2400}b\u{241B}c\u{2421}");
        assert_eq!(shown.marks.len(), 3);
        // Each is one character of the line.
        assert_eq!(shown.map.byte_offset(shown.text.len()), 6);
        assert_eq!(shown.map.display_offset(2), "a\u{2400}".len());
    }
}
