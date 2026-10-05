//! Line starts of a text, kept up to date as the text is edited.
//!
//! Lines end with LF, CRLF or CR, so a file with mixed line ends keeps them byte for byte.
//! The layout follows Scintilla's `Partitioning`: line starts in a vector with a gap, plus a shift
//! ("step") applied lazily to the lines after the last edit, so typing costs O(1) instead of
//! updating every following line.

use std::fmt;
use std::ops::Range;

use memchr::memchr2_iter;

use super::{MAX_LEN, TextStore};

/// Start offsets of the lines of a text.
pub struct LineIndex {
    starts: SplitVec,
    /// The starts of the lines after this one still lack `step` (added with wrapping).
    step_line: usize,
    step: u32,
}

impl LineIndex {
    /// The index of an empty text: one empty line.
    pub fn new() -> Self {
        Self::from_starts(vec![0])
    }

    /// `starts` must begin with 0 and increase.
    fn from_starts(starts: Vec<u32>) -> Self {
        debug_assert_eq!(starts.first(), Some(&0));
        let last = starts.len() - 1;
        LineIndex { starts: SplitVec::from_vec(starts), step_line: last, step: 0 }
    }

    /// Number of lines. A text that ends with a line break has an empty last line.
    #[inline]
    pub fn count(&self) -> usize {
        self.starts.len()
    }

    /// Offset of the first byte of `line`.
    ///
    /// # Panics
    ///
    /// If `line >= self.count()`.
    #[inline]
    pub fn start(&self, line: usize) -> usize {
        let start = self.starts.get(line);
        (if line > self.step_line { start.wrapping_add(self.step) } else { start }) as usize
    }

    /// The line that contains byte `pos`; the end of the text belongs to the last line.
    pub fn line_of(&self, pos: usize) -> usize {
        let (mut lo, mut hi) = (0, self.count());
        while hi - lo > 1 {
            let mid = (lo + hi) / 2;
            if self.start(mid) <= pos {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        lo
    }

    /// The bytes of `line` without its line break, and the length of the line break: 0, 1 or 2.
    pub fn line_range<S: TextStore>(&self, text: &S, line: usize) -> (Range<usize>, usize) {
        let start = self.start(line);
        if line + 1 == self.count() {
            return (start..text.len(), 0);
        }
        let next = self.start(line + 1);
        let mut end = next - 1;
        if text.byte(end) == b'\n' && end > start && text.byte(end - 1) == b'\r' {
            end -= 1;
        }
        (start..end, next - end)
    }

    /// Updates the index after the bytes of `range` were replaced with `inserted` bytes;
    /// `text` must already hold the new text.
    pub fn on_edit<S: TextStore>(&mut self, text: &S, range: Range<usize>, inserted: usize) {
        debug_assert!(text.len() <= MAX_LEN);
        let (pos, deleted) = (range.start, range.len());
        if deleted == 0 && inserted == 0 {
            return;
        }
        // A line start at p depends on bytes p - 1 and p, so only the starts in [pos, pos + deleted]
        // of the old text and in [pos, pos + inserted] of the new one can change. Line 0 always
        // starts at 0.
        let lo_pos = pos.max(1);
        let lo = self.line_of(lo_pos - 1) + 1;
        let hi = self.line_of(pos + deleted) + 1;
        if hi > lo {
            self.remove_range(lo, hi);
        }
        self.shift_after(lo - 1, (inserted as u32).wrapping_sub(deleted as u32));
        let mut fresh = Vec::new();
        scan_starts(text, lo_pos - 1..pos + inserted, &mut fresh);
        if !fresh.is_empty() {
            self.insert_at(lo, &fresh);
        }
    }

    /// Resets the step if it is zero or no lines are left after `step_line`.
    fn normalize(&mut self) {
        let last = self.count() - 1;
        if self.step == 0 || self.step_line >= last {
            self.step_line = last;
            self.step = 0;
        }
    }

    /// Adds the pending step to the lines up to `upto`.
    fn apply_step(&mut self, upto: usize) {
        if self.step != 0 && upto > self.step_line {
            self.starts.add_range(self.step_line + 1..upto + 1, self.step);
        }
        self.step_line = upto;
        self.normalize();
    }

    /// Takes the step back from the lines after `downto`, making it pending for them again.
    fn back_step(&mut self, downto: usize) {
        if self.step != 0 && downto < self.step_line {
            self.starts.add_range(downto + 1..self.step_line + 1, self.step.wrapping_neg());
        }
        self.step_line = downto;
    }

    /// Adds `delta` (wrapping) to the starts of all lines after `line`.
    fn shift_after(&mut self, line: usize, delta: u32) {
        let count = self.count();
        if delta == 0 || line + 1 >= count {
            return;
        }
        if self.step == 0 {
            self.step_line = line;
            self.step = delta;
        } else if line >= self.step_line {
            self.apply_step(line);
            self.step = self.step.wrapping_add(delta);
        } else if line + count / 10 >= self.step_line {
            // Close behind the step: take it back over the few lines in between.
            self.back_step(line);
            self.step = self.step.wrapping_add(delta);
        } else {
            self.apply_step(count - 1);
            self.step_line = line;
            self.step = delta;
        }
        self.normalize();
    }

    /// Removes the starts of lines `lo..hi`; `lo >= 1`.
    fn remove_range(&mut self, lo: usize, hi: usize) {
        let n = hi - lo;
        if hi - 1 <= self.step_line {
            self.step_line -= n;
        } else if lo <= self.step_line {
            self.step_line = lo - 1;
        }
        self.starts.remove_range(lo, n);
        self.normalize();
    }

    /// Inserts final (already shifted) starts before line `at`.
    fn insert_at(&mut self, at: usize, starts: &[u32]) {
        if self.step_line + 1 < at {
            self.apply_step(at - 1);
        }
        self.step_line += starts.len();
        self.starts.insert_slice(at, starts);
        self.normalize();
    }
}

impl Default for LineIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for LineIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LineIndex").field("count", &self.count()).finish()
    }
}

/// Pushes the line starts that follow the line breaks in `range` of `text`; the byte after
/// the range is read to tell CRLF from CR.
fn scan_starts<S: TextStore>(text: &S, range: Range<usize>, out: &mut Vec<u32>) {
    let len = text.len();
    let mut base = range.start;
    for chunk in text.chunks(range) {
        for i in memchr2_iter(b'\n', b'\r', chunk) {
            let p = base + i;
            if chunk[i] == b'\n' || p + 1 >= len || text.byte(p + 1) != b'\n' {
                out.push((p + 1) as u32);
            }
        }
        base += chunk.len();
    }
}

/// Numbers of line breaks of each kind in a text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EolCounts {
    pub lf: u64,
    pub crlf: u64,
    pub cr: u64,
}

/// A text indexed by [`Indexer`].
#[derive(Debug)]
pub struct Indexed {
    pub lines: LineIndex,
    pub eol: EolCounts,
    /// Length in bytes of the longest line, its line break included.
    pub longest_line: usize,
}

/// Builds a [`LineIndex`] from consecutive chunks of a text, as the text is being loaded.
pub struct Indexer {
    starts: Vec<u32>,
    eol: EolCounts,
    longest_line: usize,
    len: usize,
    /// The last chunk ended with CR: a CRLF may continue into the next one.
    pending_cr: bool,
}

impl Indexer {
    /// `lines_hint` is the expected number of lines, to reserve memory.
    pub fn new(lines_hint: usize) -> Self {
        let mut starts = Vec::with_capacity(lines_hint.max(16));
        starts.push(0);
        Indexer { starts, eol: EolCounts::default(), longest_line: 0, len: 0, pending_cr: false }
    }

    /// Indexes a whole text.
    pub fn index<S: TextStore>(text: &S) -> Indexed {
        let mut indexer = Indexer::new(0);
        for chunk in text.chunks(0..text.len()) {
            indexer.feed(chunk);
        }
        indexer.finish()
    }

    /// Indexes the next chunk of the text.
    ///
    /// # Panics
    ///
    /// If the text grows beyond [`MAX_LEN`].
    pub fn feed(&mut self, chunk: &[u8]) {
        if chunk.is_empty() {
            return;
        }
        let base = self.len;
        self.len += chunk.len();
        assert!(self.len <= MAX_LEN, "text longer than {MAX_LEN} bytes");
        let prev_cr = std::mem::take(&mut self.pending_cr);
        if prev_cr && chunk[0] != b'\n' {
            self.push(base);
            self.eol.cr += 1;
        }
        for i in memchr2_iter(b'\n', b'\r', chunk) {
            if chunk[i] == b'\n' {
                let after_cr = if i > 0 { chunk[i - 1] == b'\r' } else { prev_cr };
                if after_cr {
                    self.eol.crlf += 1;
                } else {
                    self.eol.lf += 1;
                }
                self.push(base + i + 1);
            } else if i + 1 < chunk.len() {
                if chunk[i + 1] != b'\n' {
                    self.push(base + i + 1);
                    self.eol.cr += 1;
                }
            } else {
                self.pending_cr = true;
            }
        }
    }

    /// Finishes the index once the whole text has been fed.
    pub fn finish(mut self) -> Indexed {
        if self.pending_cr {
            self.push(self.len);
            self.eol.cr += 1;
        }
        let last = *self.starts.last().unwrap() as usize;
        self.longest_line = self.longest_line.max(self.len - last);
        Indexed { lines: LineIndex::from_starts(self.starts), eol: self.eol, longest_line: self.longest_line }
    }

    fn push(&mut self, start: usize) {
        let last = *self.starts.last().unwrap() as usize;
        self.longest_line = self.longest_line.max(start - last);
        self.starts.push(start as u32);
    }
}

/// A vector of `u32` with a gap, like Scintilla's `SplitVector`.
struct SplitVec {
    data: Vec<u32>,
    gap_start: usize,
    gap_len: usize,
}

impl SplitVec {
    fn from_vec(mut data: Vec<u32>) -> Self {
        let len = data.len();
        let gap = (len / 64).max(256);
        data.resize(len + gap, 0);
        SplitVec { data, gap_start: len, gap_len: gap }
    }

    #[inline]
    fn len(&self) -> usize {
        self.data.len() - self.gap_len
    }

    #[inline]
    fn get(&self, i: usize) -> u32 {
        if i < self.gap_start { self.data[i] } else { self.data[i + self.gap_len] }
    }

    fn move_gap(&mut self, pos: usize) {
        if pos < self.gap_start {
            self.data.copy_within(pos..self.gap_start, pos + self.gap_len);
        } else if pos > self.gap_start {
            self.data.copy_within(self.gap_start + self.gap_len..pos + self.gap_len, self.gap_start);
        }
        self.gap_start = pos;
    }

    fn insert_slice(&mut self, at: usize, values: &[u32]) {
        self.move_gap(at);
        if self.gap_len < values.len() {
            let grow = values.len().max(self.len() / 8).max(256);
            let old_len = self.data.len();
            self.data.resize(old_len + grow, 0);
            let tail = self.gap_start + self.gap_len;
            self.data.copy_within(tail..old_len, tail + grow);
            self.gap_len += grow;
        }
        self.data[self.gap_start..self.gap_start + values.len()].copy_from_slice(values);
        self.gap_start += values.len();
        self.gap_len -= values.len();
    }

    fn remove_range(&mut self, at: usize, n: usize) {
        self.move_gap(at);
        self.gap_len += n;
    }

    /// Adds `delta` (wrapping) to the elements of `range`.
    fn add_range(&mut self, range: Range<usize>, delta: u32) {
        let before_gap_end = range.end.min(self.gap_start);
        if range.start < before_gap_end {
            for x in &mut self.data[range.start..before_gap_end] {
                *x = x.wrapping_add(delta);
            }
        }
        let after_gap_start = range.start.max(self.gap_start);
        if after_gap_start < range.end {
            for x in &mut self.data[after_gap_start + self.gap_len..range.end + self.gap_len] {
                *x = x.wrapping_add(delta);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::GapBuffer;

    /// Line starts by definition: after every LF, and after every CR not followed by LF.
    fn naive_starts(text: &[u8]) -> Vec<usize> {
        let mut starts = vec![0];
        for i in 0..text.len() {
            if text[i] == b'\n' || (text[i] == b'\r' && text.get(i + 1) != Some(&b'\n')) {
                starts.push(i + 1);
            }
        }
        starts
    }

    fn starts(index: &LineIndex) -> Vec<usize> {
        (0..index.count()).map(|line| index.start(line)).collect()
    }

    fn rng(mut seed: u64) -> impl FnMut(usize) -> usize {
        move |n| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((seed >> 33) as usize) % n.max(1)
        }
    }

    fn random_edits(init: &[u8], rounds: usize, max_delete: usize) {
        let mut rnd = rng(12345);
        let alphabet = b"ab\r\n\n\rcx";
        let mut model = init.to_vec();
        let mut text = GapBuffer::from_vec(init.to_vec());
        let mut index = Indexer::index(&text).lines;
        assert_eq!(starts(&index), naive_starts(&model));
        for _ in 0..rounds {
            let len = model.len();
            let pos = rnd(len + 1);
            let end = pos + rnd((len - pos).min(max_delete) + 1);
            let insert: Vec<u8> = (0..rnd(4)).map(|_| alphabet[rnd(alphabet.len())]).collect();
            text.replace(pos..end, &insert);
            index.on_edit(&text, pos..end, insert.len());
            model.splice(pos..end, insert.iter().copied());
            assert_eq!(text.to_vec(0..text.len()), model);
            assert_eq!(starts(&index), naive_starts(&model), "text {:?}", String::from_utf8_lossy(&model));
        }
    }

    #[test]
    fn random_edits_keep_the_index_consistent() {
        random_edits(b"", 2000, 4);
        random_edits(b"hello\nworld\r\nfoo\rbar\n", 20_000, 4);
        let mut big = Vec::new();
        for i in 0..3000 {
            big.extend_from_slice(format!("line {i}{}", ["\n", "\r\n", "\r"][i % 3]).as_bytes());
        }
        random_edits(&big, 3000, 40);
    }

    #[test]
    fn indexer_handles_any_chunk_boundary() {
        let text = b"a\r\nb\rc\nd\r\r\ne\r";
        for split in 0..=text.len() {
            let mut indexer = Indexer::new(4);
            indexer.feed(&text[..split]);
            indexer.feed(b"");
            indexer.feed(&text[split..]);
            let indexed = indexer.finish();
            assert_eq!(starts(&indexed.lines), naive_starts(text), "split {split}");
            assert_eq!(indexed.eol, EolCounts { lf: 1, crlf: 2, cr: 3 }, "split {split}");
        }
    }

    #[test]
    fn indexer_reports_the_longest_line() {
        let indexed = Indexer::index(&GapBuffer::from_vec(b"ab\r\nabcdef\nabc".to_vec()));
        assert_eq!(indexed.lines.count(), 3);
        assert_eq!(indexed.longest_line, 7);
        let empty = Indexer::index(&GapBuffer::new());
        assert_eq!((empty.lines.count(), empty.longest_line), (1, 0));
    }

    #[test]
    fn line_ranges_exclude_line_breaks() {
        let text = GapBuffer::from_vec(b"lf\ncrlf\r\ncr\rlast".to_vec());
        let index = Indexer::index(&text).lines;
        let ranges: Vec<_> = (0..index.count()).map(|line| index.line_range(&text, line)).collect();
        assert_eq!(ranges, [(0..2, 1), (3..7, 2), (9..11, 1), (12..16, 0)]);

        let text = GapBuffer::from_vec(b"a\n".to_vec());
        let index = Indexer::index(&text).lines;
        assert_eq!(index.count(), 2);
        assert_eq!(index.line_range(&text, 1), (2..2, 0));
    }

    #[test]
    fn line_of_finds_the_line_of_each_byte() {
        let text = GapBuffer::from_vec(b"ab\ncd\r\n\nef".to_vec());
        let index = Indexer::index(&text).lines;
        let lines: Vec<usize> = (0..=text.len()).map(|pos| index.line_of(pos)).collect();
        assert_eq!(lines, [0, 0, 0, 1, 1, 1, 1, 2, 3, 3, 3]);
    }
}
