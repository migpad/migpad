//! Opening files: the first screen at once, the rest in the background.

use std::fmt;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use super::{Document, Fingerprint, Format, Text};
use crate::encoding::{Decoder, Detected, Encoding, Losses, detect};
use crate::line_ending::LineEnding;
use crate::text::{GapBuffer, Indexed, Indexer, MAX_LEN};

/// Bytes read at once, for the first screen and for detecting the encoding.
pub const PREVIEW_LEN: usize = 256 << 10;

/// Files from this size on are large: features that would slow them down are turned off,
/// and the encoding is detected from the first [`PREVIEW_LEN`] bytes only.
pub const LARGE_FILE: u64 = 64 << 20;

/// How [`open`] chooses the encoding of a file.
#[derive(Clone, Copy, Debug)]
pub enum OpenAs<'a> {
    /// Detect it; `tld` hints the region, see [`detect`].
    Detect { tld: Option<&'a str> },
    /// Use this encoding: "Reopen in encoding", or the one remembered for a recent file.
    /// A byte order mark of this encoding is still recognized.
    Encoding(Encoding),
}

/// A file opened by [`open`].
#[derive(Debug)]
pub enum Opened {
    /// The whole file.
    Complete(Document),
    /// The beginning of a larger file, to show while [`Loader::load`] reads the rest.
    Partial { preview: Document, loader: Loader },
}

/// Why a file could not be opened.
#[derive(Debug)]
pub enum OpenError {
    Io(io::Error),
    /// The file, or its text in UTF-8, is longer than [`MAX_LEN`] bytes.
    TooLarge,
    /// [`Loader::load`] was cancelled.
    Cancelled,
}

impl From<io::Error> for OpenError {
    fn from(error: io::Error) -> Self {
        OpenError::Io(error)
    }
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OpenError::Io(error) => error.fmt(f),
            OpenError::TooLarge => write!(f, "the text is longer than {MAX_LEN} bytes"),
            OpenError::Cancelled => f.write_str("loading was cancelled"),
        }
    }
}

impl std::error::Error for OpenError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            OpenError::Io(error) => Some(error),
            _ => None,
        }
    }
}

/// Opens the file at `path`: reads its beginning, chooses the encoding and returns the whole
/// document for a small file, or a preview and a [`Loader`] for a larger one.
pub fn open(path: &Path, open_as: OpenAs) -> Result<Opened, OpenError> {
    open_with(path, open_as, Limits::DEFAULT)
}

/// Sizes used by loading; tests make them small.
#[derive(Clone, Copy, Debug)]
struct Limits {
    preview: usize,
    chunk: usize,
    large: u64,
    max: usize,
}

impl Limits {
    const DEFAULT: Limits = Limits { preview: PREVIEW_LEN, chunk: 16 << 20, large: LARGE_FILE, max: MAX_LEN };
}

fn open_with(path: &Path, open_as: OpenAs, limits: Limits) -> Result<Opened, OpenError> {
    let mut file = File::open(path)?;
    let metadata = file.metadata()?;
    if metadata.len() > limits.max as u64 {
        return Err(OpenError::TooLarge);
    }
    let len = metadata.len() as usize;
    let mut sample = Vec::with_capacity(len.min(limits.preview) + 1);
    // One byte more than the preview tells whether the sample is the whole file.
    (&mut file).take(limits.preview as u64 + 1).read_to_end(&mut sample)?;
    let complete = sample.len() <= limits.preview;
    sample.truncate(limits.preview);
    let (Detected { encoding, bom }, tld) = match open_as {
        OpenAs::Detect { tld } => (detect(&sample, complete, tld), tld.map(str::to_owned)),
        OpenAs::Encoding(encoding) => {
            let bom = !encoding.bom().is_empty() && sample.starts_with(encoding.bom());
            (Detected { encoding, bom }, None)
        }
    };
    let disk = Fingerprint::of_file(&file)?;
    let loader = Loader {
        file,
        path: path.to_owned(),
        disk,
        len,
        encoding,
        bom,
        detected: matches!(open_as, OpenAs::Detect { .. }),
        tld,
        large: metadata.len() >= limits.large,
        progress: Arc::new(AtomicU64::new(0)),
        cancel: Arc::new(AtomicBool::new(false)),
        limits,
    };
    let body = &sample[if bom { encoding.bom().len() } else { 0 }..];
    if complete {
        let (text, losses) = decode_all(body, encoding, limits.max)?;
        return Ok(Opened::Complete(loader.document(text, losses)));
    }
    let preview = loader.preview(body);
    Ok(Opened::Partial { preview, loader })
}

/// Reads the rest of a file opened by [`open`]; meant for a background thread.
#[derive(Debug)]
pub struct Loader {
    file: File,
    path: PathBuf,
    disk: Fingerprint,
    len: usize,
    encoding: Encoding,
    bom: bool,
    /// The encoding was detected rather than chosen, so it may be detected again.
    detected: bool,
    tld: Option<String>,
    large: bool,
    progress: Arc<AtomicU64>,
    cancel: Arc<AtomicBool>,
    limits: Limits,
}

impl Loader {
    /// Length of the file in bytes.
    pub fn file_len(&self) -> u64 {
        self.len as u64
    }

    /// Bytes of the file read so far; updated as [`Loader::load`] goes.
    pub fn progress(&self) -> Arc<AtomicU64> {
        self.progress.clone()
    }

    /// Setting it to `true` makes [`Loader::load`] stop with [`OpenError::Cancelled`].
    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.cancel.clone()
    }

    /// Reads the whole file. UTF-8 is read straight into the memory of the text and indexed chunk
    /// by chunk; other encodings are decoded once.
    pub fn load(mut self) -> Result<Document, OpenError> {
        let skip = if self.bom { self.encoding.bom().len() } else { 0 };
        self.file.seek(SeekFrom::Start(skip as u64))?;
        let len = self.len - skip;
        if !self.encoding.is_utf8() {
            return self.load_decoded(len);
        }
        // Zeroed memory comes from the system lazily: pages are touched only by reading.
        let mut data = vec![0u8; len + GapBuffer::initial_gap(len)];
        let mut indexer = Indexer::new(len / 64);
        let mut done = 0;
        while done < len {
            self.check_cancel()?;
            let n = self.limits.chunk.min(len - done);
            self.file.read_exact(&mut data[done..done + n])?;
            indexer.feed(&data[done..done + n]);
            done += n;
            self.progress.store((skip + done) as u64, Ordering::Relaxed);
        }
        data.truncate(len);
        // A file that looked like UTF-8 at its beginning may turn out to be in another encoding.
        // Large files trust the beginning: checking all of them would take too long.
        if self.detected && !self.large && !self.bom && std::str::from_utf8(&data).is_err() {
            let Detected { encoding, bom } = detect(&data, true, self.tld.as_deref());
            if !encoding.is_utf8() {
                self.encoding = encoding;
                self.bom = bom;
                let (text, losses) = decode_all(&data, encoding, self.limits.max)?;
                return Ok(self.document(text, losses));
            }
        }
        Ok(self.assemble(GapBuffer::from_vec(data), indexer.finish(), Losses::default()))
    }

    fn load_decoded(mut self, len: usize) -> Result<Document, OpenError> {
        let mut decoder = Decoder::new(self.encoding);
        let mut text = String::with_capacity(len.saturating_mul(2).min(self.limits.max) + GapBuffer::initial_gap(len));
        let mut indexer = Indexer::new(len / 64);
        let mut chunk = vec![0u8; self.limits.chunk.min(len)];
        let mut done = 0;
        loop {
            self.check_cancel()?;
            let n = chunk.len().min(len - done);
            self.file.read_exact(&mut chunk[..n])?;
            done += n;
            let indexed = text.len();
            decoder.decode(&chunk[..n], done == len, &mut text);
            if text.len() > self.limits.max {
                return Err(OpenError::TooLarge);
            }
            indexer.feed(&text.as_bytes()[indexed..]);
            self.progress.store((self.len - len + done) as u64, Ordering::Relaxed);
            if done == len {
                break;
            }
        }
        let losses = decoder.losses();
        Ok(self.assemble(GapBuffer::from_vec(text.into_bytes()), indexer.finish(), losses))
    }

    /// The document for a preview: the beginning of the file, cut at a character boundary.
    fn preview(&self, body: &[u8]) -> Document {
        let (text, losses) = if self.encoding.is_utf8() {
            let end = match std::str::from_utf8(body) {
                Err(error) if error.error_len().is_none() => error.valid_up_to(),
                _ => body.len(),
            };
            (body[..end].to_vec(), Losses::default())
        } else {
            let mut decoder = Decoder::new(self.encoding);
            let mut text = String::new();
            decoder.decode(body, false, &mut text);
            (text.into_bytes(), decoder.losses())
        };
        Document { preview: true, ..self.document(text, losses) }
    }

    fn document(&self, text: Vec<u8>, losses: Losses) -> Document {
        let text = GapBuffer::from_vec(text);
        let indexed = Indexer::index(&text);
        self.assemble(text, indexed, losses)
    }

    fn assemble(&self, text: Text, indexed: Indexed, losses: Losses) -> Document {
        Document {
            text,
            lines: indexed.lines,
            format: Format {
                encoding: self.encoding,
                bom: self.bom,
                line_ending: LineEnding::dominant(indexed.eol).unwrap_or(LineEnding::Lf),
            },
            path: Some(self.path.clone()),
            disk: Some(self.disk),
            large: self.large,
            decode_losses: losses,
            ..Document::new()
        }
    }

    fn check_cancel(&self) -> Result<(), OpenError> {
        if self.cancel.load(Ordering::Relaxed) { Err(OpenError::Cancelled) } else { Ok(()) }
    }
}

/// The text of `body` (the file without its byte order mark) in UTF-8, with a gap reserved,
/// and the bytes lost in decoding.
fn decode_all(body: &[u8], encoding: Encoding, max: usize) -> Result<(Vec<u8>, Losses), OpenError> {
    if encoding.is_utf8() {
        let mut text = Vec::with_capacity(body.len() + GapBuffer::initial_gap(body.len()));
        text.extend_from_slice(body);
        return Ok((text, Losses::default()));
    }
    let mut decoder = Decoder::new(encoding);
    let mut text = String::with_capacity(body.len() * 2 + GapBuffer::initial_gap(body.len()));
    decoder.decode(body, true, &mut text);
    if text.len() > max {
        return Err(OpenError::TooLarge);
    }
    Ok((text.into_bytes(), decoder.losses()))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::testing::TempFile;
    use crate::text::TextStore;

    /// Limits that make files of a few hundred bytes partial, and large from 1000 bytes.
    const SMALL: Limits = Limits { preview: 64, chunk: 50, large: 1000, max: 5000 };

    const PANGRAM: &str = "Съешь же ещё этих мягких французских булок, да выпей чаю.\n";

    fn detect_ru(file: &TempFile, limits: Limits) -> Result<Opened, OpenError> {
        open_with(&file.0, OpenAs::Detect { tld: Some("ru") }, limits)
    }

    fn complete(opened: Opened) -> Document {
        match opened {
            Opened::Complete(doc) => doc,
            Opened::Partial { .. } => panic!("expected the whole file"),
        }
    }

    fn partial(opened: Opened) -> (Document, Loader) {
        match opened {
            Opened::Partial { preview, loader } => (preview, loader),
            Opened::Complete(_) => panic!("expected a preview"),
        }
    }

    fn text(doc: &Document) -> Vec<u8> {
        doc.text().to_vec(0..doc.text().len())
    }

    fn windows_1251() -> Encoding {
        Encoding::for_name("windows-1251").unwrap()
    }

    fn cp1251(text: &str) -> Vec<u8> {
        encoding_rs::WINDOWS_1251.encode(text).0.into_owned()
    }

    #[test]
    fn small_files_open_whole() {
        let utf16: Vec<u8> =
            b"\xFF\xFE".iter().copied().chain("hi\n".encode_utf16().flat_map(u16::to_le_bytes)).collect();
        let cases = [
            ("utf8", b"hello\nworld\n".to_vec(), Encoding::UTF_8, false, "hello\nworld\n", LineEnding::Lf),
            ("utf8-bom", b"\xEF\xBB\xBFhi\r\nthere".to_vec(), Encoding::UTF_8, true, "hi\r\nthere", LineEnding::CrLf),
            ("cp1251", cp1251(&PANGRAM.repeat(20)), windows_1251(), false, &PANGRAM.repeat(20), LineEnding::Lf),
            ("utf16", utf16, Encoding::UTF_16LE, true, "hi\n", LineEnding::Lf),
            ("empty", Vec::new(), Encoding::UTF_8, false, "", LineEnding::Lf),
        ];
        for (name, bytes, encoding, bom, expected, line_ending) in cases {
            let file = TempFile::new(name, &bytes);
            let doc = complete(detect_ru(&file, Limits::DEFAULT).unwrap());
            assert_eq!(doc.format, Format { encoding, bom, line_ending }, "{name}");
            assert_eq!(text(&doc), expected.as_bytes(), "{name}");
            assert_eq!(doc.lines().count(), expected.split('\n').count(), "{name}");
            assert_eq!(doc.path.as_deref(), Some(file.0.as_path()), "{name}");
            assert_eq!(doc.disk.unwrap().len, bytes.len() as u64, "{name}");
            assert!(doc.decode_losses().is_empty() && !doc.is_large(), "{name}");
        }
    }

    #[test]
    fn larger_files_show_a_preview_then_load() {
        let content = PANGRAM.repeat(5);
        for (name, bytes) in
            [("plain", content.as_bytes().to_vec()), ("bom", [b"\xEF\xBB\xBF", content.as_bytes()].concat())]
        {
            let file = TempFile::new(&format!("partial-{name}"), &bytes);
            let (preview, loader) = partial(detect_ru(&file, SMALL).unwrap());
            let shown =
                std::str::from_utf8(&text(&preview)).expect("the preview ends at a character boundary").to_owned();
            assert!(!shown.is_empty() && content.starts_with(&shown), "{name}");
            assert_eq!(preview.format.bom, name == "bom");
            let progress = loader.progress();
            let doc = loader.load().unwrap();
            assert_eq!(text(&doc), content.as_bytes(), "{name}");
            assert_eq!(doc.lines().count(), 6);
            assert_eq!(progress.load(Ordering::Relaxed), bytes.len() as u64, "{name}");
            assert_eq!(doc.format.bom, name == "bom");
        }
    }

    #[test]
    fn a_preview_is_neither_edited_nor_saved() {
        use std::time::Instant;

        use crate::document::{EditError, SaveError};
        use crate::history::{EditKind, Selection};

        let content = PANGRAM.repeat(5);
        let file = TempFile::new("partial-read-only", content.as_bytes());
        let (mut preview, loader) = partial(detect_ru(&file, SMALL).unwrap());
        let typed = |doc: &mut Document| {
            doc.edit(&[(0..0, b"x")], Selection::default(), Selection::caret(1), EditKind::Typing, Instant::now())
        };
        assert!(preview.is_preview());
        assert_eq!(typed(&mut preview), Err(EditError::Preview));
        let format = preview.format;
        assert!(matches!(preview.save(&file.0, format, false), Err(SaveError::Preview)));
        assert_eq!(fs::read(&file.0).unwrap(), content.as_bytes(), "the file is as it was");
        let mut doc = loader.load().unwrap();
        assert!(!doc.is_preview());
        assert_eq!(typed(&mut doc), Ok(()));
    }

    #[test]
    fn legacy_and_utf16_files_load_in_chunks() {
        let content = PANGRAM.repeat(5);
        let utf16: Vec<u8> =
            b"\xFE\xFF".iter().copied().chain(content.encode_utf16().flat_map(u16::to_be_bytes)).collect();
        for (name, bytes, open_as) in [
            ("cp1251", cp1251(&content), OpenAs::Encoding(windows_1251())),
            ("utf16", utf16, OpenAs::Detect { tld: None }),
        ] {
            let file = TempFile::new(&format!("chunks-{name}"), &bytes);
            let (preview, loader) = partial(open_with(&file.0, open_as, SMALL).unwrap());
            assert!(content.as_bytes().starts_with(&text(&preview)), "{name}");
            let doc = loader.load().unwrap();
            assert_eq!(String::from_utf8(text(&doc)).unwrap(), content, "{name}");
            assert_eq!(doc.lines().count(), 6, "{name}");
        }
    }

    #[test]
    fn a_file_that_is_not_utf8_after_its_beginning_is_detected_again() {
        let bytes = [b"plain ascii\n".repeat(10), cp1251(&PANGRAM.repeat(3))].concat();
        let file = TempFile::new("redetect", &bytes);
        let (preview, loader) = partial(detect_ru(&file, SMALL).unwrap());
        assert_eq!(preview.format.encoding, Encoding::UTF_8);
        let doc = loader.load().unwrap();
        assert_eq!(doc.format.encoding, windows_1251());
        assert_eq!(String::from_utf8(text(&doc)).unwrap(), "plain ascii\n".repeat(10) + &PANGRAM.repeat(3));
    }

    #[test]
    fn large_files_trust_their_beginning() {
        let bytes = [b"plain ascii\n".repeat(100), cp1251(PANGRAM)].concat();
        let file = TempFile::new("large", &bytes);
        let (preview, loader) = partial(detect_ru(&file, SMALL).unwrap());
        assert!(preview.is_large());
        let doc = loader.load().unwrap();
        assert!(doc.is_large());
        assert_eq!(doc.format.encoding, Encoding::UTF_8);
        assert_eq!(text(&doc), bytes, "bytes stay as they are");
    }

    #[test]
    fn a_chosen_encoding_is_used() {
        let file = TempFile::new("chosen", &cp1251(PANGRAM));
        let doc = complete(open_with(&file.0, OpenAs::Encoding(Encoding::UTF_8), Limits::DEFAULT).unwrap());
        assert_eq!(doc.format.encoding, Encoding::UTF_8);
        assert_eq!(text(&doc), cp1251(PANGRAM), "UTF-8 keeps invalid bytes");
        let file = TempFile::new("chosen-bom", b"\xFF\xFEh\0i\0");
        let doc = complete(open_with(&file.0, OpenAs::Encoding(Encoding::UTF_16LE), Limits::DEFAULT).unwrap());
        assert_eq!((doc.format.bom, text(&doc)), (true, b"hi".to_vec()));
    }

    #[test]
    fn too_large_texts_are_refused() {
        let file = TempFile::new("too-large", &vec![b'x'; 6000]);
        assert!(matches!(detect_ru(&file, SMALL), Err(OpenError::TooLarge)));
        // 3000 bytes of Cyrillic in windows-1251 take about 6000 in UTF-8.
        let file = TempFile::new("too-large-decoded", &cp1251(&PANGRAM.repeat(52)));
        let (_, loader) = partial(open_with(&file.0, OpenAs::Encoding(windows_1251()), SMALL).unwrap());
        assert!(matches!(loader.load(), Err(OpenError::TooLarge)));
    }

    #[test]
    fn loading_can_be_cancelled() {
        let file = TempFile::new("cancel", PANGRAM.repeat(5).as_bytes());
        let (_, loader) = partial(detect_ru(&file, SMALL).unwrap());
        loader.cancel_flag().store(true, Ordering::Relaxed);
        assert!(matches!(loader.load(), Err(OpenError::Cancelled)));
    }

    #[test]
    fn a_file_that_shrinks_while_loading_fails() {
        let file = TempFile::new("shrink", PANGRAM.repeat(5).as_bytes());
        let (_, loader) = partial(detect_ru(&file, SMALL).unwrap());
        fs::write(&file.0, PANGRAM).unwrap();
        match loader.load() {
            Err(OpenError::Io(error)) => assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof),
            other => panic!("unexpected {other:?}"),
        }
    }
}
