//! The edit journal of a document: every edit, undo and redo appended to a file as one record,
//! so that the text and its undo history survive quitting and crashes.
//!
//! Format version 1; numbers are little-endian.
//!
//! - Header: `MPJ`, the version byte, the document id (16 bytes), the path.
//! - Record: a kind byte, the payload length (`u64`), the payload, and a checksum (`u32`) of all
//!   three. A record that is cut off or fails its checksum ends the journal: it is the trace of
//!   a crash in the middle of a write.
//!
//! Kinds of records:
//!
//! - `B`, base: what the following records start from — the path, the format, the fingerprint
//!   of the file on disk if there is a file, and a snapshot of the text if one was taken; without
//!   a snapshot, the text of the base is the file;
//! - `T`, transaction: its kind, whether it joined the last undo step, the selections before and
//!   after, and the edits;
//! - `U`, undo; `R`, redo;
//! - `F`, format: the format changed without saving.
//!
//! Records before the last base only rebuild the undo history, since the base contains their
//! edits already; records after it are applied to the text of the base as well.

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use super::{Document, DocumentId, Fingerprint, Format, Text, apply};
use crate::encoding::{Encoding, Losses};
use crate::history::{Edit, EditKind, History, Selection, Transaction};
use crate::line_ending::LineEnding;
use crate::text::{GapBuffer, Indexer, TextStore};

const MAGIC: &[u8; 3] = b"MPJ";
const VERSION: u8 = 1;

const BASE: u8 = b'B';
const TRANSACTION: u8 = b'T';
const UNDO: u8 = b'U';
const REDO: u8 = b'R';
const FORMAT: u8 = b'F';

/// A journal file open for appending records.
#[derive(Debug)]
pub struct Journal {
    file: File,
    size: u64,
}

impl Journal {
    /// Creates the journal file of document `id`, replacing any file at `file_path`, and writes
    /// the header and the first base.
    pub fn create(file_path: &Path, id: DocumentId, path: Option<&Path>, base: BaseRef) -> io::Result<Journal> {
        let mut header = Vec::new();
        header.extend_from_slice(MAGIC);
        header.push(VERSION);
        header.extend_from_slice(&id.0);
        put_path(&mut header, path);
        let mut file = OpenOptions::new().write(true).create(true).truncate(true).open(file_path)?;
        file.write_all(&header)?;
        drop(file);
        let mut journal = Journal::resume(file_path, header.len() as u64)?;
        journal.write_base(base)?;
        Ok(journal)
    }

    /// Continues the journal at `file_path` after its first `valid_len` bytes — the whole journal
    /// as [`read`] found it, without a torn last record.
    pub fn resume(file_path: &Path, valid_len: u64) -> io::Result<Journal> {
        let file = OpenOptions::new().write(true).open(file_path)?;
        file.set_len(valid_len)?;
        drop(file);
        let file = OpenOptions::new().append(true).open(file_path)?;
        Ok(Journal { file, size: valid_len })
    }

    pub fn write_base(&mut self, base: BaseRef) -> io::Result<()> {
        let mut payload = Vec::with_capacity(64 + base.snapshot.map_or(0, |text| text.len()));
        put_path(&mut payload, base.path);
        put_format(&mut payload, base.format);
        put_fingerprint(&mut payload, base.disk);
        put_u64(&mut payload, base.decode_losses.count as u64);
        put_u64(&mut payload, base.decode_losses.first.map_or(u64::MAX, |pos| pos as u64));
        match base.snapshot {
            Some(text) => {
                payload.push(1);
                put_u64(&mut payload, text.len() as u64);
                text.copy_to(0..text.len(), &mut payload);
            }
            None => payload.push(0),
        }
        self.write_record(BASE, &payload)
    }

    pub fn write_transaction(&mut self, transaction: &Transaction, kind: EditKind, merged: bool) -> io::Result<()> {
        let size: usize = transaction.edits.iter().map(|e| 24 + e.deleted.len() + e.inserted.len()).sum();
        let mut payload = Vec::with_capacity(42 + size);
        payload.push(match kind {
            EditKind::Typing => 0,
            EditKind::Deleting => 1,
            EditKind::Other => 2,
        });
        payload.push(merged as u8);
        put_selection(&mut payload, transaction.before);
        put_selection(&mut payload, transaction.after);
        put_u64(&mut payload, transaction.edits.len() as u64);
        for edit in &transaction.edits {
            put_u64(&mut payload, edit.pos as u64);
            put_bytes(&mut payload, &edit.deleted);
            put_bytes(&mut payload, &edit.inserted);
        }
        self.write_record(TRANSACTION, &payload)
    }

    pub fn write_undo(&mut self) -> io::Result<()> {
        self.write_record(UNDO, &[])
    }

    pub fn write_redo(&mut self) -> io::Result<()> {
        self.write_record(REDO, &[])
    }

    pub fn write_format(&mut self, format: Format) -> io::Result<()> {
        let mut payload = Vec::new();
        put_format(&mut payload, format);
        self.write_record(FORMAT, &payload)
    }

    /// Flushes the journal to the disk, so that it survives a power loss as well. Writes alone
    /// survive the process being killed.
    pub fn sync(&self) -> io::Result<()> {
        self.file.sync_data()
    }

    /// Length of the journal file in bytes.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// One record, one `write`.
    fn write_record(&mut self, kind: u8, payload: &[u8]) -> io::Result<()> {
        let mut record = Vec::with_capacity(payload.len() + 13);
        record.push(kind);
        put_u64(&mut record, payload.len() as u64);
        record.extend_from_slice(payload);
        let sum = checksum(&record);
        record.extend_from_slice(&sum.to_le_bytes());
        self.file.write_all(&record)?;
        self.size += record.len() as u64;
        Ok(())
    }
}

/// A base to write: the snapshot is borrowed from the document.
#[derive(Clone, Copy, Debug)]
pub struct BaseRef<'a> {
    pub path: Option<&'a Path>,
    pub format: Format,
    pub disk: Option<Fingerprint>,
    /// Saving must still warn about bytes the snapshot lost in decoding.
    pub decode_losses: Losses,
    pub snapshot: Option<&'a Text>,
}

/// A record read from a journal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Record {
    Base {
        path: Option<PathBuf>,
        format: Format,
        disk: Option<Fingerprint>,
        decode_losses: Losses,
        snapshot: Option<Vec<u8>>,
    },
    Transaction {
        transaction: Transaction,
        kind: EditKind,
        merged: bool,
    },
    Undo,
    Redo,
    Format(Format),
}

/// The intact part of a journal file.
#[derive(Clone, Debug)]
pub struct Contents {
    pub id: DocumentId,
    pub path: Option<PathBuf>,
    pub records: Vec<Record>,
    /// Length of the header and the intact records; [`Journal::resume`] continues from here.
    pub valid_len: u64,
}

/// Why a journal file cannot be read.
#[derive(Debug)]
pub enum ReadError {
    Io(io::Error),
    /// The file has no complete header of a supported version.
    NotAJournal,
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReadError::Io(error) => error.fmt(f),
            ReadError::NotAJournal => f.write_str("not a journal of a supported version"),
        }
    }
}

impl std::error::Error for ReadError {}

/// Reads the journal at `file_path` up to the first torn or damaged record.
pub fn read(file_path: &Path) -> Result<Contents, ReadError> {
    let file = File::open(file_path).map_err(ReadError::Io)?;
    let file_len = file.metadata().map_err(ReadError::Io)?.len();
    let mut reader = BufReader::with_capacity(1 << 20, file);
    let mut head = [0u8; 4 + 16 + 8];
    if !read_exact_or_eof(&mut reader, &mut head).map_err(ReadError::Io)? || &head[..3] != MAGIC || head[3] != VERSION {
        return Err(ReadError::NotAJournal);
    }
    let id = DocumentId(head[4..20].try_into().unwrap());
    let path_len = u64::from_le_bytes(head[20..28].try_into().unwrap());
    let mut valid_len = head.len() as u64;
    let path = if path_len == u64::MAX {
        None
    } else {
        if path_len > file_len {
            return Err(ReadError::NotAJournal);
        }
        let mut bytes = vec![0u8; path_len as usize];
        if !read_exact_or_eof(&mut reader, &mut bytes).map_err(ReadError::Io)? {
            return Err(ReadError::NotAJournal);
        }
        valid_len += path_len;
        Some(Cursor::new(&bytes).path_bytes(bytes.len()).ok_or(ReadError::NotAJournal)?)
    };
    let mut records = Vec::new();
    loop {
        let mut prefix = [0u8; 9];
        if !read_exact_or_eof(&mut reader, &mut prefix).map_err(ReadError::Io)? {
            break;
        }
        let len = u64::from_le_bytes(prefix[1..9].try_into().unwrap());
        if len > file_len - valid_len {
            break;
        }
        let mut rest = vec![0u8; len as usize + 4];
        if !read_exact_or_eof(&mut reader, &mut rest).map_err(ReadError::Io)? {
            break;
        }
        let (payload, sum) = rest.split_at(len as usize);
        let mut whole = Vec::with_capacity(9 + payload.len());
        whole.extend_from_slice(&prefix);
        whole.extend_from_slice(payload);
        if checksum(&whole).to_le_bytes() != sum {
            break;
        }
        let Some(record) = decode(prefix[0], payload) else { break };
        records.push(record);
        valid_len += 9 + len + 4;
    }
    Ok(Contents { id, path, records, valid_len })
}

/// Why a document cannot be rebuilt from its journal.
#[derive(Debug, PartialEq, Eq)]
pub enum ReplayError {
    /// The journal has no base.
    NoBase,
    /// The base is the file on disk, but no text of it was given.
    NoFileText,
    /// An edit does not fit the text, or the history cannot do an undo or redo: the base is not
    /// the text the journal started from.
    Mismatch,
}

impl fmt::Display for ReplayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ReplayError::NoBase => "the journal has no base",
            ReplayError::NoFileText => "the text of the file is needed",
            ReplayError::Mismatch => "the journal does not match its base",
        })
    }
}

impl std::error::Error for ReplayError {}

impl Document {
    /// Rebuilds the document from its journal. When the last base has no snapshot, its text is
    /// the file on disk: the caller loads it into `file` (its fingerprint may differ from the one
    /// in the base — whether that matters is for the caller to decide).
    pub fn replay(contents: &Contents, file: Option<Document>) -> Result<Document, ReplayError> {
        let last_base =
            contents.records.iter().rposition(|r| matches!(r, Record::Base { .. })).ok_or(ReplayError::NoBase)?;
        let Record::Base { path, format, disk, decode_losses, snapshot } = &contents.records[last_base] else {
            unreachable!()
        };
        let mut doc = match snapshot {
            Some(bytes) => {
                let text = GapBuffer::from_vec(bytes.clone());
                let lines = Indexer::index(&text).lines;
                Document { text, lines, decode_losses: *decode_losses, ..Document::new() }
            }
            None => file.ok_or(ReplayError::NoFileText)?,
        };
        doc.path = path.clone();
        doc.format = *format;
        doc.disk = *disk;
        // The records before the base: history only.
        let mut history = History::new();
        for record in &contents.records[..last_base] {
            let done = match record {
                Record::Transaction { transaction, kind, merged } => {
                    history.replay(transaction.clone(), *kind, *merged)
                }
                Record::Undo => history.undo().is_some(),
                Record::Redo => history.redo().is_some(),
                Record::Base { .. } | Record::Format(_) => true,
            };
            if !done {
                return Err(ReplayError::Mismatch);
            }
        }
        doc.history = history;
        // The records after it: the text as well.
        for record in &contents.records[last_base + 1..] {
            match record {
                Record::Transaction { transaction, kind, merged } => {
                    doc.replay_transaction(transaction, *kind, *merged)?
                }
                Record::Undo => {
                    doc.undo().ok_or(ReplayError::Mismatch)?;
                }
                Record::Redo => {
                    doc.redo().ok_or(ReplayError::Mismatch)?;
                }
                Record::Format(format) => doc.format = *format,
                Record::Base { .. } => unreachable!("the last base"),
            }
        }
        doc.history.seal();
        Ok(doc)
    }

    /// Applies a transaction from the journal after checking that its edits fit the text.
    fn replay_transaction(
        &mut self,
        transaction: &Transaction,
        kind: EditKind,
        merged: bool,
    ) -> Result<(), ReplayError> {
        let mut len = self.text.len();
        for edit in &transaction.edits {
            let end = edit.pos.checked_add(edit.deleted.len()).ok_or(ReplayError::Mismatch)?;
            if end > len {
                return Err(ReplayError::Mismatch);
            }
            len = len - edit.deleted.len() + edit.inserted.len();
        }
        for (i, edit) in transaction.edits.iter().enumerate() {
            let range = edit.pos..edit.pos + edit.deleted.len();
            if self.text.to_vec(range.clone()) != edit.deleted {
                // Undo the edits applied so far, so the document stays as it was.
                for done in transaction.edits[..i].iter().rev() {
                    apply(&mut self.text, &mut self.lines, done.pos..done.pos + done.inserted.len(), &done.deleted);
                }
                return Err(ReplayError::Mismatch);
            }
            apply(&mut self.text, &mut self.lines, range, &edit.inserted);
        }
        if !self.history.replay(transaction.clone(), kind, merged) {
            return Err(ReplayError::Mismatch);
        }
        Ok(())
    }
}

fn decode(kind: u8, payload: &[u8]) -> Option<Record> {
    let mut cursor = Cursor::new(payload);
    let record = match kind {
        BASE => {
            let path = cursor.path()?;
            let format = cursor.format()?;
            let disk = cursor.fingerprint()?;
            let count = usize::try_from(cursor.u64()?).ok()?;
            let first = match cursor.u64()? {
                u64::MAX => None,
                pos => Some(usize::try_from(pos).ok()?),
            };
            let snapshot = match cursor.u8()? {
                0 => None,
                1 => Some(cursor.bytes()?.to_vec()),
                _ => return None,
            };
            Record::Base { path, format, disk, decode_losses: Losses { count, first }, snapshot }
        }
        TRANSACTION => {
            let kind = match cursor.u8()? {
                0 => EditKind::Typing,
                1 => EditKind::Deleting,
                2 => EditKind::Other,
                _ => return None,
            };
            let merged = match cursor.u8()? {
                0 => false,
                1 => true,
                _ => return None,
            };
            let before = cursor.selection()?;
            let after = cursor.selection()?;
            let count = cursor.u64()?;
            // Each edit takes at least 24 bytes: no allocation larger than the payload.
            if count > payload.len() as u64 / 24 {
                return None;
            }
            let mut edits = Vec::with_capacity(count as usize);
            for _ in 0..count {
                let pos = usize::try_from(cursor.u64()?).ok()?;
                let deleted = cursor.bytes()?.to_vec();
                let inserted = cursor.bytes()?.to_vec();
                edits.push(Edit { pos, deleted, inserted });
            }
            Record::Transaction { transaction: Transaction { edits, before, after }, kind, merged }
        }
        UNDO => Record::Undo,
        REDO => Record::Redo,
        FORMAT => Record::Format(cursor.format()?),
        _ => return None,
    };
    cursor.is_done().then_some(record)
}

/// FNV-1a over 64-bit words, folded to 32 bits: catches torn and damaged records cheaply.
fn checksum(bytes: &[u8]) -> u32 {
    const PRIME: u64 = 0x100_0000_01b3;
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let (words, rest) = bytes.as_chunks::<8>();
    for &word in words {
        hash = (hash ^ u64::from_le_bytes(word)).wrapping_mul(PRIME);
    }
    for &byte in rest {
        hash = (hash ^ u64::from(byte)).wrapping_mul(PRIME);
    }
    (hash ^ (hash >> 32)) as u32
}

/// Fills `buf` from `reader`; `false` if the input ends first.
fn read_exact_or_eof(reader: &mut impl Read, buf: &mut [u8]) -> io::Result<bool> {
    match reader.read_exact(buf) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(false),
        Err(error) => Err(error),
    }
}

fn put_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    put_u64(out, bytes.len() as u64);
    out.extend_from_slice(bytes);
}

fn put_selection(out: &mut Vec<u8>, selection: Selection) {
    put_u64(out, selection.anchor as u64);
    put_u64(out, selection.head as u64);
}

fn put_format(out: &mut Vec<u8>, format: Format) {
    put_bytes(out, format.encoding.name().as_bytes());
    out.push(format.bom as u8);
    out.push(match format.line_ending {
        LineEnding::Lf => 0,
        LineEnding::CrLf => 1,
        LineEnding::Cr => 2,
    });
}

fn put_fingerprint(out: &mut Vec<u8>, disk: Option<Fingerprint>) {
    let Some(disk) = disk else {
        out.push(0);
        return;
    };
    out.push(1);
    put_u64(out, disk.len);
    match disk.modified {
        Some(time) => {
            out.push(1);
            let (secs, nanos) = match time.duration_since(UNIX_EPOCH) {
                Ok(after) => (after.as_secs() as i64, after.subsec_nanos()),
                Err(error) => {
                    let before = error.duration();
                    match before.subsec_nanos() {
                        0 => (-(before.as_secs() as i64), 0),
                        n => (-(before.as_secs() as i64) - 1, 1_000_000_000 - n),
                    }
                }
            };
            put_u64(out, secs as u64);
            out.extend_from_slice(&nanos.to_le_bytes());
        }
        None => out.push(0),
    }
    match disk.id {
        Some((device, inode)) => {
            out.push(1);
            put_u64(out, device);
            put_u64(out, inode);
        }
        None => out.push(0),
    }
}

/// Paths are stored as the bytes of the OS: bytes on Unix, UTF-16 units on Windows.
fn put_path(out: &mut Vec<u8>, path: Option<&Path>) {
    let Some(path) = path else {
        put_u64(out, u64::MAX);
        return;
    };
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        put_bytes(out, path.as_os_str().as_bytes());
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let units: Vec<u8> = path.as_os_str().encode_wide().flat_map(u16::to_le_bytes).collect();
        put_bytes(out, &units);
    }
}

/// Reads the payload of a record; every read returns `None` past its end.
struct Cursor<'a> {
    bytes: &'a [u8],
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Cursor { bytes }
    }

    fn is_done(&self) -> bool {
        self.bytes.is_empty()
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if n > self.bytes.len() {
            return None;
        }
        let (head, rest) = self.bytes.split_at(n);
        self.bytes = rest;
        Some(head)
    }

    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn bytes(&mut self) -> Option<&'a [u8]> {
        let len = usize::try_from(self.u64()?).ok()?;
        self.take(len)
    }

    fn selection(&mut self) -> Option<Selection> {
        let anchor = usize::try_from(self.u64()?).ok()?;
        let head = usize::try_from(self.u64()?).ok()?;
        Some(Selection { anchor, head })
    }

    fn format(&mut self) -> Option<Format> {
        let encoding = Encoding::for_name(std::str::from_utf8(self.bytes()?).ok()?)?;
        let bom = match self.u8()? {
            0 => false,
            1 => true,
            _ => return None,
        };
        let line_ending = match self.u8()? {
            0 => LineEnding::Lf,
            1 => LineEnding::CrLf,
            2 => LineEnding::Cr,
            _ => return None,
        };
        Some(Format { encoding, bom, line_ending })
    }

    fn fingerprint(&mut self) -> Option<Option<Fingerprint>> {
        if self.u8()? == 0 {
            return Some(None);
        }
        let len = self.u64()?;
        let modified = match self.u8()? {
            0 => None,
            1 => {
                let secs = self.u64()? as i64;
                let nanos = self.u32()?;
                if nanos >= 1_000_000_000 {
                    return None;
                }
                let time = if secs >= 0 {
                    UNIX_EPOCH.checked_add(Duration::new(secs as u64, nanos))?
                } else {
                    UNIX_EPOCH
                        .checked_sub(Duration::from_secs(secs.unsigned_abs()))?
                        .checked_add(Duration::from_nanos(nanos.into()))?
                };
                Some(time)
            }
            _ => return None,
        };
        let id = match self.u8()? {
            0 => None,
            1 => Some((self.u64()?, self.u64()?)),
            _ => return None,
        };
        Some(Some(Fingerprint { len, modified, id }))
    }

    fn path(&mut self) -> Option<Option<PathBuf>> {
        let len = self.u64()?;
        if len == u64::MAX {
            return Some(None);
        }
        self.path_bytes(usize::try_from(len).ok()?).map(Some)
    }

    /// A path of `len` bytes; the length itself was read already.
    fn path_bytes(&mut self, len: usize) -> Option<PathBuf> {
        let bytes = self.take(len)?;
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            Some(PathBuf::from(std::ffi::OsStr::from_bytes(bytes)))
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStringExt;
            let (pairs, rest) = bytes.as_chunks::<2>();
            if !rest.is_empty() {
                return None;
            }
            let units: Vec<u16> = pairs.iter().map(|&pair| u16::from_le_bytes(pair)).collect();
            Some(PathBuf::from(std::ffi::OsString::from_wide(&units)))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{Instant, SystemTime};

    use super::*;
    use crate::testing::TempFile;

    fn text(doc: &Document) -> Vec<u8> {
        doc.text().to_vec(0..doc.text().len())
    }

    fn transaction(pos: usize, deleted: &str, inserted: &str, kind: EditKind, merged: bool) -> Record {
        let edit = Edit { pos, deleted: deleted.into(), inserted: inserted.into() };
        let before = Selection::caret(pos + deleted.len());
        let after = Selection::caret(pos + inserted.len());
        Record::Transaction { transaction: Transaction { edits: vec![edit], before, after }, kind, merged }
    }

    fn snapshot_base(text: &str) -> Record {
        Record::Base {
            path: None,
            format: Format::default(),
            disk: None,
            decode_losses: Losses::default(),
            snapshot: Some(text.into()),
        }
    }

    /// Writes `records` to a new journal file; the first one must be a base.
    fn write(name: &str, path: Option<&Path>, records: &[Record]) -> (TempFile, DocumentId) {
        let file = TempFile::new(name, b"");
        let id = DocumentId::random();
        let mut journal: Option<Journal> = None;
        for record in records {
            match record {
                Record::Base { path: base_path, format, disk, decode_losses, snapshot } => {
                    let snapshot = snapshot.clone().map(GapBuffer::from_vec);
                    let base = BaseRef {
                        path: base_path.as_deref(),
                        format: *format,
                        disk: *disk,
                        decode_losses: *decode_losses,
                        snapshot: snapshot.as_ref(),
                    };
                    match &mut journal {
                        Some(journal) => journal.write_base(base).unwrap(),
                        None => journal = Some(Journal::create(&file.0, id, path, base).unwrap()),
                    }
                }
                Record::Transaction { transaction, kind, merged } => {
                    journal.as_mut().unwrap().write_transaction(transaction, *kind, *merged).unwrap()
                }
                Record::Undo => journal.as_mut().unwrap().write_undo().unwrap(),
                Record::Redo => journal.as_mut().unwrap().write_redo().unwrap(),
                Record::Format(format) => journal.as_mut().unwrap().write_format(*format).unwrap(),
            }
        }
        (file, id)
    }

    fn typing_session() -> Vec<Record> {
        vec![
            snapshot_base(""),
            transaction(0, "", "a", EditKind::Typing, false),
            transaction(1, "", "b", EditKind::Typing, true),
            transaction(2, "", "\n", EditKind::Typing, false),
            transaction(3, "", "c", EditKind::Other, false),
            Record::Undo,
            Record::Undo,
            Record::Redo,
            Record::Format(Format { line_ending: LineEnding::CrLf, ..Format::default() }),
        ]
    }

    #[test]
    fn records_read_back_as_written() {
        let records = typing_session();
        let path = Path::new("/tmp/Документ.txt");
        let (file, id) = write("journal-round-trip", Some(path), &records);
        let contents = read(&file.0).unwrap();
        assert_eq!((contents.id, contents.path.as_deref()), (id, Some(path)));
        assert_eq!(contents.records, records);
        assert_eq!(contents.valid_len, fs::metadata(&file.0).unwrap().len());
    }

    #[test]
    fn replay_rebuilds_the_text_and_the_history() {
        let (file, _) = write("journal-replay", None, &typing_session());
        let mut doc = Document::replay(&read(&file.0).unwrap(), None).unwrap();
        assert_eq!(text(&doc), b"ab\n");
        assert_eq!(doc.format.line_ending, LineEnding::CrLf);
        assert_eq!(doc.redo(), Some(Selection::caret(4)));
        assert_eq!(text(&doc), b"ab\nc");
        doc.undo();
        doc.undo();
        assert_eq!(text(&doc), b"ab");
        assert_eq!(doc.undo(), Some(Selection::caret(0)), "typed \"ab\" is one step");
        assert_eq!((text(&doc), doc.can_undo()), (Vec::new(), false));
    }

    #[test]
    fn the_history_continues_across_bases() {
        let fingerprint = Fingerprint { len: 11, modified: Some(SystemTime::now()), id: Some((1, 2)) };
        let records = vec![
            snapshot_base("hello"),
            transaction(5, "", " world", EditKind::Other, false),
            // Saved: the file on disk is the base now.
            Record::Base {
                path: Some("/tmp/hello.txt".into()),
                format: Format::default(),
                disk: Some(fingerprint),
                decode_losses: Losses::default(),
                snapshot: None,
            },
            transaction(11, "", "!", EditKind::Other, false),
        ];
        let (file, _) = write("journal-bases", None, &records);
        let contents = read(&file.0).unwrap();
        assert_eq!(Document::replay(&contents, None).unwrap_err(), ReplayError::NoFileText);
        let mut on_disk = Document::new();
        on_disk
            .edit(
                &[(0..0, b"hello world")],
                Selection::default(),
                Selection::default(),
                EditKind::Other,
                Instant::now(),
            )
            .unwrap();
        let mut doc = Document::replay(&contents, Some(on_disk)).unwrap();
        assert_eq!(text(&doc), b"hello world!");
        assert_eq!((doc.path.as_deref(), doc.disk), (Some(Path::new("/tmp/hello.txt")), Some(fingerprint)));
        doc.undo();
        doc.undo();
        assert_eq!(text(&doc), b"hello", "the step before the save is undone too");
        assert!(!doc.can_undo());
    }

    #[test]
    fn a_cut_off_journal_keeps_every_whole_record() {
        let records = typing_session();
        let (file, _) = write("journal-torn", None, &records);
        let bytes = fs::read(&file.0).unwrap();
        let cut = TempFile::new("journal-torn-cut", b"");
        let mut lengths = Vec::new();
        for len in 0..=bytes.len() {
            fs::write(&cut.0, &bytes[..len]).unwrap();
            match read(&cut.0) {
                Err(ReadError::NotAJournal) => assert!(lengths.is_empty(), "only a cut header fails, at {len}"),
                Err(error) => panic!("{error}"),
                Ok(contents) => {
                    assert_eq!(contents.records, records[..contents.records.len()], "at {len}");
                    assert!(contents.valid_len <= len as u64);
                    lengths.push(contents.records.len());
                }
            }
        }
        // Each record appears once its last byte is written.
        assert!(lengths.windows(2).all(|w| w[0] <= w[1]));
        assert_eq!(lengths.last(), Some(&records.len()));
    }

    #[test]
    fn a_damaged_record_ends_the_journal() {
        let records = typing_session();
        let (file, _) = write("journal-damaged", None, &records);
        let whole = read(&file.0).unwrap();
        let mut bytes = fs::read(&file.0).unwrap();
        // The journal ends with a redo record (kind, length, checksum) and a format record
        // (the same around the encoding name, BOM and line ending).
        let format_len = 1 + 8 + (8 + "UTF-8".len() + 2) + 4;
        let redo_start = bytes.len() - format_len - (1 + 8 + 4);
        bytes[redo_start + 3] ^= 0x40; // the length of the redo record
        fs::write(&file.0, &bytes).unwrap();
        let damaged = read(&file.0).unwrap();
        assert_eq!(damaged.records, whole.records[..whole.records.len() - 2]);
    }

    #[test]
    fn a_resumed_journal_drops_the_torn_tail() {
        let records = typing_session();
        let (file, _) = write("journal-resume", None, &records);
        let mut bytes = fs::read(&file.0).unwrap();
        bytes.extend_from_slice(b"T\x40\0\0\0\0\0\0\0partial");
        fs::write(&file.0, &bytes).unwrap();
        let contents = read(&file.0).unwrap();
        assert_eq!(contents.records.len(), records.len());
        let mut journal = Journal::resume(&file.0, contents.valid_len).unwrap();
        journal.write_undo().unwrap();
        let resumed = read(&file.0).unwrap();
        assert_eq!(resumed.records[..records.len()], records[..]);
        assert_eq!(resumed.records.last(), Some(&Record::Undo));
        assert_eq!(resumed.valid_len, journal.size());
    }

    #[test]
    fn other_files_are_not_journals() {
        let (file, _) = write("journal-header", None, &[snapshot_base("x")]);
        let mut bytes = fs::read(&file.0).unwrap();
        for bad in [b"".to_vec(), b"MPJ".to_vec(), b"XYZ\x01".to_vec(), [b"MPJ\x02".as_slice(), &bytes[4..]].concat()] {
            fs::write(&file.0, bad).unwrap();
            assert!(matches!(read(&file.0), Err(ReadError::NotAJournal)));
        }
        bytes.truncate(20);
        fs::write(&file.0, bytes).unwrap();
        assert!(matches!(read(&file.0), Err(ReadError::NotAJournal)));
    }

    #[test]
    fn edits_that_do_not_fit_the_base_are_a_mismatch() {
        for records in [
            vec![snapshot_base("abc"), transaction(0, "x", "", EditKind::Deleting, false)],
            vec![snapshot_base("abc"), transaction(2, "cd", "", EditKind::Deleting, false)],
            vec![snapshot_base("abc"), Record::Undo],
            vec![snapshot_base("abc"), transaction(3, "", "d", EditKind::Typing, true)],
        ] {
            let (file, _) = write("journal-mismatch", None, &records);
            assert_eq!(Document::replay(&read(&file.0).unwrap(), None).unwrap_err(), ReplayError::Mismatch);
        }
    }

    #[test]
    fn bases_keep_fingerprints_and_decoding_losses() {
        let before_epoch = UNIX_EPOCH - Duration::new(5, 300);
        for modified in [Some(SystemTime::now()), Some(before_epoch), None] {
            let base = Record::Base {
                path: Some("/tmp/a.txt".into()),
                format: Format {
                    encoding: Encoding::for_name("KOI8-R").unwrap(),
                    bom: false,
                    line_ending: LineEnding::Cr,
                },
                disk: Some(Fingerprint { len: 42, modified, id: None }),
                decode_losses: Losses { count: 3, first: Some(5) },
                snapshot: Some("abcdef".into()),
            };
            let (file, _) = write("journal-base-fields", None, std::slice::from_ref(&base));
            let contents = read(&file.0).unwrap();
            assert_eq!(contents.records, [base]);
            let doc = Document::replay(&contents, None).unwrap();
            assert_eq!(doc.decode_losses(), Losses { count: 3, first: Some(5) });
        }
    }

    #[test]
    fn document_ids_are_random_and_print_as_hex() {
        let (a, b) = (DocumentId::random(), DocumentId::random());
        assert_ne!(a, b);
        let hex = a.to_string();
        assert_eq!(hex.len(), 32);
        assert_eq!(hex.parse::<DocumentId>(), Ok(a));
        assert!("xyz".parse::<DocumentId>().is_err());
        assert!("zz".repeat(16).parse::<DocumentId>().is_err());
    }
}
