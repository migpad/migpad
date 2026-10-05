//! Decoding of files that are not in UTF-8.

use encoding_rs::DecoderResult;

use super::{Encoding, Losses};

/// Decodes a file in an encoding other than UTF-8 into UTF-8 text, chunk by chunk.
///
/// Each malformed sequence becomes one U+FFFD and is counted in [`Decoder::losses`].
pub struct Decoder {
    inner: encoding_rs::Decoder,
    /// Bytes of text produced so far.
    written: usize,
    losses: Losses,
}

impl Decoder {
    /// A decoder for `encoding`. The caller skips the byte order mark, if there is one.
    ///
    /// # Panics
    ///
    /// If `encoding` is UTF-8: UTF-8 files are loaded as they are, invalid bytes included.
    pub fn new(encoding: Encoding) -> Self {
        assert!(!encoding.is_utf8(), "UTF-8 files are not decoded");
        Decoder { inner: encoding.0.new_decoder_without_bom_handling(), written: 0, losses: Losses::default() }
    }

    /// Decodes the next chunk of the file and appends the text to `out`; `last` marks the final
    /// chunk. Chunks may split sequences.
    pub fn decode(&mut self, mut input: &[u8], last: bool, out: &mut String) {
        loop {
            let max = self.inner.max_utf8_buffer_length_without_replacement(input.len()).expect("chunk too large");
            out.reserve(max);
            let before = out.len();
            let (result, read) = self.inner.decode_to_string_without_replacement(input, out, last);
            self.written += out.len() - before;
            input = &input[read..];
            match result {
                DecoderResult::InputEmpty => return,
                DecoderResult::OutputFull => {}
                DecoderResult::Malformed(..) => {
                    self.losses.record(self.written);
                    out.push(char::REPLACEMENT_CHARACTER);
                    self.written += char::REPLACEMENT_CHARACTER.len_utf8();
                }
            }
        }
    }

    /// Malformed sequences met so far; positions are offsets in the decoded text.
    pub fn losses(&self) -> Losses {
        self.losses
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(name: &str, input: &[u8]) -> (String, Losses) {
        let mut decoder = Decoder::new(Encoding::for_name(name).unwrap());
        let mut out = String::new();
        decoder.decode(input, true, &mut out);
        (out, decoder.losses())
    }

    #[test]
    fn decodes_legacy_and_utf16_text() {
        assert_eq!(decode("windows-1251", b"\xcf\xf0\xe8\xe2\xe5\xf2"), ("Привет".into(), Losses::default()));
        assert_eq!(decode("UTF-16LE", b"h\0i\0"), ("hi".into(), Losses::default()));
        assert_eq!(decode("UTF-16BE", b"\0h\0i"), ("hi".into(), Losses::default()));
    }

    #[test]
    fn malformed_input_is_replaced_and_counted() {
        // An unpaired surrogate, then a cut-off final unit.
        let (text, losses) = decode("UTF-16LE", b"a\0\x00\xD8b\0c");
        assert_eq!(text, "a\u{FFFD}b\u{FFFD}");
        assert_eq!(losses, Losses { count: 2, first: Some(1) });
        // Shift_JIS: a lead byte followed by a byte that cannot trail it.
        let (text, losses) = decode("Shift_JIS", b"ok\x81\x20!");
        assert_eq!(text, "ok\u{FFFD} !");
        assert_eq!(losses, Losses { count: 1, first: Some(2) });
    }

    #[test]
    fn chunks_may_split_sequences() {
        let encoding = Encoding::for_name("Shift_JIS").unwrap();
        let (bytes, _, _) = encoding.0.encode("日本語のテキスト, ｶﾀｶﾅ");
        let mut whole = String::new();
        let mut decoder = Decoder::new(encoding);
        decoder.decode(&bytes, true, &mut whole);
        for split in 0..=bytes.len() {
            let mut decoder = Decoder::new(encoding);
            let mut out = String::new();
            decoder.decode(&bytes[..split], false, &mut out);
            decoder.decode(&bytes[split..], true, &mut out);
            assert_eq!(out, whole, "split {split}");
            assert_eq!(decoder.losses(), Losses::default());
        }
    }

    #[test]
    #[should_panic(expected = "not decoded")]
    fn utf8_is_not_decoded() {
        Decoder::new(Encoding::UTF_8);
    }
}
