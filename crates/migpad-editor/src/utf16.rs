//! UTF-16 offsets, which input methods speak, within a part of a line. Characters are those of
//! the core: a valid UTF-8 sequence, or an invalid part that shows as one U+FFFD.

/// The length in bytes and in UTF-16 units of each character of `bytes`.
fn chars(bytes: &[u8]) -> impl Iterator<Item = (usize, usize)> + '_ {
    bytes.utf8_chunks().flat_map(|chunk| {
        let valid = chunk.valid().chars().map(|c| (c.len_utf8(), c.len_utf16()));
        let invalid = (!chunk.invalid().is_empty()).then_some((chunk.invalid().len(), 1));
        valid.chain(invalid)
    })
}

/// The UTF-16 offset of byte `offset` of `bytes`; an offset inside a character counts as its
/// start, one past the end as the end.
pub fn to_utf16(bytes: &[u8], offset: usize) -> usize {
    let (mut at, mut utf16) = (0, 0);
    for (len, len16) in chars(bytes) {
        if at + len > offset {
            break;
        }
        at += len;
        utf16 += len16;
    }
    utf16
}

/// The byte offset in `bytes` of UTF-16 offset `utf16`; inside a character — between the halves
/// of a surrogate pair — its start, past the end the end.
pub fn from_utf16(bytes: &[u8], utf16: usize) -> usize {
    let (mut at, mut units) = (0, 0);
    for (len, len16) in chars(bytes) {
        if units + len16 > utf16 {
            break;
        }
        at += len;
        units += len16;
    }
    at
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_count_utf16_units() {
        // a (1 byte, 1 unit), ж (2, 1), 漢 (3, 1), 😀 (4, 2), b.
        let bytes = "aж漢😀b".as_bytes();
        let pairs = [(0, 0), (1, 1), (3, 2), (6, 3), (10, 5), (11, 6)];
        for (byte, unit) in pairs {
            assert_eq!(to_utf16(bytes, byte), unit, "byte {byte}");
            assert_eq!(from_utf16(bytes, unit), byte, "unit {unit}");
        }
        // Inside a character: its start.
        assert_eq!(to_utf16(bytes, 2), 1);
        assert_eq!(to_utf16(bytes, 8), 3);
        assert_eq!(from_utf16(bytes, 4), 6, "between the halves of a surrogate pair");
        // Past the end: the end.
        assert_eq!((to_utf16(bytes, 99), from_utf16(bytes, 99)), (6, 11));
    }

    #[test]
    fn an_invalid_part_is_one_unit() {
        // E2 82 is one invalid part, FF another.
        let bytes = b"x\xE2\x82\xFFy";
        assert_eq!([0, 1, 3, 4, 5].map(|byte| to_utf16(bytes, byte)), [0, 1, 2, 3, 4]);
        assert_eq!([0, 1, 2, 3, 4].map(|unit| from_utf16(bytes, unit)), [0, 1, 3, 4, 5]);
    }
}
