//! Encoding of the text for saving.

use std::ops::Range;

use encoding_rs::EncoderResult;

use super::{Encoding, Losses};
use crate::text::is_continuation;

/// Encodes UTF-8 text for saving, chunk by chunk; chunks may split characters.
///
/// UTF-8 text is written as it is, invalid bytes included. In other encodings, characters that
/// would not read back the same — absent from the encoding, or mapped to another character — and
/// invalid UTF-8 are written as `?` (U+FFFD in UTF-16) and counted in [`Encoder::losses`].
pub struct Encoder {
    encoding: Encoding,
    /// The encoder of a legacy encoding; UTF-8 and UTF-16 are written directly.
    legacy: Option<encoding_rs::Encoder>,
    /// Characters the encoding writes with the codes of others, see [`substitutions`].
    substituted: Option<fn(char) -> bool>,
    /// The byte order mark is still to be written.
    bom: bool,
    /// The beginning of a character cut off by the end of the previous chunk.
    pending: Vec<u8>,
    /// Bytes of text encoded so far; pending bytes do not count yet.
    pos: usize,
    lost: Lost,
}

/// What an encoder has lost so far.
#[derive(Default)]
struct Lost {
    losses: Losses,
    /// Where each loss is in the text, if asked for: see [`Encoder::recording_places`].
    places: Option<Vec<Range<usize>>>,
}

impl Lost {
    fn record(&mut self, place: Range<usize>) {
        self.losses.record(place.start);
        if let Some(places) = &mut self.places {
            places.push(place);
        }
    }
}

impl Encoder {
    /// An encoder for `encoding`; with `bom`, the output starts with the byte order mark.
    pub fn new(encoding: Encoding, bom: bool) -> Self {
        let legacy = (!encoding.is_utf8() && !encoding.is_utf16()).then(|| encoding.0.new_encoder());
        Encoder {
            encoding,
            legacy,
            substituted: substitutions(encoding),
            bom,
            pending: Vec::with_capacity(4),
            pos: 0,
            lost: Lost::default(),
        }
    }

    /// Records where each loss is, not only the first: see [`Encoder::lost_places`].
    pub fn recording_places(self) -> Self {
        Encoder { lost: Lost { places: Some(Vec::new()), ..self.lost }, ..self }
    }

    /// Encodes the next chunk of the text and appends the result to `out`; `last` marks the final
    /// chunk. Call it with `last` even for an empty text: that writes the byte order mark and
    /// finishes stateful encodings.
    pub fn encode(&mut self, mut text: &[u8], last: bool, out: &mut Vec<u8>) {
        if std::mem::take(&mut self.bom) {
            out.extend_from_slice(self.encoding.bom());
        }
        if self.encoding.is_utf8() {
            out.extend_from_slice(text);
            self.pos += text.len();
            return;
        }
        if !self.pending.is_empty() {
            // Complete the character cut off by the previous chunk with its continuation bytes.
            let need = sequence_len(self.pending[0]) - self.pending.len();
            let take = text.iter().take(need).take_while(|&&b| is_continuation(b)).count();
            self.pending.extend_from_slice(&text[..take]);
            text = &text[take..];
            if take < need && text.is_empty() && !last {
                return;
            }
            let pending = std::mem::take(&mut self.pending);
            self.encode_whole(&pending, out);
            self.pending = pending;
            self.pending.clear();
        }
        let keep = if last { 0 } else { incomplete_tail(text) };
        let (whole, tail) = text.split_at(text.len() - keep);
        self.encode_whole(whole, out);
        self.pending.extend_from_slice(tail);
        if last {
            self.finish(out);
        }
    }

    /// Characters lost so far; positions are offsets in the text.
    pub fn losses(&self) -> Losses {
        self.lost.losses
    }

    /// The text of each loss so far, in order: a character, or an invalid sequence. Empty unless
    /// the encoder was made [`recording_places`](Encoder::recording_places).
    pub fn lost_places(&self) -> &[Range<usize>] {
        self.lost.places.as_deref().unwrap_or_default()
    }

    /// Encodes `bytes` that end at a character boundary.
    fn encode_whole(&mut self, bytes: &[u8], out: &mut Vec<u8>) {
        for chunk in bytes.utf8_chunks() {
            self.encode_str(chunk.valid(), out);
            if !chunk.invalid().is_empty() {
                self.lose(chunk.invalid().len(), out);
            }
        }
    }

    fn encode_str(&mut self, mut text: &str, out: &mut Vec<u8>) {
        if self.legacy.is_none() {
            let little_endian = self.encoding == Encoding::UTF_16LE;
            out.reserve(text.len() * 2);
            for unit in text.encode_utf16() {
                out.extend_from_slice(&if little_endian { unit.to_le_bytes() } else { unit.to_be_bytes() });
            }
            self.pos += text.len();
            return;
        }
        // Characters that would read back as others are lost like unmappable ones.
        if let Some(substituted) = self.substituted {
            while let Some((at, c)) = text.char_indices().find(|&(_, c)| substituted(c)) {
                self.encode_legacy(&text[..at], out);
                self.lose(c.len_utf8(), out);
                text = &text[at + c.len_utf8()..];
            }
        }
        self.encode_legacy(text, out);
    }

    fn encode_legacy(&mut self, mut text: &str, out: &mut Vec<u8>) {
        let Some(encoder) = &mut self.legacy else { unreachable!("UTF-8 and UTF-16 are written directly") };
        loop {
            let max = encoder.max_buffer_length_from_utf8_without_replacement(text.len()).expect("chunk too large");
            out.reserve(max);
            let (result, read) = encoder.encode_from_utf8_to_vec_without_replacement(text, out, false);
            text = &text[read..];
            self.pos += read;
            match result {
                EncoderResult::InputEmpty => return,
                EncoderResult::OutputFull => {}
                EncoderResult::Unmappable(c) => {
                    write_question_mark(encoder, out);
                    self.lost.record(self.pos - c.len_utf8()..self.pos);
                }
            }
        }
    }

    /// Writes the replacement for the `len` bytes at the current position, which are lost, and
    /// goes past them.
    fn lose(&mut self, len: usize, out: &mut Vec<u8>) {
        match &mut self.legacy {
            Some(encoder) => write_question_mark(encoder, out),
            None if self.encoding == Encoding::UTF_16LE => out.extend_from_slice(&0xFFFDu16.to_le_bytes()),
            None => out.extend_from_slice(&0xFFFDu16.to_be_bytes()),
        }
        self.lost.record(self.pos..self.pos + len);
        self.pos += len;
    }

    fn finish(&mut self, out: &mut Vec<u8>) {
        if let Some(encoder) = &mut self.legacy {
            out.reserve(encoder.max_buffer_length_from_utf8_without_replacement(0).expect("no input"));
            let (result, _) = encoder.encode_from_utf8_to_vec_without_replacement("", out, true);
            debug_assert!(matches!(result, EncoderResult::InputEmpty));
        }
    }
}

/// Writes `?` through the encoder: a stateful encoding has to switch back to ASCII first.
fn write_question_mark(encoder: &mut encoding_rs::Encoder, out: &mut Vec<u8>) {
    out.reserve(encoder.max_buffer_length_from_utf8_without_replacement(1).expect("one byte"));
    let (result, _) = encoder.encode_from_utf8_to_vec_without_replacement("?", out, false);
    debug_assert!(matches!(result, EncoderResult::InputEmpty));
}

/// The characters that `encoding` writes with the codes of other characters, so they would read
/// back as those. The Encoding Standard does so with ¥ and ‾ (as `\` and `~`) in Shift_JIS and
/// EUC-JP, with − (as －) in the three Japanese encodings, with half-width katakana (as full-width) in
/// ISO-2022-JP, and with 18 private-use characters that GB18030-2022 replaced with standard ones.
fn substitutions(encoding: Encoding) -> Option<fn(char) -> bool> {
    match encoding.name() {
        "Shift_JIS" | "EUC-JP" => Some(|c| matches!(c, '\u{A5}' | '\u{203E}' | '\u{2212}')),
        "ISO-2022-JP" => Some(|c| matches!(c, '\u{2212}' | '\u{FF61}'..='\u{FF9F}')),
        "GBK" | "gb18030" => Some(|c| matches!(c, '\u{E78D}'..='\u{E796}') || GB18030_REPLACED.contains(&c)),
        _ => None,
    }
}

/// Private-use characters that GB18030-2022 replaced, besides U+E78D–U+E796.
const GB18030_REPLACED: [char; 8] =
    ['\u{E81E}', '\u{E826}', '\u{E82B}', '\u{E82C}', '\u{E832}', '\u{E843}', '\u{E854}', '\u{E864}'];

/// The length of the UTF-8 sequence that `lead` starts.
fn sequence_len(lead: u8) -> usize {
    match lead {
        0xC2..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF4 => 4,
        _ => 1,
    }
}

/// The length of an unfinished but so far valid UTF-8 sequence at the end of `text`.
fn incomplete_tail(text: &[u8]) -> usize {
    for start in (text.len().saturating_sub(3)..text.len()).rev() {
        if !is_continuation(text[start]) {
            return match std::str::from_utf8(&text[start..]) {
                Err(error) if error.valid_up_to() == 0 && error.error_len().is_none() => text.len() - start,
                _ => 0,
            };
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encoding::Decoder;

    /// Characters of many scripts, symbols that some encodings lack or map to other characters,
    /// and an emoji, which only Unicode encodings have.
    const SAMPLE: &str = "ASCII ~\\ Latin é ß ő Č ł Cyrillic Жж ё є ї ґ Greek Ωω Hebrew א Arabic ع Thai ก \
                          Vietnamese ạ ừ CJK 中文 日本語 ｶﾀｶﾅ カナ 한국어 symbols € ™ — … ¥ ‾ − � private \u{E78D} emoji 😀";

    fn encode(encoding: Encoding, bom: bool, pieces: &[&[u8]]) -> (Vec<u8>, Losses) {
        let mut encoder = Encoder::new(encoding, bom);
        let mut out = Vec::new();
        for (i, piece) in pieces.iter().enumerate() {
            encoder.encode(piece, i + 1 == pieces.len(), &mut out);
        }
        (out, encoder.losses())
    }

    fn decode(encoding: Encoding, bytes: &[u8]) -> String {
        if encoding.is_utf8() {
            return String::from_utf8(bytes.to_vec()).unwrap();
        }
        let mut decoder = Decoder::new(encoding);
        let mut text = String::new();
        decoder.decode(bytes, true, &mut text);
        assert!(decoder.losses().is_empty(), "{encoding:?}");
        text
    }

    /// Whether `c` survives encoding and decoding in `encoding`, by encoding_rs alone.
    fn survives(encoding: Encoding, c: char) -> bool {
        if encoding.is_utf8() || encoding.is_utf16() {
            return true;
        }
        let text = c.to_string();
        let (bytes, _, unmappable) = encoding.0.encode(&text);
        !unmappable && encoding.0.decode_without_bom_handling(&bytes).0 == text
    }

    #[test]
    fn every_encoding_reads_back_what_it_does_not_report() {
        for encoding in Encoding::ALL {
            let expected: String = SAMPLE.chars().map(|c| if survives(encoding, c) { c } else { '?' }).collect();
            let lost: Vec<usize> =
                SAMPLE.char_indices().filter(|&(_, c)| !survives(encoding, c)).map(|(i, _)| i).collect();
            let (bytes, losses) = encode(encoding, false, &[SAMPLE.as_bytes()]);
            assert_eq!(losses, Losses { count: lost.len(), first: lost.first().copied() }, "{encoding:?}");
            assert_eq!(decode(encoding, &bytes), expected, "{encoding:?}");
        }
    }

    /// Checks every character in every legacy encoding, so it is slow; run it after updating
    /// encoding_rs: `cargo test -p migpad-core --release -- --ignored`.
    #[test]
    #[ignore]
    fn substitutions_are_complete() {
        for encoding in Encoding::ALL.into_iter().filter(|e| !e.is_utf8() && !e.is_utf16()) {
            for c in (0..=0x10FFFF).filter_map(char::from_u32) {
                let text = c.to_string();
                let (bytes, _, unmappable) = encoding.0.encode(&text);
                if !unmappable {
                    let same = encoding.0.decode_without_bom_handling(&bytes).0 == text;
                    let substituted = substitutions(encoding).is_some_and(|substituted| substituted(c));
                    assert_eq!(substituted, !same, "{encoding:?}, U+{:04X}", c as u32);
                }
            }
        }
    }

    #[test]
    fn chunks_may_split_characters() {
        let mut text = SAMPLE.as_bytes().to_vec();
        text.extend_from_slice(b" bad: \xE2\x82 \xF0\x9F \xC3");
        for name in ["UTF-8", "UTF-16LE", "windows-1251", "Shift_JIS", "ISO-2022-JP", "gb18030"] {
            let encoding = Encoding::for_name(name).unwrap();
            let whole = encode(encoding, true, &[&text]);
            for split in 0..=text.len() {
                let (a, b) = text.split_at(split);
                assert_eq!(encode(encoding, true, &[a, b]), whole, "{name}, split {split}");
                assert_eq!(encode(encoding, true, &[a, b"", b]), whole, "{name}, split {split} with an empty chunk");
            }
            // One byte at a time.
            let bytes: Vec<&[u8]> = text.chunks(1).collect();
            assert_eq!(encode(encoding, true, &bytes), whole, "{name}, byte by byte");
        }
    }

    #[test]
    fn invalid_utf8_is_lost_except_in_utf8() {
        let text = b"a\xFFb\xE2\x82c\xC3";
        assert_eq!(encode(Encoding::UTF_8, false, &[text]), (text.to_vec(), Losses::default()));
        let three = Losses { count: 3, first: Some(1) };
        let windows_1251 = Encoding::for_name("windows-1251").unwrap();
        assert_eq!(encode(windows_1251, false, &[text]), (b"a?b?c?".to_vec(), three));
        let (utf16, losses) = encode(Encoding::UTF_16BE, false, &[text]);
        assert_eq!(utf16, b"\0a\xFF\xFD\0b\xFF\xFD\0c\xFF\xFD");
        assert_eq!(losses, three);
    }

    #[test]
    fn the_places_of_losses_are_recorded_if_asked() {
        // An emoji, invalid UTF-8, then ¥, which Shift_JIS writes as a backslash.
        let text = [b"a".as_slice(), "😀".as_bytes(), b"b\xFF", "¥".as_bytes(), b"c"].concat();
        let places = |name: &str, record: bool| {
            let encoding = Encoding::for_name(name).unwrap();
            let mut encoder = Encoder::new(encoding, false);
            if record {
                encoder = encoder.recording_places();
            }
            let mut out = Vec::new();
            // A byte at a time: places hold across chunks that split characters.
            for (i, byte) in text.iter().enumerate() {
                encoder.encode(&[*byte], i + 1 == text.len(), &mut out);
            }
            encoder.lost_places().to_vec()
        };
        assert_eq!(places("windows-1251", true), [1..5, 6..7, 7..9]);
        assert_eq!(places("Shift_JIS", true), [1..5, 6..7, 7..9]);
        assert_eq!(places("UTF-16LE", true), vec![6..7]);
        assert!(places("UTF-8", true).is_empty());
        assert!(places("windows-1251", false).is_empty());
    }

    #[test]
    fn writes_the_byte_order_mark() {
        assert_eq!(encode(Encoding::UTF_8, true, &[b""]).0, b"\xEF\xBB\xBF");
        assert_eq!(encode(Encoding::UTF_16LE, true, &[b"a"]).0, b"\xFF\xFEa\0");
        assert_eq!(encode(Encoding::UTF_16BE, true, &[b"a"]).0, b"\xFE\xFF\0a");
        assert_eq!(encode(Encoding::UTF_16BE, false, &[b"a"]).0, b"\0a");
    }

    #[test]
    fn stateful_encodings_end_in_ascii() {
        let iso_2022_jp = Encoding::for_name("ISO-2022-JP").unwrap();
        let (bytes, losses) = encode(iso_2022_jp, false, &[&b"\xE6\x97\xA5"[..], &b"\xE6\x9C\xAC\xF0\x9F\x98\x80"[..]]);
        assert_eq!(losses, Losses { count: 1, first: Some(6) });
        assert!(bytes.ends_with(b"?"), "{bytes:02X?}");
        assert_eq!(decode(iso_2022_jp, &bytes), "日本?");
    }
}
