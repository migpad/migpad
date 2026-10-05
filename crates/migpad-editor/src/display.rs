//! A line of the text as it is shown: tabs expanded, invalid UTF-8 as U+FFFD, and where each
//! character of the line ends up on screen.

/// Lines longer than this many bytes are shaped only in a window around the visible part.
pub const MAX_SHAPED: usize = 4096;

/// The text of (part of) a line as it is shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayText {
    pub text: String,
    /// Where each character of the line part is in `text`.
    pub map: OffsetMap,
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

impl DisplayText {
    /// Builds the text shown for `bytes`, a part of a line that starts at display column
    /// `column`: tab stops depend on it.
    pub fn new(bytes: &[u8], column: usize, tab_width: usize) -> Self {
        let plain = memchr::memchr(b'\t', bytes).is_none();
        if plain && let Ok(text) = std::str::from_utf8(bytes) {
            return DisplayText { text: text.to_owned(), map: OffsetMap { pairs: None, len: bytes.len() } };
        }
        let mut text = String::with_capacity(bytes.len() + 16);
        let mut pairs = Vec::with_capacity(bytes.len() + 1);
        let mut column = column;
        let mut offset = 0;
        for chunk in bytes.utf8_chunks() {
            for c in chunk.valid().chars() {
                pairs.push((offset as u32, text.len() as u32));
                if c == '\t' {
                    let spaces = tab_width - column % tab_width;
                    text.extend(std::iter::repeat_n(' ', spaces));
                    column += spaces;
                } else {
                    text.push(c);
                    column += 1;
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
        DisplayText { text, map: OffsetMap { pairs: Some(pairs), len } }
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
        let shown = DisplayText::new("Привет, мир".as_bytes(), 0, 8);
        assert_eq!((shown.text.as_str(), shown.map.pairs.is_none()), ("Привет, мир", true));
        assert_eq!(shown.map.display_offset(4), 4);
        assert_eq!(shown.map.byte_offset(100), shown.text.len());
        assert_eq!(shown.map.byte_at(2), 2);
    }

    #[test]
    fn tabs_go_to_the_next_stop() {
        assert_eq!(DisplayText::new(b"\tx", 0, 8).text, "        x");
        assert_eq!(DisplayText::new(b"ab\tx", 0, 8).text, "ab      x");
        assert_eq!(DisplayText::new(b"ab\tx", 0, 4).text, "ab  x");
        // Columns count characters, not bytes.
        assert_eq!(DisplayText::new("жж\tx".as_bytes(), 0, 4).text, "жж  x");
        // A part of a line continues the columns of what came before it.
        assert_eq!(DisplayText::new(b"\tx", 6, 8).text, "  x");
    }

    #[test]
    fn invalid_utf8_shows_one_replacement_per_invalid_part() {
        let shown = DisplayText::new(b"a\xE2\x82b\xFF\xFE", 0, 8);
        assert_eq!(shown.text, "a\u{FFFD}b\u{FFFD}\u{FFFD}");
        // E2 82 is one part: the core moves over it as one character.
        assert_eq!(shown.map.display_offset(1), 1);
        assert_eq!(shown.map.display_offset(3), 4);
        assert_eq!(shown.map.byte_offset(4), 3);
        assert_eq!(shown.map.byte_at(1), 1);
    }

    #[test]
    fn offsets_map_both_ways() {
        let shown = DisplayText::new("a\tж".as_bytes(), 0, 4);
        assert_eq!(shown.text, "a   ж");
        let map = &shown.map;
        // Bytes: a at 0, the tab at 1 shown as 1..4, ж at 2..4 shown at 4, the end at 4 shown at 6.
        assert_eq!([0, 1, 2, 4].map(|byte| map.display_offset(byte)), [0, 1, 4, 6]);
        assert_eq!(map.display_offset(3), 4, "inside ж: its start");
        assert_eq!([0, 1, 2, 3, 4, 6].map(|display| map.byte_offset(display)), [0, 1, 1, 2, 2, 4]);
        // The character under a display offset: inside the tab, the tab.
        assert_eq!([0, 1, 2, 3, 4, 6].map(|display| map.byte_at(display)), [0, 1, 1, 1, 2, 4]);
    }
}
