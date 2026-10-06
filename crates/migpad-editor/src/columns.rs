//! Display columns of long lines — characters, tabs reaching to their stops — kept at checkpoints,
//! so that the column of a byte and the character at a column take one short scan however long
//! the line is.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;

use migpad_core::text::TextStore;

/// Bytes of a line between checkpoints: a scan from one is short enough for every frame.
const STEP: usize = 4 << 10;

/// Checkpoints of the long lines laid out since the text last changed. For each line, by the
/// offset where it starts: pairs of a character boundary and its column, ascending, starting at
/// the start of the line; they are found as far into the line as asked for.
pub(crate) struct Columns {
    lines: RefCell<HashMap<usize, Vec<(usize, usize)>>>,
    tab_width: usize,
    step: usize,
}

impl Columns {
    pub fn new(tab_width: usize) -> Self {
        Self::with_step(tab_width, STEP)
    }

    fn with_step(tab_width: usize, step: usize) -> Self {
        Columns { lines: RefCell::default(), tab_width, step }
    }

    /// Forgets every checkpoint: the text has changed.
    pub fn clear(&mut self) {
        self.lines.get_mut().clear();
    }

    /// The column of byte `pos` of the line with the bytes `range`; inside a character, the
    /// column of its start.
    pub fn column_of<S: TextStore>(&self, text: &S, range: &Range<usize>, pos: usize) -> usize {
        let pos = pos.clamp(range.start, range.end);
        let (from, column) = self.checkpoint(text, range, |byte, _| byte <= pos);
        // The bytes up to the character that `pos` belongs to, all of it.
        let bytes = text.to_vec(from..range.end.min(pos + 4));
        walk(&bytes, column, self.tab_width, |at, len, _, _| from + at + len <= pos).1
    }

    /// The character at `column` of the line with the bytes `range`: where it starts, and the
    /// columns where it starts and ends, several for a tab. Past the end of the line: its end,
    /// and the column there twice.
    pub fn char_at<S: TextStore>(&self, text: &S, range: &Range<usize>, column: usize) -> (usize, usize, usize) {
        let (from, start_column) = self.checkpoint(text, range, |_, at| at <= column);
        // The character is before the next checkpoint, which starts past `column`.
        let bytes = text.to_vec(from..range.end.min(from + self.step + 4));
        let mut width = 0;
        let (at, start) = walk(&bytes, start_column, self.tab_width, |_, _, start, w| {
            width = w;
            start + w <= column
        });
        if from + at == range.end || at == bytes.len() {
            (from + at, start, start)
        } else {
            (from + at, start, start + width)
        }
    }

    /// The column of byte `pos`, as [`Columns::column_of`] finds it, if that takes walking no more
    /// than `budget` bytes of the line past the checkpoints known; otherwise `None`, the line walked
    /// that much further for the next call.
    pub fn column_within<S: TextStore>(
        &self,
        text: &S,
        range: &Range<usize>,
        pos: usize,
        budget: usize,
    ) -> Option<usize> {
        let pos = pos.clamp(range.start, range.end);
        self.extend(text, range, &|byte, _| byte <= pos, budget).then(|| self.column_of(text, range, pos))
    }

    /// The last checkpoint for which `before` holds, finding further checkpoints while the last
    /// one known does.
    fn checkpoint<S: TextStore>(
        &self,
        text: &S,
        range: &Range<usize>,
        before: impl Fn(usize, usize) -> bool,
    ) -> (usize, usize) {
        self.extend(text, range, &before, usize::MAX);
        let lines = self.lines.borrow();
        let points = &lines[&range.start];
        let i = points.partition_point(|&(byte, column)| before(byte, column));
        points[i.saturating_sub(1)]
    }

    /// Finds checkpoints while the last one known satisfies `before`, walking at most `budget`
    /// bytes; returns whether it got that far.
    fn extend<S: TextStore>(
        &self,
        text: &S,
        range: &Range<usize>,
        before: &impl Fn(usize, usize) -> bool,
        budget: usize,
    ) -> bool {
        let mut lines = self.lines.borrow_mut();
        let points = lines.entry(range.start).or_insert_with(|| vec![(range.start, 0)]);
        let mut walked = 0;
        while let Some(&(byte, column)) = points.last()
            && before(byte, column)
            && byte < range.end
        {
            if walked >= budget {
                return false;
            }
            let next = text.floor_char_boundary((byte + self.step).min(range.end), byte);
            if next <= byte {
                break;
            }
            let (_, next_column) = walk(&text.to_vec(byte..next), column, self.tab_width, |_, _, _, _| true);
            points.push((next, next_column));
            walked += next - byte;
        }
        true
    }
}

/// Walks over the characters of `bytes` from `column` while `each` says so: `each` gets the
/// offset of a character, its length, the column where it starts and the columns it takes.
/// Returns the offset and the column of the character `each` stopped at, or of the end.
fn walk(
    bytes: &[u8],
    mut column: usize,
    tab_width: usize,
    mut each: impl FnMut(usize, usize, usize, usize) -> bool,
) -> (usize, usize) {
    let mut at = 0;
    for chunk in bytes.utf8_chunks() {
        let valid = chunk.valid().chars().map(|c| (c.len_utf8(), c == '\t'));
        let invalid = (!chunk.invalid().is_empty()).then_some((chunk.invalid().len(), false));
        for (len, tab) in valid.chain(invalid) {
            let width = if tab { tab_width - column % tab_width } else { 1 };
            if !each(at, len, column, width) {
                return (at, column);
            }
            at += len;
            column += width;
        }
    }
    (at, column)
}

#[cfg(test)]
mod tests {
    use migpad_core::text::GapBuffer;

    use super::*;

    /// Columns of every character boundary of `s`, counted plainly.
    fn plain_columns(s: &[u8], tab_width: usize) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let end = walk(s, 0, tab_width, |at, _, column, _| {
            out.push((at, column));
            true
        });
        out.push(end);
        out
    }

    #[test]
    fn columns_match_a_plain_count_across_checkpoints() {
        // Tabs, two-byte letters, an invalid part and a lone continuation byte, over several steps.
        let line: Vec<u8> =
            [b"ab\t".as_slice(), "жж\t".as_bytes(), b"\xE2\x82x\x80\t", "Ёд".as_bytes()].concat().repeat(7);
        let text = GapBuffer::from_vec(line.clone());
        let range = 0..line.len();
        let columns = Columns::with_step(4, 9);
        let plain = plain_columns(&line, 4);
        for &(byte, column) in &plain {
            assert_eq!(columns.column_of(&text, &range, byte), column, "byte {byte}");
        }
        // The character at each column: the one that covers it.
        for (i, &(byte, start)) in plain.iter().enumerate().take(plain.len() - 1) {
            let end = plain[i + 1].1;
            for column in start..end {
                assert_eq!(columns.char_at(&text, &range, column), (byte, start, end), "column {column}");
            }
        }
        let (end_byte, end_column) = *plain.last().unwrap();
        assert_eq!(columns.char_at(&text, &range, end_column + 5), (end_byte, end_column, end_column));
    }

    #[test]
    fn a_far_column_is_found_a_part_at_a_time() {
        let line = "ab\tж".repeat(100);
        let text = GapBuffer::from_vec(line.clone().into_bytes());
        let range = 0..line.len();
        let columns = Columns::with_step(4, 9);
        let mut calls = 1;
        let found = loop {
            match columns.column_within(&text, &range, line.len(), 50) {
                Some(column) => break column,
                None => calls += 1,
            }
        };
        assert!(calls > 1, "a part at a time");
        assert_eq!(found, Columns::with_step(4, 9).column_of(&text, &range, line.len()));
        // Near the start, at once.
        assert_eq!(columns.column_within(&text, &range, 4, 0), Some(4));
    }

    #[test]
    fn a_byte_inside_a_character_has_its_column() {
        let line = "aжb".as_bytes();
        let text = GapBuffer::from_vec(line.to_vec());
        let columns = Columns::with_step(8, 2);
        assert_eq!(columns.column_of(&text, &(0..line.len()), 2), 1, "inside ж");
        assert_eq!(columns.column_of(&text, &(0..line.len()), 3), 2);
    }

    #[test]
    fn lines_are_told_apart_by_their_start() {
        let text = GapBuffer::from_vec(b"\tx\nab\tx".to_vec());
        let columns = Columns::with_step(4, 2);
        assert_eq!(columns.column_of(&text, &(0..2), 1), 4);
        assert_eq!(columns.column_of(&text, &(3..7), 6), 4);
        assert_eq!(columns.char_at(&text, &(3..7), 3), (5, 2, 4));
    }
}
