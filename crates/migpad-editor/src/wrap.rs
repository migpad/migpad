//! Word wrap: where the rows of a line start when it is wrapped to a width in cells, the room of a
//! character of the monospace font. Counting cells needs no shaping, so a line of megabytes wraps
//! quickly.

use std::ops::Range;

use migpad_core::text::TextStore;

/// Bytes of a line read at once.
const PIECE: usize = 64 << 10;

/// Closing punctuation of East Asian text, which a row does not start with.
const NO_BREAK_BEFORE: &str = "、。，．・：；？！）」』】〉》〕］｝";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Space,
    /// East Asian wide characters and emoji: two cells, and a row may break next to them.
    Wide,
    Other,
}

fn kind(c: char) -> Kind {
    match c {
        ' ' | '\t' | '\u{3000}' => Kind::Space,
        _ if is_wide(c) => Kind::Wide,
        _ => Kind::Other,
    }
}

/// Characters that take two cells in a monospace font: East Asian wide and full-width ones,
/// and emoji.
fn is_wide(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x115F
            | 0x2E80..=0x303E
            | 0x3041..=0x33FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xA000..=0xA4CF
            | 0xAC00..=0xD7A3
            | 0xF900..=0xFAFF
            | 0xFE30..=0xFE4F
            | 0xFF00..=0xFF60
            | 0xFFE0..=0xFFE6
            | 0x1F300..=0x1F64F
            | 0x1F900..=0x1F9FF
            | 0x20000..=0x3FFFD)
}

/// Whether a row may start with `c`, of kind `kind`, after a character of kind `prev`: after
/// spaces, and next to wide characters.
fn breaks_before(prev: Kind, kind: Kind, c: char) -> bool {
    match (prev, kind) {
        (_, Kind::Space) => false,
        (Kind::Space, _) => true,
        (Kind::Wide, _) | (_, Kind::Wide) => !NO_BREAK_BEFORE.contains(c),
        _ => false,
    }
}

/// Where the rows of the line with the bytes `range` start when it wraps to `width` cells: the
/// start of the line, then the start of each row it wraps to. A character takes a cell, a wide
/// one two, a tab reaches to the next of the stops every `tab_width` columns. Rows break after
/// spaces and next to wide characters, a word wider than a row is cut, and spaces at the end of
/// a row may run past its edge.
pub fn row_starts<S: TextStore>(text: &S, range: Range<usize>, width: usize, tab_width: usize) -> Vec<usize> {
    let mut starts = vec![range.start];
    // Columns of the line, for tab stops; cells of the row before the current character; where
    // the row may break, with the cells before that.
    let (mut column, mut cells) = (0, 0);
    let mut candidate: Option<(usize, usize)> = None;
    let mut prev: Option<Kind> = None;
    let mut piece = range.start;
    while piece < range.end {
        let end = if range.end - piece > PIECE { text.floor_char_boundary(piece + PIECE, piece) } else { range.end };
        let bytes = text.to_vec(piece..end);
        for (offset, c) in chars(&bytes) {
            let at = piece + offset;
            let kind = kind(c);
            let size = match c {
                '\t' => tab_width - column % tab_width,
                _ if is_wide(c) => 2,
                _ => 1,
            };
            let row_start = *starts.last().unwrap();
            if at > row_start && prev.is_some_and(|prev| breaks_before(prev, kind, c)) {
                candidate = Some((at, cells));
            }
            if kind != Kind::Space && cells + size > width && at > row_start {
                match candidate.take() {
                    Some((break_at, before)) if break_at > row_start => {
                        starts.push(break_at);
                        cells -= before;
                    }
                    _ => {
                        starts.push(at);
                        cells = 0;
                    }
                }
                // A word wider than a row, from its start: cut here too.
                if cells + size > width && at > *starts.last().unwrap() {
                    starts.push(at);
                    cells = 0;
                }
            }
            cells += size;
            column += if c == '\t' { size } else { 1 };
            prev = Some(kind);
        }
        piece = end.max(piece + 1);
    }
    starts
}

/// The characters of `bytes` and where they start; an invalid part of UTF-8 is one U+FFFD.
fn chars(bytes: &[u8]) -> impl Iterator<Item = (usize, char)> + '_ {
    let mut at = 0;
    bytes.utf8_chunks().flat_map(move |chunk| {
        let start = at;
        at += chunk.valid().len() + chunk.invalid().len();
        let valid = chunk.valid().char_indices().map(move |(offset, c)| (start + offset, c));
        let invalid =
            (!chunk.invalid().is_empty()).then_some((start + chunk.valid().len(), char::REPLACEMENT_CHARACTER));
        valid.chain(invalid)
    })
}

#[cfg(test)]
mod tests {
    use migpad_core::text::GapBuffer;

    use super::*;

    /// The rows of `line` wrapped to `width` cells.
    fn rows(line: &str, width: usize) -> Vec<&str> {
        let text = GapBuffer::from_vec(line.as_bytes().to_vec());
        let starts = row_starts(&text, 0..line.len(), width, 8);
        let ends = starts.iter().skip(1).copied().chain([line.len()]);
        starts.iter().zip(ends).map(|(&start, end)| &line[start..end]).collect()
    }

    #[test]
    fn rows_break_after_spaces() {
        assert_eq!(rows("aaa bbb ccc", 7), ["aaa bbb ", "ccc"]);
        assert_eq!(rows("съешь же ещё этих", 10), ["съешь же ", "ещё этих"]);
        assert_eq!(rows("fits", 4), ["fits"]);
    }

    #[test]
    fn words_wider_than_a_row_are_cut() {
        assert_eq!(rows("abcdefghij", 4), ["abcd", "efgh", "ij"]);
        assert_eq!(rows("ab cdefghij", 4), ["ab ", "cdef", "ghij"]);
    }

    #[test]
    fn spaces_run_past_the_edge() {
        assert_eq!(rows("ab      cd", 4), ["ab      ", "cd"]);
    }

    #[test]
    fn tabs_reach_to_their_stops() {
        // A tab at column 2 takes 6 cells: the row is full after it.
        assert_eq!(rows("ab\tcd", 8), ["ab\t", "cd"]);
        assert_eq!(rows("ab\tcd", 10), ["ab\tcd"]);
    }

    #[test]
    fn wide_characters_take_two_cells_and_break_anywhere() {
        assert_eq!(rows("漢字漢字", 5), ["漢字", "漢字"]);
        assert_eq!(rows("ab漢字", 5), ["ab漢", "字"]);
        // A row does not start with closing punctuation.
        assert_eq!(rows("漢字。漢", 6), ["漢字。", "漢"]);
    }

    #[test]
    fn invalid_utf8_is_a_character_of_one_cell() {
        let text = GapBuffer::from_vec(b"ab\xFFcd".to_vec());
        assert_eq!(row_starts(&text, 0..5, 3, 8), [0, 3]);
    }

    #[test]
    fn long_lines_wrap_across_pieces() {
        let line = "word ".repeat(30_000);
        let text = GapBuffer::from_vec(line.as_bytes().to_vec());
        let starts = row_starts(&text, 0..line.len(), 12, 8);
        // Two words and their spaces a row: "word word ".
        assert_eq!(starts.len(), 15_000);
        assert!(starts.windows(2).all(|pair| pair[1] - pair[0] == 10));
    }
}
