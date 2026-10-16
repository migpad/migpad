//! Encoding detection: the byte order mark, then UTF-16 without it, then valid or mostly valid
//! UTF-8, then a statistical guess.

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
    // Before UTF-8: text in UTF-16 with Latin or Cyrillic letters only is valid UTF-8 too.
    if let Some(encoding) = utf16_without_bom(sample) {
        return Detected { encoding, bom: false };
    }
    let utf8 = match std::str::from_utf8(sample) {
        Ok(_) => true,
        Err(error) => !complete && error.error_len().is_none(),
    };
    if utf8 || mostly_utf8(sample) {
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

/// UTF-16 told by its high bytes, which text in an alphabet keeps few: zero for spaces, digits,
/// punctuation and Latin letters, `0x04` for Cyrillic ones, `0x03` for Greek ones. Either many of
/// the high bytes are zero, few of the low ones, and few units are two printable ASCII characters;
/// or nearly all high bytes are zero or that of one alphabet. Then the sample, read as UTF-16,
/// must look like text: no unpaired surrogates, no control characters but tabs, line and page
/// breaks. Binary files have plenty of those.
///
/// Text in scripts with thousands of characters, as Chinese, has no such high bytes and is not
/// told from other encodings without others among it.
fn utf16_without_bom(sample: &[u8]) -> Option<Encoding> {
    // High bytes come second in little-endian code units.
    [(Encoding::UTF_16LE, 1), (Encoding::UTF_16BE, 0)]
        .into_iter()
        .find(|&(encoding, high)| has_utf16_high_bytes(sample, high) && reads_as_text(sample, encoding))
        .map(|(encoding, _)| encoding)
}

fn has_utf16_high_bytes(sample: &[u8], high: usize) -> bool {
    let (pairs, _) = sample.as_chunks::<2>();
    let units = pairs.len();
    if units == 0 {
        return false;
    }
    let mut counts = [0usize; 256];
    let (mut low_zeros, mut ascii) = (0, 0);
    for unit in pairs {
        counts[unit[high] as usize] += 1;
        low_zeros += usize::from(unit[1 - high] == 0);
        ascii += usize::from(unit.iter().all(|b| (0x20..0x7F).contains(b)));
    }
    let zeros = counts[0];
    // Latin text and any with spaces. In UTF-8 text with NUL characters, the zeros fall on either
    // byte; in a list of words that NUL characters end and in ASCII text that UTF-16 text follows,
    // as PowerShell appends it, many units are two ASCII characters.
    if zeros * 8 >= units && low_zeros * 8 <= zeros && ascii * 4 <= zeros {
        return true;
    }
    // Alphabets from Greek to Arabic, from Thai to Balinese. Not Latin ones, which have ASCII
    // letters among them, nor the high bytes of tabs and line breaks or of `^A` and other
    // separators: in UTF-8, lines and fields of one character would have them every other byte.
    let alphabet = (0x03..=0x08).chain(0x0E..=0x1B).map(|b| counts[b]).max().unwrap_or(0);
    (zeros + alphabet) * 10 >= units * 9
}

/// Whether `sample` in `encoding`, UTF-16, is text. A sample may cut a character: an odd byte or
/// the first half of a surrogate pair at its end do not count, nor NUL characters that end a file.
fn reads_as_text(sample: &[u8], encoding: Encoding) -> bool {
    let (pairs, _) = sample.as_chunks::<2>();
    let from_bytes = if encoding == Encoding::UTF_16LE { u16::from_le_bytes } else { u16::from_be_bytes };
    let mut units: Vec<u16> = pairs.iter().map(|&unit| from_bytes(unit)).collect();
    if units.last().is_some_and(|unit| (0xD800..0xDC00).contains(unit)) {
        units.pop();
    }
    while units.last() == Some(&0) {
        units.pop();
    }
    !units.is_empty()
        && char::decode_utf16(units).all(|c| {
            c.is_ok_and(|c| {
                !(c.is_control() && !matches!(c, '\t' | '\n' | '\r' | '\x0C')) && !matches!(c, '\u{FFFE}' | '\u{FFFF}')
            })
        })
}

/// Whether `sample` is UTF-8 with a few broken places: its valid non-ASCII characters outnumber
/// its invalid parts four to one. Read as UTF-8, such a file keeps its bytes and shows the broken
/// places as U+FFFD, while a legacy encoding would garble all of its non-ASCII text. Text in
/// legacy encodings seldom forms valid UTF-8 sequences, so it stays far below this.
fn mostly_utf8(sample: &[u8]) -> bool {
    let (mut valid, mut invalid) = (0, 0);
    for chunk in sample.utf8_chunks() {
        valid += chunk.valid().chars().filter(|c| !c.is_ascii()).count();
        invalid += usize::from(!chunk.invalid().is_empty());
    }
    valid >= 4 * invalid.max(1)
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
        // A whole file cut off like this is still mostly UTF-8.
        assert_eq!(detect(cut, true, None).encoding, Encoding::UTF_8);
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
    fn mostly_valid_utf8_is_utf8() {
        let sample = [
            "Привет, мир! Кириллица и ёлка.\n".as_bytes(),
            b"Invalid bytes: [\xFF\xFE] and a cut sequence [\xE2\x82] end\n",
            "日本語のテキスト、カタカナ\n".as_bytes(),
        ]
        .concat();
        assert_eq!(detect(&sample, true, Some("ru")), Detected { encoding: Encoding::UTF_8, bom: false });
    }

    #[test]
    fn legacy_text_is_not_mostly_utf8() {
        let texts = [
            ("windows-1251", PANGRAM),
            ("KOI8-R", PANGRAM),
            ("IBM866", PANGRAM),
            ("Shift_JIS", "日本語のテキストとカタカナ、ひらがなの例文です。\n"),
            ("EUC-JP", "日本語のテキストとカタカナ、ひらがなの例文です。\n"),
            ("GBK", "这是一个中文文本示例，用来测试编码检测。\n"),
            ("Big5", "這是一個中文文本示範，用來測試編碼偵測。\n"),
            ("EUC-KR", "이것은 인코딩 감지를 시험하기 위한 한국어 예문입니다.\n"),
        ];
        for (name, text) in texts {
            let text = text.repeat(20);
            let (bytes, _, unmappable) = legacy(name).0.encode(&text);
            assert!(!unmappable, "{name}");
            assert!(!mostly_utf8(&bytes), "{name}");
            assert_ne!(detect(&bytes, true, None).encoding, Encoding::UTF_8, "{name}");
        }
    }

    #[test]
    fn a_single_latin1_byte_is_not_utf8() {
        assert_eq!(detect(b"caf\xe9 au lait", true, None).encoding, legacy("windows-1252"));
    }

    fn utf16(text: &str, encoding: Encoding) -> Vec<u8> {
        let to_bytes = if encoding == Encoding::UTF_16LE { u16::to_le_bytes } else { u16::to_be_bytes };
        text.encode_utf16().flat_map(to_bytes).collect()
    }

    #[test]
    fn utf16_without_a_byte_order_mark_is_told_by_its_high_bytes() {
        let texts = [
            "Hello, world!\r\nA second line\twith a tab.\r\n",
            PANGRAM,
            "Mixed: Привет, world! 123 — «кавычки» and “quotes”.\n",
            // Cyrillic only: no zero bytes at all.
            "Съешьжеещёэтихмягкихфранцузскихбулок",
            "Καλημέρα κόσμε, ΑΒΓΔ.\n",
            "x",
        ];
        for text in texts {
            for encoding in [Encoding::UTF_16LE, Encoding::UTF_16BE] {
                let bytes = utf16(text, encoding);
                assert_eq!(
                    detect(&bytes, true, Some("ru")),
                    Detected { encoding, bom: false },
                    "{text:?} {encoding:?}"
                );
            }
        }
    }

    #[test]
    fn a_utf16_sample_may_cut_a_character() {
        for encoding in [Encoding::UTF_16LE, Encoding::UTF_16BE] {
            let detected = Detected { encoding, bom: false };
            let bytes = utf16("Some text, a smile \u{1F600}", encoding);
            // An odd byte at the end: a file cut off, or the end of a sample.
            assert_eq!(detect(&bytes[..bytes.len() - 1], true, None), detected, "{encoding:?}");
            // The first half of the surrogate pair of the smile.
            assert_eq!(detect(&bytes[..bytes.len() - 2], false, None), detected, "{encoding:?}");
            // An unpaired surrogate within the text is not text.
            let broken = [&bytes[..bytes.len() - 2], &utf16(" more", encoding)].concat();
            assert_ne!(detect(&broken, true, None), detected, "{encoding:?}");
        }
    }

    #[test]
    fn binary_files_are_not_utf16() {
        let png = [
            &b"\x89PNG\r\n\x1A\n\0\0\0\rIHDR\0\0\0\x10\0\0\0\x10\x08\x06\0\0\0\x1F\xF3\xFFa"[..],
            b"\0\0\0\x0CIDATx\x9Cc\0\x01\0\0\x05\0\x01\r\n-\xB4\0\0\0\0IEND\xAEB`\x82",
        ]
        .concat();
        // This program: an executable of the system the tests run on.
        let mut program = std::fs::read(std::env::current_exe().unwrap()).unwrap();
        program.truncate(256 << 10);
        let numbers: Vec<u8> = (0..2000u16).flat_map(|n| (n % 300).to_le_bytes()).collect();
        for (name, bytes) in [("png", png), ("program", program), ("16-bit numbers", numbers)] {
            let encoding = detect(&bytes, true, None).encoding;
            assert!(![Encoding::UTF_16LE, Encoding::UTF_16BE].contains(&encoding), "{name}: {encoding:?}");
        }
    }

    #[test]
    fn utf8_with_nul_characters_stays_utf8() {
        let utf8 = Detected { encoding: Encoding::UTF_8, bom: false };
        let texts = [
            &b"name\0value\0another name\0another value\0"[..],
            b"ab\0cd\0ef and a longer text after them, with no zero bytes",
            "Кириллица\0и\0нули".as_bytes(),
            // Every other byte the same: a line break, a space, a lead byte of Cyrillic in UTF-8.
            b"1\n2\n3\n4\n5\n6\n7\n8\n9\n",
            b"a a a a a a a a a a",
            "ПАПА МАМА ПАПА".as_bytes(),
        ];
        for text in texts {
            assert_eq!(detect(text, true, Some("ru")), utf8, "{:?}", String::from_utf8_lossy(text));
        }
    }

    #[test]
    fn utf8_followed_by_utf16_stays_utf8() {
        // PowerShell 5.1 appends UTF-16 to a log in another encoding.
        let log = "2026-10-16 21:45:00 INFO service started, listening on port 8080\r\n".repeat(16);
        let appended = utf16(&"2026-10-16 21:46:00 ERROR connection refused\r\n".repeat(4), Encoding::UTF_16LE);
        for odd in ["", "!"] {
            let bytes = [log.as_bytes(), odd.as_bytes(), &appended].concat();
            assert_eq!(detect(&bytes, true, None).encoding, Encoding::UTF_8, "{odd:?}");
        }
    }

    #[test]
    fn separated_fields_are_not_utf16() {
        let texts: [&[u8]; 4] = [
            b"abc\0def\0ghi\0jkl\0mno\0pqr\0",
            // `git log --format=%h -z`.
            b"abc1234\0def5678\0fabcde9\0bcd1234\0",
            // Fields of one character, as Hive separates them.
            b"Y\x01N\x01Y\x01N\x01Y\x01N\x01Y\x01N\x01",
            b"a\x1Fb\x1Fc\x1Fd\x1Fe\x1Ff\x1Fg\x1Fh\x1F",
        ];
        for text in texts {
            let encoding = detect(text, true, None).encoding;
            assert!(![Encoding::UTF_16LE, Encoding::UTF_16BE].contains(&encoding), "{text:?}: {encoding:?}");
        }
    }

    #[test]
    fn utf16_in_other_scripts_and_with_a_nul_at_the_end() {
        let texts = [
            "नमस्ते दुनिया, यह एक परीक्षण है।\n",
            "// 这是一个中文注释\nfn main() {\n    println!(\"你好, {}\", name);\n}\n",
            "A string that a NUL character ends\0\0",
        ];
        for text in texts {
            for encoding in [Encoding::UTF_16LE, Encoding::UTF_16BE] {
                let bytes = utf16(text, encoding);
                assert_eq!(detect(&bytes, true, None), Detected { encoding, bom: false }, "{text:?} {encoding:?}");
            }
        }
        // Not only NUL characters.
        assert_eq!(detect(&[0; 8], true, None).encoding, Encoding::UTF_8);
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
