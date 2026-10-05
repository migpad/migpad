//! A gap buffer: the whole text in one allocation, with a movable gap where the last edit was.

use std::fmt;
use std::ops::Range;

use super::TextStore;

/// The gap never grows by less than this.
const MIN_GAP: usize = 64 * 1024;
/// The largest gap reserved for a loaded text.
const MAX_INITIAL_GAP: usize = 16 << 20;

/// UTF-8 text with a gap, like Scintilla's `SplitVector`: edits next to the previous one move
/// no data, and the text stays in a single allocation.
pub struct GapBuffer {
    data: Vec<u8>,
    gap_start: usize,
    gap_end: usize,
}

impl GapBuffer {
    /// An empty buffer; memory is allocated on the first edit.
    pub fn new() -> Self {
        Self::from_vec(Vec::new())
    }

    /// A buffer holding `text`. The vector's spare capacity becomes the gap, so a text read into
    /// `Vec::with_capacity(len + GapBuffer::initial_gap(len))` is neither copied nor reallocated.
    pub fn from_vec(mut text: Vec<u8>) -> Self {
        let len = text.len();
        text.resize(text.capacity(), 0);
        let gap_end = text.len();
        GapBuffer { data: text, gap_start: len, gap_end }
    }

    /// The gap to reserve when loading a text of `len` bytes.
    pub fn initial_gap(len: usize) -> usize {
        (len / 64).clamp(MIN_GAP, MAX_INITIAL_GAP)
    }

    /// The parts of `range` before and after the gap; either may be empty.
    ///
    /// # Panics
    ///
    /// If `range` is reversed or out of bounds.
    pub fn as_slices(&self, range: Range<usize>) -> (&[u8], &[u8]) {
        let Range { start, end } = range;
        assert!(start <= end && end <= self.len(), "range {start}..{end} out of 0..{}", self.len());
        let gap = self.gap_len();
        if end <= self.gap_start {
            (&self.data[start..end], &[])
        } else if start >= self.gap_start {
            (&self.data[start + gap..end + gap], &[])
        } else {
            (&self.data[start..self.gap_start], &self.data[self.gap_end..end + gap])
        }
    }

    /// Moves the gap to the end and returns the whole text as one slice.
    pub fn make_contiguous(&mut self) -> &[u8] {
        let len = self.len();
        self.move_gap(len);
        &self.data[..len]
    }

    fn gap_len(&self) -> usize {
        self.gap_end - self.gap_start
    }

    fn move_gap(&mut self, pos: usize) {
        let gap = self.gap_len();
        if pos < self.gap_start {
            self.data.copy_within(pos..self.gap_start, pos + gap);
        } else if pos > self.gap_start {
            self.data.copy_within(self.gap_end..pos + gap, self.gap_start);
        }
        self.gap_start = pos;
        self.gap_end = pos + gap;
    }

    /// Makes the gap at least `need` bytes long; it grows by at least 1/16 of the text.
    fn ensure_gap(&mut self, need: usize) {
        let gap = self.gap_len();
        if gap >= need {
            return;
        }
        let grow = need.max(MIN_GAP).max(self.len() / 16) - gap;
        let old_len = self.data.len();
        self.data.resize(old_len + grow, 0);
        self.data.copy_within(self.gap_end..old_len, self.gap_end + grow);
        self.gap_end += grow;
    }
}

impl Default for GapBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for GapBuffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GapBuffer").field("len", &self.len()).field("gap", &(self.gap_start..self.gap_end)).finish()
    }
}

impl TextStore for GapBuffer {
    type Chunks<'a> = Chunks<'a>;

    #[inline]
    fn len(&self) -> usize {
        self.data.len() - self.gap_len()
    }

    #[inline]
    fn byte(&self, pos: usize) -> u8 {
        if pos < self.gap_start { self.data[pos] } else { self.data[pos + self.gap_len()] }
    }

    fn chunks(&self, range: Range<usize>) -> Chunks<'_> {
        let (first, second) = self.as_slices(range);
        Chunks { first, second }
    }

    fn replace(&mut self, range: Range<usize>, text: &[u8]) {
        let Range { start, end } = range;
        assert!(start <= end && end <= self.len(), "range {start}..{end} out of 0..{}", self.len());
        self.move_gap(start);
        self.gap_end += end - start;
        self.ensure_gap(text.len());
        self.data[self.gap_start..self.gap_start + text.len()].copy_from_slice(text);
        self.gap_start += text.len();
    }
}

/// Iterator over the non-empty parts of a range of a [`GapBuffer`]: at most two, split by the gap.
#[derive(Clone, Debug)]
pub struct Chunks<'a> {
    first: &'a [u8],
    second: &'a [u8],
}

impl<'a> Iterator for Chunks<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<&'a [u8]> {
        if !self.first.is_empty() {
            Some(std::mem::take(&mut self.first))
        } else if !self.second.is_empty() {
            Some(std::mem::take(&mut self.second))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(buf: &GapBuffer) -> Vec<u8> {
        buf.to_vec(0..buf.len())
    }

    #[test]
    fn edits_at_start_middle_and_end() {
        let mut buf = GapBuffer::new();
        assert!(buf.is_empty());
        buf.replace(0..0, b"world");
        buf.replace(0..0, b"hello ");
        buf.replace(11..11, b"!");
        assert_eq!(text(&buf), b"hello world!");
        buf.replace(0..5, b"goodbye");
        buf.replace(8..13, b"moon");
        buf.replace(12..13, b"");
        assert_eq!(text(&buf), b"goodbye moon");
        assert_eq!(buf.len(), 12);
        assert_eq!(buf.byte(8), b'm');
    }

    #[test]
    fn spare_capacity_becomes_the_gap() {
        let mut data = Vec::with_capacity(100 + MIN_GAP);
        data.extend_from_slice(&[b'x'; 100]);
        let mut buf = GapBuffer::from_vec(data);
        let ptr = buf.data.as_ptr();
        assert_eq!(buf.len(), 100);
        assert!(buf.gap_len() >= MIN_GAP);
        buf.replace(100..100, &[b'y'; 1000]);
        assert_eq!(buf.data.as_ptr(), ptr, "the first edits must not reallocate");
        assert_eq!(buf.len(), 1100);
    }

    #[test]
    fn gap_grows_when_insertion_does_not_fit() {
        let mut buf = GapBuffer::from_vec(b"abc".to_vec());
        let big = vec![b'z'; 3 * MIN_GAP];
        buf.replace(1..2, &big);
        assert_eq!(buf.len(), 2 + big.len());
        assert_eq!(buf.byte(0), b'a');
        assert_eq!(buf.byte(buf.len() - 1), b'c');
        assert!(buf.to_vec(1..1 + big.len()).iter().all(|&b| b == b'z'));
    }

    #[test]
    fn chunks_split_at_the_gap() {
        let mut buf = GapBuffer::from_vec(b"0123456789".to_vec());
        buf.replace(5..5, b"-");
        // The gap is now after "01234-".
        let parts: Vec<&[u8]> = buf.chunks(0..11).collect();
        assert_eq!(parts, [&b"01234-"[..], &b"56789"[..]]);
        assert_eq!(buf.chunks(1..4).collect::<Vec<_>>(), [&b"123"[..]]);
        assert_eq!(buf.chunks(7..9).collect::<Vec<_>>(), [&b"67"[..]]);
        assert_eq!(buf.chunks(3..3).count(), 0);
        assert_eq!(buf.as_slices(0..11), (&b"01234-"[..], &b"56789"[..]));
    }

    #[test]
    fn make_contiguous_returns_the_whole_text() {
        let mut buf = GapBuffer::from_vec(b"hello world".to_vec());
        buf.replace(5..5, b",");
        assert_eq!(buf.make_contiguous(), b"hello, world");
        assert_eq!(buf.chunks(0..12).count(), 1);
    }

    #[test]
    fn random_edits_match_a_vec() {
        let mut seed = 99u64;
        let mut rnd = |n: usize| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((seed >> 33) as usize) % n.max(1)
        };
        let mut buf = GapBuffer::new();
        let mut model = Vec::new();
        for round in 0..5000 {
            let len = model.len();
            let start = rnd(len + 1);
            let end = start + rnd((len - start).min(64) + 1);
            let insert: Vec<u8> = (0..rnd(80)).map(|i| b'a' + ((round + i) % 26) as u8).collect();
            buf.replace(start..end, &insert);
            model.splice(start..end, insert);
            assert_eq!(buf.len(), model.len());
            let a = rnd(model.len() + 1);
            let b = a + rnd(model.len() - a + 1);
            assert_eq!(buf.to_vec(a..b), model[a..b]);
        }
        assert_eq!(text(&buf), model);
    }

    #[test]
    #[should_panic(expected = "out of")]
    fn replace_out_of_bounds_panics() {
        GapBuffer::from_vec(b"abc".to_vec()).replace(2..4, b"");
    }

    #[test]
    #[should_panic]
    fn byte_out_of_bounds_panics() {
        GapBuffer::from_vec(b"abc".to_vec()).byte(3);
    }
}
