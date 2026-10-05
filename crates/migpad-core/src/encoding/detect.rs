//! Encoding detection: the byte order mark, then valid UTF-8, then a statistical guess.

use chardetng::{EncodingDetector, Iso2022JpDetection, Utf8Detection};

use super::Encoding;

/// The encoding of a file and whether it starts with a byte order mark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Detected {
    pub encoding: Encoding,
    pub bom: bool,
}

/// KOI8-U letters missing from KOI8-R: є і ї ґ Є І Ї Ґ.
const KOI8_U_UKRAINIAN: [u8; 8] = [0xA4, 0xA6, 0xA7, 0xAD, 0xB4, 0xB6, 0xB7, 0xBD];

/// Detects the encoding of a file from its first bytes.
///
/// `complete` tells whether `sample` is the whole file: a sample cut off in the middle of a UTF-8
/// sequence is still UTF-8. `tld` is the lower-case country-code top-level domain of the region
/// whose legacy encodings are likely, like `"ru"`; it improves the guess, other values are ignored.
pub fn detect(sample: &[u8], complete: bool, tld: Option<&str>) -> Detected {
    for encoding in [Encoding::UTF_8, Encoding::UTF_16LE, Encoding::UTF_16BE] {
        if sample.starts_with(encoding.bom()) {
            return Detected { encoding, bom: true };
        }
    }
    let utf8 = match std::str::from_utf8(sample) {
        Ok(_) => true,
        Err(error) => !complete && error.error_len().is_none(),
    };
    if utf8 {
        return Detected { encoding: Encoding::UTF_8, bom: false };
    }
    let mut detector = EncodingDetector::new(Iso2022JpDetection::Deny);
    detector.feed(sample, complete);
    // `guess` panics on a label with anything but lower-case ASCII letters, digits and hyphens.
    let tld = tld.filter(|tld| {
        !tld.is_empty() && tld.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    });
    let mut guess = detector.guess(tld.map(str::as_bytes), Utf8Detection::Deny);
    // chardetng reports the KOI8 family as KOI8-U; without Ukrainian letters it is KOI8-R.
    if guess == encoding_rs::KOI8_U && !sample.iter().any(|b| KOI8_U_UKRAINIAN.contains(b)) {
        guess = encoding_rs::KOI8_R;
    }
    // chardetng only guesses supported encodings; UTF-8 would keep the bytes as they are anyway.
    let encoding = Encoding::from_encoding_rs(guess).unwrap_or(Encoding::UTF_8);
    Detected { encoding, bom: false }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PANGRAM: &str = "Съешь же ещё этих мягких французских булок, да выпей чаю.\n";

    fn legacy(name: &str) -> Encoding {
        Encoding::for_name(name).unwrap()
    }

    #[test]
    fn byte_order_marks_win() {
        let cases = [
            (&b"\xEF\xBB\xBFtext"[..], Encoding::UTF_8),
            (b"\xFF\xFEt\0", Encoding::UTF_16LE),
            (b"\xFE\xFF\0t", Encoding::UTF_16BE),
            (b"\xEF\xBB\xBF\xFF\xFE", Encoding::UTF_8),
        ];
        for (sample, encoding) in cases {
            assert_eq!(detect(sample, true, None), Detected { encoding, bom: true });
        }
    }

    #[test]
    fn valid_utf8_is_utf8() {
        let utf8 = Detected { encoding: Encoding::UTF_8, bom: false };
        assert_eq!(detect(b"", true, None), utf8);
        assert_eq!(detect(b"plain ASCII", true, None), utf8);
        assert_eq!(detect(PANGRAM.as_bytes(), true, Some("ru")), utf8);
    }

    #[test]
    fn a_sample_may_cut_a_utf8_sequence() {
        let bytes = PANGRAM.as_bytes();
        let cut = &bytes[..bytes.len() - 7]; // inside "ч" of "чаю.\n"
        assert!(std::str::from_utf8(cut).is_err());
        assert_eq!(detect(cut, false, None).encoding, Encoding::UTF_8);
        assert_ne!(detect(cut, true, None).encoding, Encoding::UTF_8, "a whole file must be valid");
    }

    #[test]
    fn guesses_cyrillic_legacy_encodings() {
        let text = PANGRAM.repeat(20);
        for name in ["windows-1251", "KOI8-R", "IBM866"] {
            let encoding = legacy(name);
            let (bytes, _, _) = encoding.0.encode(&text);
            for tld in [Some("ru"), None] {
                assert_eq!(detect(&bytes, true, tld), Detected { encoding, bom: false }, "{name}, {tld:?}");
            }
        }
    }

    #[test]
    fn koi8_u_needs_ukrainian_letters() {
        let text = "Їжак з'їв ґрунт, і все - Євген.\n".repeat(20);
        let (bytes, _, _) = encoding_rs::KOI8_U.encode(&text);
        assert_eq!(detect(&bytes, true, Some("ua")).encoding, legacy("KOI8-U"));
    }

    #[test]
    fn invalid_tld_hints_are_ignored() {
        let text = PANGRAM.repeat(20);
        let (bytes, _, _) = encoding_rs::WINDOWS_1251.encode(&text);
        for tld in ["RU", "co.uk", "рф", ""] {
            assert_eq!(detect(&bytes, true, Some(tld)).encoding, legacy("windows-1251"), "{tld:?}");
        }
    }
}
