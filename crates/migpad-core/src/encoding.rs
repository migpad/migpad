//! Character encodings: detection, decoding on load and encoding on save, all of which report
//! what could not be converted, so that nothing is lost silently.
//!
//! The text of a document is UTF-8. A UTF-8 file is loaded as is, invalid bytes included, and
//! saved back unchanged. A file in another encoding is decoded once when it is loaded and encoded
//! again when it is saved.

mod decode;
mod detect;
mod encode;

use std::fmt;

pub use decode::Decoder;
pub use detect::{Detected, detect};
pub use encode::Encoder;

/// A character encoding MigPad can open and save files in.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Encoding(&'static encoding_rs::Encoding);

impl Encoding {
    pub const UTF_8: Encoding = Encoding(&encoding_rs::UTF_8_INIT);
    pub const UTF_16LE: Encoding = Encoding(&encoding_rs::UTF_16LE_INIT);
    pub const UTF_16BE: Encoding = Encoding(&encoding_rs::UTF_16BE_INIT);

    /// All supported encodings: Unicode first, then legacy encodings grouped by script.
    /// These are the encodings of the Encoding Standard, except `replacement` and `x-user-defined`.
    pub const ALL: [Encoding; 38] = [
        Self::UTF_8,
        Self::UTF_16LE,
        Self::UTF_16BE,
        // Cyrillic
        Encoding(&encoding_rs::WINDOWS_1251_INIT),
        Encoding(&encoding_rs::KOI8_R_INIT),
        Encoding(&encoding_rs::KOI8_U_INIT),
        Encoding(&encoding_rs::IBM866_INIT),
        Encoding(&encoding_rs::ISO_8859_5_INIT),
        Encoding(&encoding_rs::X_MAC_CYRILLIC_INIT),
        // Western European
        Encoding(&encoding_rs::WINDOWS_1252_INIT),
        Encoding(&encoding_rs::ISO_8859_15_INIT),
        Encoding(&encoding_rs::MACINTOSH_INIT),
        // Central and Eastern European
        Encoding(&encoding_rs::WINDOWS_1250_INIT),
        Encoding(&encoding_rs::ISO_8859_2_INIT),
        Encoding(&encoding_rs::ISO_8859_16_INIT),
        // Baltic
        Encoding(&encoding_rs::WINDOWS_1257_INIT),
        Encoding(&encoding_rs::ISO_8859_4_INIT),
        Encoding(&encoding_rs::ISO_8859_13_INIT),
        // South European, Nordic, Celtic
        Encoding(&encoding_rs::ISO_8859_3_INIT),
        Encoding(&encoding_rs::ISO_8859_10_INIT),
        Encoding(&encoding_rs::ISO_8859_14_INIT),
        // Greek
        Encoding(&encoding_rs::WINDOWS_1253_INIT),
        Encoding(&encoding_rs::ISO_8859_7_INIT),
        // Turkish
        Encoding(&encoding_rs::WINDOWS_1254_INIT),
        // Hebrew
        Encoding(&encoding_rs::WINDOWS_1255_INIT),
        Encoding(&encoding_rs::ISO_8859_8_INIT),
        Encoding(&encoding_rs::ISO_8859_8_I_INIT),
        // Arabic
        Encoding(&encoding_rs::WINDOWS_1256_INIT),
        Encoding(&encoding_rs::ISO_8859_6_INIT),
        // Vietnamese
        Encoding(&encoding_rs::WINDOWS_1258_INIT),
        // Thai
        Encoding(&encoding_rs::WINDOWS_874_INIT),
        // Chinese
        Encoding(&encoding_rs::GBK_INIT),
        Encoding(&encoding_rs::GB18030_INIT),
        Encoding(&encoding_rs::BIG5_INIT),
        // Japanese
        Encoding(&encoding_rs::SHIFT_JIS_INIT),
        Encoding(&encoding_rs::EUC_JP_INIT),
        Encoding(&encoding_rs::ISO_2022_JP_INIT),
        // Korean
        Encoding(&encoding_rs::EUC_KR_INIT),
    ];

    /// The name from the Encoding Standard, like `windows-1251`; settings and the list of recent
    /// files store encodings by it.
    pub fn name(self) -> &'static str {
        self.0.name()
    }

    /// The supported encoding with this name, in any letter case.
    pub fn for_name(name: &str) -> Option<Encoding> {
        Self::ALL.into_iter().find(|encoding| encoding.name().eq_ignore_ascii_case(name))
    }

    pub fn is_utf8(self) -> bool {
        self == Self::UTF_8
    }

    pub fn is_utf16(self) -> bool {
        self == Self::UTF_16LE || self == Self::UTF_16BE
    }

    /// The byte order mark; empty for encodings that have none.
    pub fn bom(self) -> &'static [u8] {
        if self == Self::UTF_8 {
            b"\xEF\xBB\xBF"
        } else if self == Self::UTF_16LE {
            b"\xFF\xFE"
        } else if self == Self::UTF_16BE {
            b"\xFE\xFF"
        } else {
            b""
        }
    }

    fn from_encoding_rs(encoding: &'static encoding_rs::Encoding) -> Option<Encoding> {
        Self::ALL.into_iter().find(|supported| supported.0 == encoding)
    }
}

impl fmt::Debug for Encoding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Characters lost in a conversion: malformed input, or characters the target encoding cannot
/// represent. Each became one replacement character.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Losses {
    pub count: usize,
    /// Byte offset of the first one in the UTF-8 text.
    pub first: Option<usize>,
}

impl Losses {
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    fn record(&mut self, pos: usize) {
        self.count += 1;
        self.first.get_or_insert(pos);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn all_encodings_are_distinct_and_known_by_name() {
        let names: HashSet<&str> = Encoding::ALL.iter().map(|e| e.name()).collect();
        assert_eq!(names.len(), 38);
        for encoding in Encoding::ALL {
            assert_eq!(Encoding::for_name(encoding.name()), Some(encoding));
            assert_eq!(Encoding::for_name(&encoding.name().to_ascii_uppercase()), Some(encoding));
        }
        assert_eq!(Encoding::for_name("replacement"), None);
        assert_eq!(Encoding::for_name("x-user-defined"), None);
        assert_eq!(Encoding::for_name("cp1251"), None, "labels are not names");
    }

    #[test]
    fn only_unicode_encodings_have_a_bom() {
        for encoding in Encoding::ALL {
            assert_eq!(!encoding.bom().is_empty(), encoding.is_utf8() || encoding.is_utf16(), "{encoding:?}");
        }
    }
}
