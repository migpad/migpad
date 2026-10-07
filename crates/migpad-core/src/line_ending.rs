//! Line endings: LF, CRLF and CR.

use crate::text::{EolCounts, TextStore};

/// A kind of line break.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    CrLf,
    Cr,
}

impl LineEnding {
    /// The bytes of the line break.
    pub fn as_bytes(self) -> &'static [u8] {
        match self {
            LineEnding::Lf => b"\n",
            LineEnding::CrLf => b"\r\n",
            LineEnding::Cr => b"\r",
        }
    }

    /// The kind used most in a text — Enter inserts it — or `None` for a text without line breaks.
    /// A tie goes to LF, then to CRLF.
    pub fn dominant(counts: EolCounts) -> Option<LineEnding> {
        let EolCounts { lf, crlf, cr } = counts;
        if lf + crlf + cr == 0 {
            None
        } else if lf >= crlf && lf >= cr {
            Some(LineEnding::Lf)
        } else if crlf >= cr {
            Some(LineEnding::CrLf)
        } else {
            Some(LineEnding::Cr)
        }
    }
}

/// The line breaks of each kind in `text`.
pub fn count<S: TextStore>(text: &S) -> EolCounts {
    let mut counts = EolCounts::default();
    // A CR at the end of a chunk may start a CRLF that the next chunk ends.
    let mut pending_cr = false;
    for chunk in text.chunks(0..text.len()) {
        let mut rest = chunk;
        if pending_cr && !rest.is_empty() {
            pending_cr = false;
            if rest[0] == b'\n' {
                counts.crlf += 1;
                rest = &rest[1..];
            } else {
                counts.cr += 1;
            }
        }
        while let Some(i) = memchr::memchr2(b'\r', b'\n', rest) {
            if rest[i] == b'\n' {
                counts.lf += 1;
            } else if i + 1 == rest.len() {
                pending_cr = true;
            } else if rest[i + 1] == b'\n' {
                counts.crlf += 1;
                rest = &rest[i + 2..];
                continue;
            } else {
                counts.cr += 1;
            }
            rest = &rest[i + 1..];
        }
    }
    if pending_cr {
        counts.cr += 1;
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::GapBuffer;

    #[test]
    fn line_breaks_are_counted_across_chunks() {
        let mut text = GapBuffer::from_vec(b"a\r\nb\nc\rd\r\n".to_vec());
        assert_eq!(count(&text), EolCounts { lf: 1, crlf: 2, cr: 1 });
        // The gap between CR and LF splits the text into two chunks.
        text.replace(2..2, b"");
        text.replace(1..2, b"\r");
        assert_eq!(count(&text), EolCounts { lf: 1, crlf: 2, cr: 1 });
        assert_eq!(count(&GapBuffer::from_vec(b"x\r".to_vec())), EolCounts { lf: 0, crlf: 0, cr: 1 });
        assert_eq!(count(&GapBuffer::new()), EolCounts::default());
    }

    fn dominant(lf: u64, crlf: u64, cr: u64) -> Option<LineEnding> {
        LineEnding::dominant(EolCounts { lf, crlf, cr })
    }

    #[test]
    fn the_most_used_line_ending_wins() {
        assert_eq!(dominant(0, 0, 0), None);
        assert_eq!(dominant(0, 5, 0), Some(LineEnding::CrLf));
        assert_eq!(dominant(1, 5, 2), Some(LineEnding::CrLf));
        assert_eq!(dominant(0, 1, 2), Some(LineEnding::Cr));
        assert_eq!(dominant(3, 3, 3), Some(LineEnding::Lf));
        assert_eq!(dominant(0, 2, 2), Some(LineEnding::CrLf));
    }

    #[test]
    fn line_breaks_as_bytes() {
        assert_eq!(LineEnding::Lf.as_bytes(), b"\n");
        assert_eq!(LineEnding::CrLf.as_bytes(), b"\r\n");
        assert_eq!(LineEnding::Cr.as_bytes(), b"\r");
    }
}
