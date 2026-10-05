//! Text storage: the [`TextStore`] trait, the gap buffer that implements it, and the line index.
//!
//! Text is kept as the UTF-8 bytes it was loaded as: invalid sequences stay untouched, and every
//! position is a byte offset.

mod gap_buffer;
mod line_index;

use std::ops::Range;

pub use gap_buffer::{Chunks, GapBuffer};
pub use line_index::{EolCounts, Indexed, Indexer, LineIndex};

/// The longest supported text in bytes: line starts are stored as `u32`.
pub const MAX_LEN: usize = u32::MAX as usize;

/// Storage of a document's text.
///
/// [`GapBuffer`] keeps the whole text in memory; a store that reads large files in parts
/// can implement the same trait later.
pub trait TextStore {
    /// Iterator over the parts of a range, see [`TextStore::chunks`].
    type Chunks<'a>: Iterator<Item = &'a [u8]>
    where
        Self: 'a;

    /// Length of the text in bytes.
    fn len(&self) -> usize;

    /// Whether the text is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The byte at `pos`.
    ///
    /// # Panics
    ///
    /// If `pos` is out of bounds.
    fn byte(&self, pos: usize) -> u8;

    /// The bytes of `range` as consecutive non-empty slices, without copying.
    ///
    /// # Panics
    ///
    /// If `range` is reversed or out of bounds.
    fn chunks(&self, range: Range<usize>) -> Self::Chunks<'_>;

    /// Replaces the bytes of `range` with `text`.
    ///
    /// # Panics
    ///
    /// If `range` is reversed or out of bounds.
    fn replace(&mut self, range: Range<usize>, text: &[u8]);

    /// Appends the bytes of `range` to `out`.
    fn copy_to(&self, range: Range<usize>, out: &mut Vec<u8>) {
        for chunk in self.chunks(range) {
            out.extend_from_slice(chunk);
        }
    }

    /// The bytes of `range` as a new vector.
    fn to_vec(&self, range: Range<usize>) -> Vec<u8> {
        let mut out = Vec::with_capacity(range.len());
        self.copy_to(range, &mut out);
        out
    }

    /// The character boundary after `pos`, but not after `ceil`.
    ///
    /// A character is a valid UTF-8 sequence or an invalid part shown as one U+FFFD,
    /// the same split as [`String::from_utf8_lossy`] makes.
    fn next_char_boundary(&self, pos: usize, ceil: usize) -> usize {
        if pos >= ceil { pos } else { (pos + char_len(self, pos)).min(ceil) }
    }

    /// The character boundary before `pos`, but not before `floor`.
    ///
    /// Characters are split as in [`TextStore::next_char_boundary`].
    fn prev_char_boundary(&self, pos: usize, floor: usize) -> usize {
        if pos <= floor {
            return pos;
        }
        // A character ending at `pos` starts with a non-continuation byte at most four bytes back;
        // if there is none, or its character ends earlier, the byte before `pos` stands alone.
        let start = (pos.saturating_sub(4)..pos).rev().find(|&p| !is_continuation(self.byte(p)));
        let boundary = match start {
            Some(s) if s + char_len(self, s) == pos => s,
            _ => pos - 1,
        };
        boundary.max(floor)
    }

    /// The start of the character that byte `pos` belongs to: `pos` itself if a character starts
    /// there. Unlike [`TextStore::prev_char_boundary`], `pos` may be any offset up to the length of
    /// the text; `floor` is a character boundary at or before it, such as the start of its line,
    /// and the search does not look before it.
    ///
    /// Characters are split as in [`TextStore::next_char_boundary`].
    fn floor_char_boundary(&self, pos: usize, floor: usize) -> usize {
        if pos >= self.len() {
            return pos;
        }
        // The character that contains `pos` starts with a non-continuation byte at most three bytes
        // back; if that character ends before `pos`, the byte at `pos` stands alone.
        let start = (pos.saturating_sub(3).max(floor)..=pos).rev().find(|&p| !is_continuation(self.byte(p)));
        match start {
            Some(s) if s + char_len(self, s) > pos => s,
            _ => pos,
        }
    }
}

/// Length of the character at `pos`: a valid UTF-8 sequence, or the maximal invalid part that
/// [`String::from_utf8_lossy`] replaces with one U+FFFD.
fn char_len<S: TextStore + ?Sized>(text: &S, pos: usize) -> usize {
    // The sequence length a lead byte announces, and the valid range of the second byte
    // (narrower after E0, ED, F0 and F4: no overlong forms, surrogates or values above U+10FFFF).
    let (len, second) = match text.byte(pos) {
        0x00..=0x7F => return 1,
        0xC2..=0xDF => (2, 0x80..=0xBF),
        0xE0 => (3, 0xA0..=0xBF),
        0xE1..=0xEC | 0xEE..=0xEF => (3, 0x80..=0xBF),
        0xED => (3, 0x80..=0x9F),
        0xF0 => (4, 0x90..=0xBF),
        0xF1..=0xF3 => (4, 0x80..=0xBF),
        0xF4 => (4, 0x80..=0x8F),
        _ => return 1,
    };
    let mut n = 1;
    while n < len && pos + n < text.len() {
        let byte = text.byte(pos + n);
        let valid = if n == 1 { second.contains(&byte) } else { is_continuation(byte) };
        if !valid {
            break;
        }
        n += 1;
    }
    n
}

/// Whether `byte` continues a UTF-8 sequence rather than starting one.
pub(crate) fn is_continuation(byte: u8) -> bool {
    byte & 0xC0 == 0x80
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(text: &[u8]) -> GapBuffer {
        GapBuffer::from_vec(text.to_vec())
    }

    fn forward(text: &GapBuffer) -> Vec<usize> {
        let mut stops = vec![0];
        while *stops.last().unwrap() < text.len() {
            stops.push(text.next_char_boundary(*stops.last().unwrap(), text.len()));
        }
        stops
    }

    fn backward(text: &GapBuffer) -> Vec<usize> {
        let mut stops = vec![text.len()];
        while *stops.last().unwrap() > 0 {
            stops.push(text.prev_char_boundary(*stops.last().unwrap(), 0));
        }
        stops.reverse();
        stops
    }

    /// Character boundaries as `from_utf8_lossy` sees them: one per valid character
    /// and one per invalid part.
    fn lossy_boundaries(bytes: &[u8]) -> Vec<usize> {
        let mut stops = vec![0];
        let mut pos = 0;
        for chunk in bytes.utf8_chunks() {
            for c in chunk.valid().chars() {
                pos += c.len_utf8();
                stops.push(pos);
            }
            if !chunk.invalid().is_empty() {
                pos += chunk.invalid().len();
                stops.push(pos);
            }
        }
        stops
    }

    #[test]
    fn char_boundaries_of_valid_utf8() {
        let text = store("aé€😀b".as_bytes());
        let want = vec![0, 1, 3, 6, 10, 11];
        assert_eq!(forward(&text), want);
        assert_eq!(backward(&text), want);
    }

    #[test]
    fn char_boundaries_split_invalid_utf8_like_lossy_decoding() {
        // A lone lead byte, a truncated sequence, stray continuation bytes, a surrogate
        // and an overlong form.
        let bytes = b"\xC3a\xE2\x82b\x80\x80\x80\x80c\xED\xA0\x80d\xF0\x80\x80e";
        let text = store(bytes);
        let want = lossy_boundaries(bytes);
        assert_eq!(want, vec![0, 1, 2, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18]);
        assert_eq!(forward(&text), want);
        assert_eq!(backward(&text), want);
    }

    #[test]
    fn char_boundaries_match_lossy_decoding_on_random_bytes() {
        let mut seed = 7u64;
        let mut rnd = |n: usize| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((seed >> 33) as usize) % n
        };
        // Bytes that make interesting sequences: ASCII, leads with special second-byte
        // ranges, ordinary leads, continuation bytes of every range, never-valid bytes.
        let alphabet = [b'a', 0xC3, 0xE0, 0xE2, 0xED, 0xF0, 0xF1, 0xF4, 0x80, 0x8F, 0x90, 0x9F, 0xA0, 0xBF, 0xC0, 0xF5];
        for _ in 0..2000 {
            let bytes: Vec<u8> = (0..rnd(12)).map(|_| alphabet[rnd(alphabet.len())]).collect();
            let text = store(&bytes);
            let want = lossy_boundaries(&bytes);
            assert_eq!(forward(&text), want, "{bytes:02X?}");
            assert_eq!(backward(&text), want, "{bytes:02X?}");
            for pos in 0..=bytes.len() {
                let floor = *want.iter().rfind(|&&b| b <= pos).unwrap();
                assert_eq!(text.floor_char_boundary(pos, 0), floor, "{bytes:02X?} at {pos}");
            }
        }
    }

    #[test]
    fn floor_char_boundary_finds_the_start_of_the_character() {
        let text = store("a€b".as_bytes());
        assert_eq!([0, 1, 2, 3, 4, 5].map(|pos| text.floor_char_boundary(pos, 0)), [0, 1, 1, 1, 4, 5]);
        assert_eq!(text.floor_char_boundary(3, 1), 1);
        // An invalid part is one character.
        let text = store(b"x\xE2\x82y");
        assert_eq!(text.floor_char_boundary(2, 0), 1);
    }

    #[test]
    fn char_boundaries_stop_at_limits() {
        let text = store("€€".as_bytes());
        assert_eq!(text.next_char_boundary(0, 2), 2);
        assert_eq!(text.prev_char_boundary(6, 4), 4);
        assert_eq!(text.next_char_boundary(6, 6), 6);
        assert_eq!(text.prev_char_boundary(0, 0), 0);
    }
}
