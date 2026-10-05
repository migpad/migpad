//! Line endings: LF, CRLF and CR.

use crate::text::EolCounts;

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

#[cfg(test)]
mod tests {
    use super::*;

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
