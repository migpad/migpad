//! Saving without losses: what would be lost is found before anything is written, and the file
//! keeps its properties.

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::Instant;

use super::{Document, EditError, Fingerprint, Format};
use crate::encoding::{Encoder, Encoding, Losses};
use crate::history::{EditKind, Selection};
use crate::platform;
use crate::text::TextStore;

/// The text is encoded and written in pieces of this size.
const PIECE: usize = 1 << 20;

/// Why a document was not saved. In every case the file on disk is as it was.
#[derive(Debug)]
pub enum SaveError {
    /// Characters would be lost: `encoding` counts those the encoding cannot represent, and
    /// `decoding` the bytes of the file that could not be decoded when it was opened and would
    /// now be overwritten. Saving again with the losses accepted writes `?` for them.
    Losses {
        encoding: Losses,
        decoding: Losses,
    },
    /// The file, or the directory of a new file, may not be written.
    PermissionDenied(io::Error),
    Io(io::Error),
    /// The document is a preview: saving it would cut the file short, see
    /// [`Document::is_preview`].
    Preview,
}

impl From<io::Error> for SaveError {
    fn from(error: io::Error) -> Self {
        if error.kind() == io::ErrorKind::PermissionDenied {
            SaveError::PermissionDenied(error)
        } else {
            SaveError::Io(error)
        }
    }
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SaveError::Losses { encoding, decoding } => {
                write!(f, "{} characters would be lost", encoding.count + decoding.count)
            }
            SaveError::PermissionDenied(error) | SaveError::Io(error) => error.fmt(f),
            SaveError::Preview => f.write_str("the file is still loading"),
        }
    }
}

impl std::error::Error for SaveError {}

impl Document {
    /// Saves the text to `path` in `format`. Nothing is written if characters would be lost,
    /// unless `accept_losses`; see [`SaveError::Losses`]. A symbolic link stays: the file it
    /// points to is written.
    ///
    /// The file is replaced through a temporary file next to it, which takes over the
    /// permissions, owner, ACL and extended attributes of the old one; a file with several hard
    /// links, or in a directory that may not be written, is written in place instead.
    pub fn save(&mut self, path: &Path, format: Format, accept_losses: bool) -> Result<(), SaveError> {
        if self.preview {
            return Err(SaveError::Preview);
        }
        let target = fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
        // Bytes lost in decoding are gone for good once the file they came from is overwritten,
        // in whatever encoding.
        let source = self.path.as_deref().and_then(|source| fs::canonicalize(source).ok());
        let decoding = if source.as_deref() == Some(target.as_path()) { self.decode_losses } else { Losses::default() };
        let check = |encoding: Losses| {
            if accept_losses || (encoding.is_empty() && decoding.is_empty()) {
                Ok(())
            } else {
                Err(SaveError::Losses { encoding, decoding })
            }
        };
        let existing = match File::open(&target) {
            Ok(file) => Some(file),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        if let Some(file) = &existing {
            // A file that may not be written stays so: replacing it would get around that.
            OpenOptions::new().write(true).open(&target)?;
            if platform::link_count(file)? > 1 {
                self.write_in_place(&target, format, check)?;
                return self.finish_save(path, format, &target);
            }
        }
        if !self.write_replacing(&target, existing, format, check)? {
            // The directory may not be written, but the file may.
            self.write_in_place(&target, format, check)?;
        }
        self.finish_save(path, format, &target)
    }

    /// Writes a temporary file next to `target` and puts it in place of `target`. Returns
    /// `false` if the directory may not be written, so `target` should be written in place.
    fn write_replacing(
        &self,
        target: &Path,
        existing: Option<File>,
        format: Format,
        check: impl Fn(Losses) -> Result<(), SaveError>,
    ) -> Result<bool, SaveError> {
        let temp = temporary_path(target, self);
        let file = match OpenOptions::new().write(true).create_new(true).open(&temp) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied && existing.is_some() => return Ok(false),
            Err(error) => return Err(error.into()),
        };
        let write = || -> Result<(), SaveError> {
            let mut writer = BufWriter::with_capacity(PIECE, file);
            check(self.encode_into(format, &mut writer)?)?;
            let file = writer.into_inner().map_err(io::IntoInnerError::into_error)?;
            file.sync_all()?;
            if let Some(existing) = &existing {
                platform::copy_metadata(existing, &file)?;
            }
            // Windows does not replace a file that is open.
            drop((file, existing));
            platform::replace(target, &temp)?;
            Ok(())
        };
        let result = write();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result.map(|()| true)
    }

    /// Writes `target` in place. Losses are checked first, and the journal is flushed, so a crash
    /// in the middle of writing loses nothing but the file's old text.
    fn write_in_place(
        &mut self,
        target: &Path,
        format: Format,
        check: impl Fn(Losses) -> Result<(), SaveError>,
    ) -> Result<(), SaveError> {
        check(self.encode_into(format, &mut io::sink())?)?;
        self.sync_journal();
        let file = OpenOptions::new().write(true).truncate(true).open(target)?;
        let mut writer = BufWriter::with_capacity(PIECE, file);
        self.encode_into(format, &mut writer)?;
        writer.into_inner().map_err(io::IntoInnerError::into_error)?.sync_all()?;
        Ok(())
    }

    /// Replaces in the text what saving in `encoding` would lose — characters it lacks or writes
    /// as others, and invalid UTF-8 — with what is written for them: `?`, or U+FFFD in UTF-16.
    /// Then the document shows what its file gets. One undo step, in which `selection` moves with
    /// the text; returns the selection after it.
    pub fn replace_losses(
        &mut self,
        encoding: Encoding,
        selection: Selection,
        now: Instant,
    ) -> Result<Selection, EditError> {
        let mut encoder = Encoder::new(encoding, false).recording_places();
        self.encode_with(&mut encoder, &mut io::sink()).expect("a sink takes everything");
        let places = encoder.lost_places();
        if places.is_empty() {
            return Ok(selection);
        }
        let replacement: &[u8] = if encoding.is_utf16() { "\u{FFFD}".as_bytes() } else { b"?" };
        // The last first, so that each range still holds when the ones after it are replaced.
        let edits: Vec<(Range<usize>, &[u8])> = places.iter().rev().map(|place| (place.clone(), replacement)).collect();
        let moved = |pos: usize| {
            let mut moved = pos;
            for place in places.iter().take_while(|place| place.start < pos) {
                if place.end > pos {
                    // Within a replaced sequence: to the start of its replacement.
                    return moved - (pos - place.start);
                }
                moved = moved + replacement.len() - place.len();
            }
            moved
        };
        let after = Selection { anchor: moved(selection.anchor), head: moved(selection.head) };
        self.edit(&edits, selection, after, EditKind::Other, now)?;
        Ok(after)
    }

    /// Encodes the text in `format` into `out`; returns what was lost.
    fn encode_into(&self, format: Format, out: &mut impl Write) -> io::Result<Losses> {
        let mut encoder = Encoder::new(format.encoding, format.bom);
        self.encode_with(&mut encoder, out)?;
        Ok(encoder.losses())
    }

    /// Encodes the text with `encoder` into `out`.
    fn encode_with(&self, encoder: &mut Encoder, out: &mut impl Write) -> io::Result<()> {
        let mut encoded = Vec::with_capacity(PIECE);
        let len = self.text.len();
        let mut pos = 0;
        while pos < len {
            let end = (pos + PIECE).min(len);
            for chunk in self.text.chunks(pos..end) {
                encoder.encode(chunk, false, &mut encoded);
            }
            out.write_all(&encoded)?;
            encoded.clear();
            pos = end;
        }
        encoder.encode(&[], true, &mut encoded);
        out.write_all(&encoded)
    }

    fn finish_save(&mut self, path: &Path, format: Format, target: &Path) -> Result<(), SaveError> {
        let disk = Fingerprint::of_path(target)?;
        self.saved(path.to_owned(), format, disk);
        Ok(())
    }
}

/// A hidden file next to `target`, named after it and the document.
fn temporary_path(target: &Path, doc: &Document) -> PathBuf {
    let name = target.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    target.with_file_name(format!(".{name}.{}.migpad-tmp", doc.id))
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::document::{OpenAs, Opened, open};
    use crate::encoding::Encoding;
    use crate::history::{EditKind, Selection};
    use crate::testing::TempDir;

    fn load(path: &Path, open_as: OpenAs) -> Document {
        match open(path, open_as).unwrap() {
            Opened::Complete(doc) => doc,
            Opened::Partial { loader, .. } => loader.load().unwrap(),
        }
    }

    fn append(doc: &mut Document, text: &str) {
        let end = doc.text().len();
        let (before, after) = (Selection::caret(end), Selection::caret(end + text.len()));
        doc.edit(&[(end..end, text.as_bytes())], before, after, EditKind::Other, Instant::now()).unwrap();
    }

    /// A file with `content` in a new directory, opened and edited to end with "!".
    fn edited(name: &str, content: &[u8]) -> (TempDir, PathBuf, Document) {
        let dir = TempDir::new(name);
        let path = dir.0.join("file.txt");
        fs::write(&path, content).unwrap();
        let mut doc = load(&path, OpenAs::Detect { tld: None });
        append(&mut doc, "!");
        (dir, path, doc)
    }

    #[cfg(unix)]
    fn is_root() -> bool {
        // SAFETY: no arguments, no failure.
        unsafe { libc::geteuid() == 0 }
    }

    #[test]
    fn saves_a_new_file() {
        let dir = TempDir::new("save-new");
        let path = dir.0.join("new.txt");
        let mut doc = Document::new();
        append(&mut doc, "hello\n");
        doc.save(&path, Format::default(), false).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"hello\n");
        assert_eq!(doc.path.as_deref(), Some(path.as_path()));
        assert!(!doc.is_modified());
        assert_eq!(doc.disk, Some(Fingerprint::of_path(&path).unwrap()));
        assert_eq!(dir.files(), [path], "no temporary file is left");
    }

    #[test]
    fn losses_are_replaced_in_the_text_as_saving_writes_them() {
        let (_dir, path, mut doc) = edited("save-replace", b"text");
        append(&mut doc, " \u{1F600} and \u{20AC}\u{1F600}.");
        let windows_1251 = Encoding::for_name("windows-1251").unwrap();
        // The anchor within the first emoji, the caret after the second.
        let selected = Selection { anchor: 7, head: 22 };
        let after = doc.replace_losses(windows_1251, selected, Instant::now()).unwrap();
        assert_eq!(doc.text().to_vec(0..doc.text().len()), "text! ? and €?.".as_bytes());
        assert_eq!(after, Selection { anchor: 6, head: 16 });
        doc.save(&path, Format { encoding: windows_1251, ..doc.format }, false).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"text! ? and \x88?.");
        // One step back to the characters, which saving would lose again.
        assert_eq!(doc.undo(), Some(selected));
        assert!(doc.is_modified());
        assert_eq!(doc.replace_losses(Encoding::UTF_8, selected, Instant::now()), Ok(selected), "nothing to lose");
        assert!(doc.can_redo(), "no edit made");
    }

    #[test]
    fn the_saved_file_is_modified_now() {
        let (_dir, path, mut doc) = edited("save-time", b"old");
        let long_ago = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
        File::options().write(true).open(&path).unwrap().set_modified(long_ago).unwrap();
        let format = doc.format;
        doc.save(&path, format, false).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        assert!(modified > long_ago + std::time::Duration::from_secs(86_400 * 365 * 20), "{modified:?}");
        assert_eq!(doc.disk.and_then(|disk| disk.modified), Some(modified));
    }

    #[test]
    fn writes_the_byte_order_mark() {
        let (_dir, path, mut doc) = edited("save-bom", b"hi");
        let format = Format { encoding: Encoding::UTF_16LE, bom: true, ..doc.format };
        doc.save(&path, format, false).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"\xFF\xFEh\0i\0!\0");
        assert_eq!(doc.format, format);
    }

    #[cfg(unix)]
    #[test]
    fn replacing_keeps_the_permissions() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let (_dir, path, mut doc) = edited("save-permissions", b"#!/bin/sh\n");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o751)).unwrap();
        let inode = fs::metadata(&path).unwrap().ino();
        let format = doc.format;
        doc.save(&path, format, false).unwrap();
        let metadata = fs::metadata(&path).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"#!/bin/sh\n!");
        assert_eq!(metadata.mode() & 0o777, 0o751);
        assert_ne!(metadata.ino(), inode, "replaced, not written in place");
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn replacing_keeps_extended_attributes() {
        use std::ffi::CString;
        use std::os::fd::AsRawFd;
        let name =
            CString::new(if cfg!(target_os = "macos") { "com.migpad.test" } else { "user.migpad.test" }).unwrap();
        let set = |file: &File| -> bool {
            // SAFETY: an open descriptor, a NUL-terminated name and a 3-byte value.
            #[cfg(target_os = "macos")]
            let result = unsafe { libc::fsetxattr(file.as_raw_fd(), name.as_ptr(), b"tag".as_ptr().cast(), 3, 0, 0) };
            #[cfg(target_os = "linux")]
            let result = unsafe { libc::fsetxattr(file.as_raw_fd(), name.as_ptr(), b"tag".as_ptr().cast(), 3, 0) };
            result == 0
        };
        let get = |file: &File| -> Vec<u8> {
            let mut value = [0u8; 16];
            // SAFETY: an open descriptor, a NUL-terminated name and a 16-byte buffer.
            #[cfg(target_os = "macos")]
            let len = unsafe { libc::fgetxattr(file.as_raw_fd(), name.as_ptr(), value.as_mut_ptr().cast(), 16, 0, 0) };
            #[cfg(target_os = "linux")]
            let len = unsafe { libc::fgetxattr(file.as_raw_fd(), name.as_ptr(), value.as_mut_ptr().cast(), 16) };
            value[..usize::try_from(len).unwrap_or(0)].to_vec()
        };
        let (_dir, path, mut doc) = edited("save-xattr", b"text");
        if !set(&File::open(&path).unwrap()) {
            eprintln!("extended attributes are not supported here");
            return;
        }
        let format = doc.format;
        doc.save(&path, format, false).unwrap();
        assert_eq!(get(&File::open(&path).unwrap()), b"tag");
    }

    #[cfg(windows)]
    #[test]
    fn replacing_keeps_streams_and_creation_time() {
        let (_dir, path, mut doc) = edited("save-stream", b"text");
        let stream = PathBuf::from(format!("{}:migpad", path.display()));
        fs::write(&stream, b"stream").unwrap();
        let created = fs::metadata(&path).unwrap().created().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        let format = doc.format;
        doc.save(&path, format, false).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"text!");
        assert_eq!(fs::read(&stream).unwrap(), b"stream");
        assert_eq!(fs::metadata(&path).unwrap().created().unwrap(), created);
    }

    #[cfg(unix)]
    #[test]
    fn writes_through_a_symbolic_link() {
        let (dir, path, _) = edited("save-symlink", b"text");
        let link = dir.0.join("link.txt");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        let mut doc = load(&link, OpenAs::Detect { tld: None });
        append(&mut doc, "!");
        let format = doc.format;
        doc.save(&link, format, false).unwrap();
        assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(fs::read(&path).unwrap(), b"text!");
        assert_eq!(doc.path.as_deref(), Some(link.as_path()));
    }

    #[test]
    fn writes_hard_links_in_place() {
        let (dir, path, mut doc) = edited("save-hardlink", b"text");
        let link = dir.0.join("link.txt");
        fs::hard_link(&path, &link).unwrap();
        let format = doc.format;
        doc.save(&path, format, false).unwrap();
        assert_eq!(fs::read(&link).unwrap(), b"text!", "both names see the new text");
    }

    #[cfg(unix)]
    #[test]
    fn writes_in_place_when_the_directory_is_closed() {
        use std::os::unix::fs::PermissionsExt;
        if is_root() {
            return;
        }
        let (dir, path, mut doc) = edited("save-closed-dir", b"text");
        fs::set_permissions(&dir.0, fs::Permissions::from_mode(0o555)).unwrap();
        let format = doc.format;
        let result = doc.save(&path, format, false);
        fs::set_permissions(&dir.0, fs::Permissions::from_mode(0o755)).unwrap();
        result.unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"text!");
    }

    #[test]
    fn a_read_only_file_is_refused() {
        #[cfg(unix)]
        if is_root() {
            return;
        }
        let (_dir, path, mut doc) = edited("save-read-only", b"text");
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&path, permissions.clone()).unwrap();
        let format = doc.format;
        let result = doc.save(&path, format, false);
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        fs::set_permissions(&path, permissions).unwrap();
        assert!(matches!(result, Err(SaveError::PermissionDenied(_))), "{result:?}");
        assert_eq!(fs::read(&path).unwrap(), b"text");
        assert!(doc.is_modified());
    }

    #[test]
    fn characters_the_encoding_lacks_are_not_lost_silently() {
        let windows_1251 = Encoding::for_name("windows-1251").unwrap();
        let dir = TempDir::new("save-losses");
        let path = dir.0.join("file.txt");
        let original = encoding_rs::WINDOWS_1251.encode("Привет").0.into_owned();
        fs::write(&path, &original).unwrap();
        let mut doc = load(&path, OpenAs::Encoding(windows_1251));
        append(&mut doc, " 😀");
        let format = doc.format;
        match doc.save(&path, format, false) {
            Err(SaveError::Losses { encoding, decoding }) => {
                assert_eq!(encoding, Losses { count: 1, first: Some("Привет ".len()) });
                assert!(decoding.is_empty());
            }
            other => panic!("expected losses, got {other:?}"),
        }
        assert_eq!(fs::read(&path).unwrap(), original, "the file is untouched");
        doc.save(&path, format, true).unwrap();
        assert_eq!(fs::read(&path).unwrap(), [original.as_slice(), b" ?"].concat());
    }

    #[test]
    fn undecodable_bytes_are_not_overwritten_silently() {
        let shift_jis = Encoding::for_name("Shift_JIS").unwrap();
        let dir = TempDir::new("save-undecodable");
        let path = dir.0.join("file.txt");
        fs::write(&path, b"ok\x81\x20").unwrap();
        let mut doc = load(&path, OpenAs::Encoding(shift_jis));
        assert_eq!(doc.decode_losses().count, 1);
        let utf8 = Format { encoding: Encoding::UTF_8, ..doc.format };
        assert!(matches!(
            doc.save(&path, utf8, false),
            Err(SaveError::Losses { decoding: Losses { count: 1, .. }, .. })
        ));
        assert_eq!(fs::read(&path).unwrap(), b"ok\x81\x20");
        // Saving elsewhere keeps the original file, so nothing is lost.
        let copy = dir.0.join("copy.txt");
        doc.save(&copy, utf8, false).unwrap();
        assert_eq!(fs::read(&copy).unwrap(), "ok\u{FFFD} ".as_bytes());
    }
}
