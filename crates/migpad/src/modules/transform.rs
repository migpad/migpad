//! Transforms of the selected text: hex, Base64, Base58, URL encoding and decoding, BIP-39
//! mnemonic ↔ dig (word indices concatenated by fours), and its SHA-256 and SHA-512 hashes.

use base64::Engine;
use gpui::actions;
use migpad_core::history::{EditKind, Selection};
use migpad_core::text::TextStore;
use sha2::{Digest, Sha256, Sha512};

use crate::commands::{Command, MenuId, Module, Registry};
use crate::notices::Notice;
use crate::strings::Key;
use crate::workspace::Workspace;
use migpad_ui::notification::Severity;

actions!(
    transform,
    [
        HexEncode,
        HexDecode,
        Base64Encode,
        Base64Decode,
        Base58Encode,
        Base58Decode,
        UrlEncode,
        UrlDecode,
        Bip39ToDig,
        DigToBip39,
        Sha256Hash,
        Sha512Hash
    ]
);

pub struct TransformModule;

impl Module for TransformModule {
    fn id(&self) -> &'static str {
        "transform"
    }

    fn register(&self, registry: &mut Registry) {
        // A page shows no selection of the text: what would be replaced is not seen.
        let has_selection = |workspace: &Workspace, cx: &gpui::App| {
            let editor = workspace.editor().read(cx);
            !workspace.previewing(cx) && !editor.is_block_selection() && {
                let sel = editor.selection();
                sel.anchor != sel.head
            }
        };
        let commands = vec![
            Command::new("transform.hex_encode", Key::TransformHexEncode, HexEncode).enabled(has_selection),
            Command::new("transform.hex_decode", Key::TransformHexDecode, HexDecode).enabled(has_selection),
            Command::new("transform.base64_encode", Key::TransformBase64Encode, Base64Encode).enabled(has_selection),
            Command::new("transform.base64_decode", Key::TransformBase64Decode, Base64Decode).enabled(has_selection),
            Command::new("transform.base58_encode", Key::TransformBase58Encode, Base58Encode).enabled(has_selection),
            Command::new("transform.base58_decode", Key::TransformBase58Decode, Base58Decode).enabled(has_selection),
            Command::new("transform.url_encode", Key::TransformUrlEncode, UrlEncode).enabled(has_selection),
            Command::new("transform.url_decode", Key::TransformUrlDecode, UrlDecode).enabled(has_selection),
            Command::new("transform.bip39_to_dig", Key::TransformBip39ToDig, Bip39ToDig).enabled(has_selection),
            Command::new("transform.dig_to_bip39", Key::TransformDigToBip39, DigToBip39).enabled(has_selection),
            Command::new("transform.sha256", Key::TransformSha256, Sha256Hash).enabled(has_selection),
            Command::new("transform.sha512", Key::TransformSha512, Sha512Hash).enabled(has_selection),
        ];
        registry.add_group(Key::TransformMenu, commands, MenuId::Edit, 3);

        macro_rules! on_transform {
            ($registry:expr, $action:ty, $func:expr) => {
                $registry.on_window_action(|workspace, _: &$action, window, cx| {
                    apply_transform(workspace, $func, window, cx);
                });
            };
        }
        on_transform!(registry, HexEncode, |bytes: &[u8]| Ok(hex_encode(bytes)));
        on_transform!(registry, HexDecode, |bytes: &[u8]| hex_decode(bytes));
        on_transform!(registry, Base64Encode, |bytes: &[u8]| Ok(base64_encode(bytes)));
        on_transform!(registry, Base64Decode, |bytes: &[u8]| base64_decode(bytes));
        on_transform!(registry, Base58Encode, |bytes: &[u8]| Ok(base58_encode(bytes)));
        on_transform!(registry, Base58Decode, |bytes: &[u8]| base58_decode(bytes));
        on_transform!(registry, UrlEncode, |bytes: &[u8]| Ok(url_encode(bytes)));
        on_transform!(registry, UrlDecode, |bytes: &[u8]| url_decode(bytes));
        on_transform!(registry, Bip39ToDig, |bytes: &[u8]| bip39_to_dig(bytes));
        on_transform!(registry, DigToBip39, |bytes: &[u8]| dig_to_bip39(bytes));
        on_transform!(registry, Sha256Hash, |bytes: &[u8]| Ok(hash::<Sha256>(bytes)));
        on_transform!(registry, Sha512Hash, |bytes: &[u8]| Ok(hash::<Sha512>(bytes)));
    }
}

fn apply_transform(
    workspace: &mut Workspace,
    transform: fn(&[u8]) -> Result<Vec<u8>, &'static str>,
    window: &mut gpui::Window,
    cx: &mut gpui::Context<Workspace>,
) {
    let editor = workspace.editor().clone();
    let sel = editor.read(cx).selection();
    let range = sel.anchor.min(sel.head)..sel.anchor.max(sel.head);
    if range.is_empty() || workspace.previewing(cx) {
        return;
    }
    let bytes = editor.read(cx).document().read(cx).text().to_vec(range.clone());
    match transform(&bytes) {
        Ok(result) => {
            let after = Selection { anchor: range.start, head: range.start + result.len() };
            editor.update(cx, |view, cx| {
                view.replace_selection(&result, EditKind::Other, after, window, cx);
            });
        }
        Err(message) => {
            workspace.notify(Notice::new(Severity::Warning, message.to_owned()), cx);
        }
    }
}

fn hex_encode(bytes: &[u8]) -> Vec<u8> {
    let hex: String = bytes.iter().map(|b| format!("{b:02X}")).collect();
    hex.into_bytes()
}

fn hex_decode(bytes: &[u8]) -> Result<Vec<u8>, &'static str> {
    let s = std::str::from_utf8(bytes).map_err(|_| "not valid UTF-8")?;
    let s = s.trim();
    if !s.is_ascii() {
        return Err("hex: non-hex characters");
    }
    if s.len() % 2 != 0 {
        return Err("hex: odd number of digits");
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| "hex: invalid digit")).collect()
}

fn base64_encode(bytes: &[u8]) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD.encode(bytes).into_bytes()
}

fn base64_decode(bytes: &[u8]) -> Result<Vec<u8>, &'static str> {
    let s = std::str::from_utf8(bytes).map_err(|_| "not valid UTF-8")?;
    base64::engine::general_purpose::STANDARD.decode(s.trim()).map_err(|_| "Base64: invalid input")
}

fn base58_encode(bytes: &[u8]) -> Vec<u8> {
    bs58::encode(bytes).into_string().into_bytes()
}

fn base58_decode(bytes: &[u8]) -> Result<Vec<u8>, &'static str> {
    let s = std::str::from_utf8(bytes).map_err(|_| "not valid UTF-8")?;
    bs58::decode(s.trim()).into_vec().map_err(|_| "Base58: invalid input")
}

fn url_encode(bytes: &[u8]) -> Vec<u8> {
    percent_encoding::percent_encode(bytes, percent_encoding::NON_ALPHANUMERIC).to_string().into_bytes()
}

fn url_decode(bytes: &[u8]) -> Result<Vec<u8>, &'static str> {
    let s = std::str::from_utf8(bytes).map_err(|_| "not valid UTF-8")?;
    let decoded = percent_encoding::percent_decode_str(s).decode_utf8().map_err(|_| "URL: invalid encoding")?;
    Ok(decoded.into_owned().into_bytes())
}

const BIP39_WORDS: &str = include_str!("../bip39_english.txt");

fn bip39_word_list() -> Vec<&'static str> {
    BIP39_WORDS.lines().collect()
}

fn bip39_to_dig(bytes: &[u8]) -> Result<Vec<u8>, &'static str> {
    let s = std::str::from_utf8(bytes).map_err(|_| "not valid UTF-8")?;
    let words = bip39_word_list();
    let mut dig = String::new();
    for word in s.split_whitespace() {
        let word_lower = word.to_lowercase();
        let index = words.iter().position(|&w| w == word_lower).ok_or("BIP-39: unknown word")?;
        dig.push_str(&format!("{index:04}"));
    }
    Ok(dig.into_bytes())
}

fn dig_to_bip39(bytes: &[u8]) -> Result<Vec<u8>, &'static str> {
    let s = std::str::from_utf8(bytes).map_err(|_| "not valid UTF-8")?;
    let s = s.trim();
    if !s.is_ascii() {
        return Err("dig: non-digit characters");
    }
    if s.len() % 4 != 0 {
        return Err("dig: length must be a multiple of 4");
    }
    let words = bip39_word_list();
    let mut out = Vec::new();
    for chunk in (0..s.len()).step_by(4) {
        let index: usize = s[chunk..chunk + 4].parse().map_err(|_| "dig: invalid digit")?;
        if index >= words.len() {
            return Err("dig: index out of range");
        }
        if !out.is_empty() {
            out.push(b' ');
        }
        out.extend_from_slice(words[index].as_bytes());
    }
    Ok(out)
}

/// The hash of `bytes` in hex, in small letters as `sha256sum` and `sha512sum` print it.
fn hash<D: Digest>(bytes: &[u8]) -> Vec<u8> {
    let hex: String = D::digest(bytes).iter().map(|b| format!("{b:02x}")).collect();
    hex.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        let original = b"Hello, World!";
        let encoded = hex_encode(original);
        assert_eq!(encoded, b"48656C6C6F2C20576F726C6421");
        let decoded = hex_decode(&encoded).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn base64_round_trip() {
        let original = b"Hello, World!";
        let encoded = base64_encode(original);
        assert_eq!(encoded, b"SGVsbG8sIFdvcmxkIQ==");
        let decoded = base64_decode(&encoded).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn base58_round_trip() {
        let original = b"Hello";
        let encoded = base58_encode(original);
        let decoded = base58_decode(&encoded).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn url_round_trip() {
        let original = b"hello world&foo=bar";
        let encoded = url_encode(original);
        assert_eq!(encoded, b"hello%20world%26foo%3Dbar");
        let decoded = url_decode(&encoded).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn bip39_dig_round_trip() {
        let mnemonic = b"abandon ability";
        let dig = bip39_to_dig(mnemonic).unwrap();
        assert_eq!(dig, b"00000001");
        let back = dig_to_bip39(&dig).unwrap();
        assert_eq!(back, b"abandon ability");
    }

    #[test]
    fn sha_hashes_of_the_standard() {
        assert_eq!(hash::<Sha256>(b"abc"), b"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(
            hash::<Sha512>(b"abc"),
            b"ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a\
              2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
        );
        // UTF-8, as the text is kept.
        assert_eq!(hash::<Sha256>("ё".as_bytes()), hash::<Sha256>(&[0xd1, 0x91]));
    }

    #[test]
    fn hex_decode_errors() {
        assert!(hex_decode(b"GG").is_err());
        assert!(hex_decode(b"ABC").is_err());
    }
}
