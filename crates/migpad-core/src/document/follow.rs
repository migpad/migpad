//! Following a file that another program adds to, as `tail -f` does: the bytes added to its end
//! are read and go on the end of the document; a file cut short or replaced is to be read anew.
//!
//! The file is not kept open between looks: the program that writes it may rename it and start
//! another, as logs are rotated.

use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use super::{Document, EditError, Fingerprint, Format, LARGE_FILE, apply, next_version};
use crate::encoding::{Decoder, Encoder, Losses};
use crate::history::History;
use crate::text::{MAX_LEN, TextStore};

/// How many of the bytes read last are compared with the file at each look: they must still be
/// there for what follows them to be an addition.
const TAIL: usize = 4096;

/// How many looks in a row must not find the file for it to be taken for removed: a log rotated
/// is renamed, and the program that writes it makes another a moment later.
const MISSING_LOOKS: u32 = 4;

/// What a look at a followed file found.
#[derive(Debug)]
pub enum Change {
    /// Nothing that shows: the file is as it was, or its time only changed.
    None,
    /// Bytes added at its end; their text, if they make any yet, for [`Document::append_from_file`].
    Appended(Appended),
    /// Cut short, or written anew, or another file in its place: it is to be read anew.
    Replaced,
    /// The file is not there any more, at [`MISSING_LOOKS`] looks in a row.
    Deleted,
    /// The file, or the text with what was added, is longer than a document can be.
    TooLong,
}

/// Text added to the end of a followed file.
#[derive(Debug)]
pub struct Appended {
    /// The file as the document has it: the text goes on from there.
    pub from: Fingerprint,
    /// The file now.
    pub disk: Fingerprint,
    /// The text, in UTF-8; empty if the bytes added do not make a whole character yet.
    pub text: Vec<u8>,
    /// Bytes that could not be decoded, as offsets in `text`.
    pub losses: Losses,
}

/// Reads what is added to the file that a document was read from.
pub struct Follower {
    path: PathBuf,
    format: Format,
    /// The file as it was at the last look; at first, as the document read it.
    disk: Fingerprint,
    /// How many bytes of the file are read.
    read: u64,
    /// The last of them, up to [`TAIL`]; at first, those of the text of the document as the file
    /// has them — not known where it lost bytes in reading.
    tail: Option<Vec<u8>>,
    /// Whether the text ends with a character lost in reading: the file may have ended with one cut
    /// off, which what is added finishes — then it is to be read anew.
    cut: bool,
    /// Bytes of UTF-8 read that do not make a whole character yet, and a CR that a LF may
    /// follow: they wait for those after them, and go on the text with them.
    held: Vec<u8>,
    /// For other encodings: it keeps the bytes of an unfinished character itself.
    decoder: Option<Decoder>,
    /// A CR at the end of the decoded text, which a LF may follow.
    held_cr: bool,
    /// How many looks in a row did not find the file.
    missing: u32,
}

impl Follower {
    /// Follows the file of `doc` from the end of what it read: the file may have grown since.
    /// `None` for a document without a file, and for a preview.
    pub fn for_document(doc: &Document) -> Option<Follower> {
        let (path, disk) = (doc.path.clone()?, doc.disk?);
        if doc.preview {
            return None;
        }
        // The end of the text as the file has it: UTF-8 is its bytes; another encoding gives them
        // back, unless bytes were lost in reading — where, is not known.
        let format = doc.format;
        let len = doc.text.len();
        let take = if format.encoding.is_utf8() { TAIL } else { TAIL / 2 };
        let start = doc.text.floor_char_boundary(len.saturating_sub(take), 0);
        let mut encoder = Encoder::new(format.encoding, false);
        let mut tail = Vec::new();
        encoder.encode(&doc.text.to_vec(start..len), true, &mut tail);
        let lost = !doc.decode_losses.is_empty();
        let tail = (!lost && encoder.losses().is_empty() && tail.len() as u64 <= disk.len).then_some(tail);
        let cut = lost && doc.text.to_vec(len.saturating_sub(3)..len) == "\u{FFFD}".as_bytes();
        let decoder = (!format.encoding.is_utf8()).then(|| Decoder::new(format.encoding));
        let held = Vec::new();
        Some(Follower { path, format, disk, read: disk.len, tail, cut, held, decoder, held_cr: false, missing: 0 })
    }

    /// Whether it follows the file `path` that a document has in `format` as `disk`: what it reads
    /// next goes on that document.
    pub fn follows(&self, path: &Path, format: Format, disk: Fingerprint) -> bool {
        self.path == path && self.format == format && self.disk == disk
    }

    /// Looks at the file, and reads what was added to it. Blocks: for a background thread.
    pub fn poll(&mut self) -> io::Result<Change> {
        // Not a file any more — a folder, or a pipe, which opening would wait on — it is as gone.
        // On Windows the standard library shares reading, writing and removing the file it opens:
        // the program that writes it goes on, and may rename or remove it.
        let opened = match fs::metadata(&self.path) {
            Ok(metadata) if !metadata.is_file() => return Ok(Change::Deleted),
            Ok(_) => File::open(&self.path),
            Err(error) => Err(error),
        };
        let mut file = match opened {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                self.missing += 1;
                return Ok(if self.missing < MISSING_LOOKS { Change::None } else { Change::Deleted });
            }
            Err(error) => return Err(error),
        };
        self.missing = 0;
        let now = Fingerprint::of_file(&file)?;
        if now == self.disk && self.tail.is_some() {
            return Ok(Change::None);
        }
        // Not to be read anew either.
        if now.len > MAX_LEN as u64 {
            return Ok(Change::TooLong);
        }
        if now.id != self.disk.id || now.len < self.read || (self.cut && now != self.disk) {
            return Ok(Change::Replaced);
        }
        // What was read is still there, or the file was written anew. Of a text that lost bytes in
        // reading, the file is taken at its word.
        let known = self.tail.as_ref().map_or(self.read.min(TAIL as u64), |tail| tail.len() as u64);
        let Some(found) = read_at(&mut file, self.read - known, known as usize)? else {
            return Ok(Change::Replaced);
        };
        if self.tail.as_ref().is_some_and(|tail| *tail != found) {
            return Ok(Change::Replaced);
        }
        let mut tail = found;
        if now == self.disk {
            self.tail = Some(tail);
            return Ok(Change::None);
        }
        let added = match usize::try_from(now.len - self.read) {
            Ok(added) if added <= MAX_LEN => added,
            _ => return Ok(Change::TooLong),
        };
        let Some(bytes) = read_at(&mut file, self.read, added)? else { return Ok(Change::Replaced) };
        let from = self.disk;
        self.read = now.len;
        self.disk = now;
        let kept = tail.len().min(TAIL.saturating_sub(bytes.len()));
        tail.drain(..tail.len() - kept);
        tail.extend_from_slice(&bytes[bytes.len().saturating_sub(TAIL)..]);
        self.tail = Some(tail);
        let (text, losses) = self.decode(&bytes);
        Ok(Change::Appended(Appended { from, disk: now, text, losses }))
    }

    /// The text of `bytes`, which follow those read before; the end of a character cut off, and a
    /// CR at the end, wait for the next bytes.
    fn decode(&mut self, bytes: &[u8]) -> (Vec<u8>, Losses) {
        match &mut self.decoder {
            // The text of UTF-8 is its bytes, invalid ones too.
            None => {
                let mut text = std::mem::take(&mut self.held);
                text.extend_from_slice(bytes);
                let mut held = unfinished_utf8(&text);
                if text[..text.len() - held].ends_with(b"\r") {
                    held += 1;
                }
                self.held = text.split_off(text.len() - held);
                (text, Losses::default())
            }
            Some(decoder) => {
                let mut text = String::new();
                let cr = std::mem::take(&mut self.held_cr);
                if cr {
                    text.push('\r');
                }
                decoder.decode(bytes, false, &mut text);
                let mut losses = decoder.take_losses();
                losses.first = losses.first.map(|first| first + usize::from(cr));
                if text.ends_with('\r') {
                    text.pop();
                    self.held_cr = true;
                }
                (text.into_bytes(), losses)
            }
        }
    }
}

/// How many bytes at the end of `text` start a UTF-8 character that the next bytes are to finish.
fn unfinished_utf8(text: &[u8]) -> usize {
    for back in 1..=text.len().min(3) {
        let byte = text[text.len() - back];
        if byte & 0xC0 == 0x80 {
            continue;
        }
        let len = match byte {
            0xC2..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF4 => 4,
            _ => 1,
        };
        return if len > back { back } else { 0 };
    }
    0
}

/// `len` bytes of `file` from `at`; `None` if it ends before.
fn read_at(file: &mut File, at: u64, len: usize) -> io::Result<Option<Vec<u8>>> {
    file.seek(SeekFrom::Start(at))?;
    let mut bytes = vec![0; len];
    match file.read_exact(&mut bytes) {
        Ok(()) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(None),
        Err(error) => Err(error),
    }
}

impl Document {
    /// Puts `appended`, the text added to the end of the file, on the end of the document: then it
    /// has the file as it is now. This is no edit: the history starts anew, as when the file is
    /// read again, and the document has nothing to save. Nothing goes on a document with changes of
    /// its own, or one that does not have the file as it was before the addition.
    pub fn append_from_file(&mut self, appended: &Appended) -> Result<(), EditError> {
        if self.preview {
            return Err(EditError::Preview);
        }
        if self.is_modified() || self.disk != Some(appended.from) {
            return Err(EditError::Changed);
        }
        let end = self.text.len();
        if end + appended.text.len() > MAX_LEN {
            return Err(EditError::TooLong);
        }
        self.disk = Some(appended.disk);
        self.large |= appended.disk.len >= LARGE_FILE;
        // Bytes that make no character yet change nothing that shows.
        if appended.text.is_empty() {
            return Ok(());
        }
        apply(&mut self.text, &mut self.lines, end..end, &appended.text);
        if let Some(first) = appended.losses.first {
            self.decode_losses.count += appended.losses.count;
            self.decode_losses.first.get_or_insert(end + first);
        }
        self.history = History::new();
        self.saved_at = Some(self.history.state());
        self.restart_journal();
        self.version = next_version();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::fs::{self, OpenOptions};
    use std::io::Write;

    use super::*;
    use crate::document::{OpenAs, Opened, open};
    use crate::encoding::Encoding;
    use crate::testing::TempDir;

    /// A file in its own directory, the document read from it, and its follower, which has looked
    /// at the file once.
    fn followed(name: &str, bytes: &[u8]) -> (TempDir, PathBuf, Document, Follower) {
        let dir = TempDir::new(name);
        let path = dir.0.join("log.txt");
        fs::write(&path, bytes).unwrap();
        let doc = match open(&path, OpenAs::Detect { tld: Some("ru") }).unwrap() {
            Opened::Complete(doc) => doc,
            Opened::Partial { loader, .. } => loader.load().unwrap(),
        };
        let mut follower = Follower::for_document(&doc).unwrap();
        assert!(matches!(follower.poll().unwrap(), Change::None));
        assert!(follower.follows(&path, doc.format, doc.disk.unwrap()));
        (dir, path, doc, follower)
    }

    fn add(path: &Path, bytes: &[u8]) {
        OpenOptions::new().append(true).open(path).unwrap().write_all(bytes).unwrap();
    }

    /// Looks at the file and puts what was added on the document.
    fn follow(follower: &mut Follower, doc: &mut Document) -> Option<Losses> {
        match follower.poll().unwrap() {
            Change::Appended(appended) => {
                doc.append_from_file(&appended).unwrap();
                Some(appended.losses)
            }
            Change::None => None,
            change => panic!("{change:?}"),
        }
    }

    fn text(doc: &Document) -> String {
        String::from_utf8(doc.text().to_vec(0..doc.text().len())).unwrap()
    }

    #[test]
    fn lines_added_go_on_the_end() {
        let (_dir, path, mut doc, mut follower) = followed("follow-lines", b"first\n");
        assert!(follow(&mut follower, &mut doc).is_none(), "nothing added");
        add(&path, b"second\nthird\n");
        follow(&mut follower, &mut doc);
        assert_eq!(text(&doc), "first\nsecond\nthird\n");
        assert_eq!(doc.lines().count(), 4);
        assert!(!doc.is_modified() && !doc.can_undo());
        assert_eq!(doc.disk, Some(Fingerprint::of_path(&path).unwrap()));
        assert!(follower.follows(&path, doc.format, doc.disk.unwrap()));
    }

    #[test]
    fn a_character_cut_off_waits_for_its_end() {
        let (_dir, path, mut doc, mut follower) = followed("follow-utf8", "старт\n".as_bytes());
        let added = "строка\n".as_bytes();
        add(&path, &added[..1]);
        follow(&mut follower, &mut doc);
        assert_eq!(text(&doc), "старт\n", "half of «с» waits");
        add(&path, &added[1..3]);
        follow(&mut follower, &mut doc);
        assert_eq!(text(&doc), "старт\nс");
        add(&path, &added[3..]);
        follow(&mut follower, &mut doc);
        assert_eq!(text(&doc), "старт\nстрока\n");
    }

    #[test]
    fn a_cr_waits_for_a_lf() {
        let (_dir, path, mut doc, mut follower) = followed("follow-crlf", b"a\r\n");
        add(&path, b"b\r");
        follow(&mut follower, &mut doc);
        assert_eq!(text(&doc), "a\r\nb");
        add(&path, b"\nc\r\n");
        follow(&mut follower, &mut doc);
        assert_eq!(text(&doc), "a\r\nb\r\nc\r\n");
        assert_eq!(doc.lines().count(), 4);
    }

    #[test]
    fn other_encodings_are_decoded_as_they_come() {
        // UTF-16 with a byte order mark: a code unit cut in two, a CR before its LF.
        let utf16 = |text: &str| text.encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<u8>>();
        let (_dir, path, mut doc, mut follower) =
            followed("follow-utf16", &[b"\xFF\xFE".as_slice(), &utf16("старт\r\n")].concat());
        assert_eq!(doc.format.encoding, Encoding::UTF_16LE);
        let added = utf16("строка\r\nещё");
        for part in [&added[..3], &added[3..14], &added[14..15], &added[15..]] {
            add(&path, part);
            follow(&mut follower, &mut doc);
        }
        assert_eq!(text(&doc), "старт\r\nстрока\r\nещё");
        assert_eq!(doc.lines().count(), 3);
        // windows-1251, one byte a letter.
        let cp1251 = |text: &str| encoding_rs::WINDOWS_1251.encode(text).0.into_owned();
        let start = "Журнал работы службы, запись первая.\n".repeat(20);
        let (_dir, path, mut doc, mut follower) = followed("follow-cp1251", &cp1251(&start));
        assert_eq!(doc.format.encoding.name(), "windows-1251");
        add(&path, &cp1251("Новая запись.\n"));
        follow(&mut follower, &mut doc);
        assert_eq!(text(&doc), start + "Новая запись.\n");
    }

    #[test]
    fn bytes_lost_in_decoding_are_counted_where_they_are() {
        let (_dir, path, mut doc, mut follower) = followed("follow-losses", b"\xFF\xFEa\0\n\0");
        // An unpaired surrogate.
        add(&path, b"b\0\x00\xD8c\0");
        let losses = follow(&mut follower, &mut doc).unwrap();
        assert_eq!(losses, Losses { count: 1, first: Some(1) });
        assert_eq!(doc.decode_losses(), Losses { count: 1, first: Some(3) });
        assert_eq!(text(&doc), "a\nb\u{FFFD}c");
    }

    #[test]
    fn a_text_with_bytes_lost_in_reading_is_followed_from_the_file() {
        // An unpaired surrogate inside: the end of the text is not the bytes of the file.
        let (_dir, path, mut doc, mut follower) = followed("follow-lossy", b"\xFF\xFEa\0\x00\xD8b\0\n\0");
        assert_eq!(doc.decode_losses().count, 1);
        add(&path, b"c\0\n\0");
        follow(&mut follower, &mut doc);
        assert_eq!(text(&doc), "a\u{FFFD}b\nc\n");
        add(&path, b"d\0");
        follow(&mut follower, &mut doc);
        assert_eq!(text(&doc), "a\u{FFFD}b\nc\nd");
    }

    #[test]
    fn a_character_cut_off_when_read_is_read_anew() {
        // Half of the code unit of «b» at the end: what is added finishes it.
        let (_dir, path, _doc, mut follower) = followed("follow-cut", b"\xFF\xFEa\0\n\0b");
        assert!(matches!(follower.poll().unwrap(), Change::None));
        add(&path, b"\0c\0");
        assert!(matches!(follower.poll().unwrap(), Change::Replaced));
    }

    #[test]
    fn a_folder_in_place_of_the_file_is_as_gone() {
        let (_dir, path, _doc, mut follower) = followed("follow-folder", b"one\n");
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(matches!(follower.poll().unwrap(), Change::Deleted));
    }

    #[test]
    fn a_file_cut_short_or_replaced_is_read_anew() {
        let (dir, path, _doc, mut follower) = followed("follow-replaced", b"one\ntwo\n");
        fs::write(&path, b"one\n").unwrap();
        assert!(matches!(follower.poll().unwrap(), Change::Replaced), "cut short");

        let (_dir, path, _doc, mut follower) = followed("follow-rotated", b"one\ntwo\n");
        let new = dir.0.join("new.txt");
        fs::write(&new, b"one\ntwo\nthree\n").unwrap();
        fs::rename(&new, &path).unwrap();
        assert!(matches!(follower.poll().unwrap(), Change::Replaced), "another file in its place");

        let (_dir, path, _doc, mut follower) = followed("follow-rewritten", b"one\ntwo\n");
        let file = OpenOptions::new().write(true).open(&path).unwrap();
        (&file).write_all(b"ONE\ntwo\nthree\n").unwrap();
        assert!(matches!(follower.poll().unwrap(), Change::Replaced), "written anew in place");

        let (_dir, path, _doc, mut follower) = followed("follow-deleted", b"one\n");
        fs::remove_file(&path).unwrap();
        for _ in 1..MISSING_LOOKS {
            assert!(matches!(follower.poll().unwrap(), Change::None), "maybe rotated");
        }
        assert!(matches!(follower.poll().unwrap(), Change::Deleted));
    }

    #[test]
    fn a_file_renamed_and_made_again_is_read_anew() {
        let (dir, path, _doc, mut follower) = followed("follow-rotated-later", b"one\ntwo\n");
        fs::rename(&path, dir.0.join("log.1")).unwrap();
        assert!(matches!(follower.poll().unwrap(), Change::None), "not there for a moment");
        fs::write(&path, b"three\n").unwrap();
        assert!(matches!(follower.poll().unwrap(), Change::Replaced));
    }

    #[test]
    fn what_was_added_before_following_is_read_too() {
        let (_dir, path, mut doc, _) = followed("follow-late", "один\n".as_bytes());
        add(&path, "два\n".as_bytes());
        let mut follower = Follower::for_document(&doc).unwrap();
        follow(&mut follower, &mut doc);
        assert_eq!(text(&doc), "один\nдва\n");
        // In another encoding, the end of the text is encoded to be compared.
        let cp1251 = |text: &str| encoding_rs::WINDOWS_1251.encode(text).0.into_owned();
        let start = "Журнал работы службы, запись первая.\n".repeat(20);
        let (_dir, path, mut doc, _) = followed("follow-late-cp1251", &cp1251(&start));
        add(&path, &cp1251("Ещё.\n"));
        let mut follower = Follower::for_document(&doc).unwrap();
        follow(&mut follower, &mut doc);
        assert_eq!(text(&doc), start + "Ещё.\n");
    }

    #[test]
    fn a_file_written_anew_before_following_is_read_anew() {
        let (_dir, path, doc, _) = followed("follow-late-rewritten", b"one\n");
        let file = OpenOptions::new().write(true).open(&path).unwrap();
        (&file).write_all(b"ONE\ntwo\n").unwrap();
        let mut follower = Follower::for_document(&doc).unwrap();
        assert!(matches!(follower.poll().unwrap(), Change::Replaced));
    }

    #[test]
    fn a_cr_read_at_the_end_meets_its_lf() {
        let (_dir, path, mut doc, mut follower) = followed("follow-late-cr", b"a\r");
        add(&path, b"\nb\r\n");
        follow(&mut follower, &mut doc);
        assert_eq!(text(&doc), "a\r\nb\r\n");
        assert_eq!(doc.lines().count(), 3);
    }

    #[test]
    fn a_time_changed_alone_is_nothing() {
        let (_dir, path, mut doc, mut follower) = followed("follow-touched", b"one\n");
        let long_ago = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
        File::options().write(true).open(&path).unwrap().set_modified(long_ago).unwrap();
        // The document has the file as it is now.
        follow(&mut follower, &mut doc);
        assert_eq!(doc.disk, Some(Fingerprint::of_path(&path).unwrap()));
        assert_eq!(text(&doc), "one\n");
    }

    #[test]
    fn nothing_goes_on_a_document_with_changes_or_read_from_another_file() {
        use std::time::Instant;

        use crate::history::{EditKind, Selection};

        let (_dir, path, mut doc, mut follower) = followed("follow-changed", b"one\n");
        doc.edit(&[(0..0, b"x")], Selection::default(), Selection::caret(1), EditKind::Typing, Instant::now()).unwrap();
        add(&path, b"two\n");
        let Change::Appended(appended) = follower.poll().unwrap() else { panic!("added") };
        assert_eq!(doc.append_from_file(&appended), Err(EditError::Changed));
        doc.undo();
        let stale = Appended { from: appended.disk, ..appended };
        assert_eq!(doc.append_from_file(&stale), Err(EditError::Changed));
        assert_eq!(text(&doc), "one\n");
    }

    #[test]
    fn a_read_only_document_takes_no_edits() {
        use std::time::Instant;

        use crate::history::{EditKind, Selection};

        let (_dir, path, mut doc, mut follower) = followed("follow-read-only", b"one\n");
        doc.edit(&[(0..0, b"x")], Selection::default(), Selection::caret(1), EditKind::Typing, Instant::now()).unwrap();
        doc.set_read_only(true);
        assert!(!doc.can_undo() && doc.undo().is_none());
        let typed =
            doc.edit(&[(0..0, b"y")], Selection::default(), Selection::caret(1), EditKind::Typing, Instant::now());
        assert_eq!(typed, Err(EditError::ReadOnly));
        assert_eq!(text(&doc), "xone\n");
        // What is added still goes on a document without changes.
        doc.set_read_only(false);
        doc.undo();
        doc.set_read_only(true);
        add(&path, b"two\n");
        follow(&mut follower, &mut doc);
        assert_eq!(text(&doc), "one\ntwo\n");
    }

    #[test]
    fn utf8_characters_cut_off_are_told() {
        assert_eq!(unfinished_utf8(b"abc"), 0);
        assert_eq!(unfinished_utf8("aж".as_bytes()), 0);
        assert_eq!(unfinished_utf8(&"aж".as_bytes()[..2]), 1);
        assert_eq!(unfinished_utf8(&"a€".as_bytes()[..3]), 2);
        assert_eq!(unfinished_utf8(&"a😀".as_bytes()[..4]), 3);
        assert_eq!(unfinished_utf8("a😀".as_bytes()), 0);
        // Invalid bytes are text as they are.
        assert_eq!(unfinished_utf8(b"a\xFF"), 0);
        assert_eq!(unfinished_utf8(b"\x80\x80\x80"), 0);
    }
}
