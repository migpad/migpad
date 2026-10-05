//! Documents: the text with its line index, the format of its file and the state of the file
//! on disk.

mod load;

use std::fmt;
use std::fs::Metadata;
use std::ops::Range;
use std::path::PathBuf;
use std::time::SystemTime;

pub use load::{LARGE_FILE, Loader, OpenAs, OpenError, Opened, PREVIEW_LEN, open};

use crate::encoding::{Encoding, Losses};
use crate::line_ending::LineEnding;
use crate::text::{GapBuffer, LineIndex, MAX_LEN, TextStore};

/// The text store of documents; a store that reads large files in parts may replace it later.
pub type Text = GapBuffer;

/// A text being edited, with or without a file.
#[derive(Debug)]
pub struct Document {
    text: Text,
    lines: LineIndex,
    pub format: Format,
    pub path: Option<PathBuf>,
    /// The file as it was when it was loaded or last saved.
    pub disk: Option<Fingerprint>,
    large: bool,
    decode_losses: Losses,
}

impl Document {
    /// An empty untitled document.
    pub fn new() -> Self {
        Document {
            text: Text::new(),
            lines: LineIndex::new(),
            format: Format::default(),
            path: None,
            disk: None,
            large: false,
            decode_losses: Losses::default(),
        }
    }

    pub fn text(&self) -> &Text {
        &self.text
    }

    pub fn lines(&self) -> &LineIndex {
        &self.lines
    }

    /// Whether the file is large enough to turn off the features that would slow it down.
    pub fn is_large(&self) -> bool {
        self.large
    }

    /// Bytes of the file that could not be decoded when it was loaded: they became U+FFFD,
    /// so saving would not restore them.
    pub fn decode_losses(&self) -> Losses {
        self.decode_losses
    }

    /// Replaces `range` with `text` and updates the line index.
    ///
    /// # Panics
    ///
    /// If `range` is reversed or out of bounds.
    pub fn replace(&mut self, range: Range<usize>, text: &[u8]) -> Result<(), TooLong> {
        if self.text.len().saturating_sub(range.len()) + text.len() > MAX_LEN {
            return Err(TooLong);
        }
        self.text.replace(range.clone(), text);
        self.lines.on_edit(&self.text, range, text.len());
        Ok(())
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

/// How a document is stored in its file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    pub encoding: Encoding,
    pub bom: bool,
    /// The line break that Enter inserts: the one the file uses most, LF for new files.
    pub line_ending: LineEnding,
}

impl Default for Format {
    fn default() -> Self {
        Format { encoding: Encoding::UTF_8, bom: false, line_ending: LineEnding::Lf }
    }
}

/// The state of a file on disk, to notice when another program changes or replaces it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fingerprint {
    pub len: u64,
    pub modified: Option<SystemTime>,
    /// Device and inode on Unix. None on Windows for now: its file index is not in stable Rust.
    pub id: Option<(u64, u64)>,
}

impl Fingerprint {
    pub fn of(metadata: &Metadata) -> Self {
        #[cfg(unix)]
        let id = {
            use std::os::unix::fs::MetadataExt;
            Some((metadata.dev(), metadata.ino()))
        };
        #[cfg(not(unix))]
        let id = None;
        Fingerprint { len: metadata.len(), modified: metadata.modified().ok(), id }
    }
}

/// An edit would make the text longer than [`MAX_LEN`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TooLong;

impl fmt::Display for TooLong {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the text would be longer than {MAX_LEN} bytes")
    }
}

impl std::error::Error for TooLong {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_updates_the_line_index() {
        let mut doc = Document::new();
        assert_eq!(doc.lines().count(), 1);
        doc.replace(0..0, b"one\ntwo\r\nthree").unwrap();
        assert_eq!(doc.lines().count(), 3);
        doc.replace(3..9, b" ").unwrap();
        assert_eq!(doc.text().to_vec(0..doc.text().len()), b"one three");
        assert_eq!(doc.lines().count(), 1);
    }

    #[test]
    fn new_documents_are_utf8_with_lf() {
        let doc = Document::new();
        assert_eq!(doc.format, Format { encoding: Encoding::UTF_8, bom: false, line_ending: LineEnding::Lf });
        assert_eq!((doc.path.as_ref(), doc.disk, doc.is_large()), (None, None, false));
    }
}
