//! What MigPad keeps for itself between starts, in TOML files in the folder `state` of the data:
//! the session — windows and their tabs — the tabs closed lately, the recent files, and how the
//! windows show. A file is written whole,
//! through a temporary file, so that a crash leaves either the old one or the new one.

pub mod closed;
pub mod recent;
pub mod session;
pub mod view;

use std::fmt;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use toml_edit::{Item, Table, value};

use crate::document::DocumentId;
use crate::history::Selection;

/// The version of the files of the state; a file of another version is not read.
const VERSION: i64 = 1;

/// Why a file of the state could not be read.
#[derive(Debug)]
pub enum StateError {
    Io(io::Error),
    /// The file is not what MigPad writes: broken, or of another version.
    Invalid(String),
}

impl From<io::Error> for StateError {
    fn from(error: io::Error) -> Self {
        StateError::Io(error)
    }
}

impl fmt::Display for StateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StateError::Io(error) => error.fmt(f),
            StateError::Invalid(why) => f.write_str(why),
        }
    }
}

impl std::error::Error for StateError {}

/// A tab as the state keeps it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabState {
    /// The document, to recover from its journal if it has one.
    pub document: Option<DocumentId>,
    /// The file of the document; an untitled one has none.
    pub path: Option<PathBuf>,
    pub selection: Selection,
}

/// Writes `text` to `path` whole: through a temporary file next to it, flushed to the disk, which
/// takes the place of the old one. The folder is made if it is not there.
pub fn write_atomically(path: &Path, text: &str) -> io::Result<()> {
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder)?;
    }
    let temp = path.with_extension("toml.tmp");
    let write = || -> io::Result<()> {
        let mut file = File::create(&temp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temp, path)
    };
    let result = write();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// The text of the file at `path`; `None` if there is no file.
fn read(path: &Path) -> Result<Option<String>, StateError> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) if error.kind() == io::ErrorKind::InvalidData => Err(StateError::Invalid(error.to_string())),
        Err(error) => Err(error.into()),
    }
}

/// Parses a file of the state and checks its version.
fn parse(text: &str) -> Result<toml_edit::DocumentMut, StateError> {
    let doc: toml_edit::DocumentMut = text.parse().map_err(|error: toml_edit::TomlError| invalid(error.message()))?;
    match doc.get("version").and_then(Item::as_integer) {
        Some(VERSION) => Ok(doc),
        Some(other) => Err(invalid(&format!("version {other} is not known"))),
        None => Err(invalid("no version")),
    }
}

/// A new file of the state: a comment that tells what it is, and the version.
fn new_document(comment: &str) -> toml_edit::DocumentMut {
    let mut doc = toml_edit::DocumentMut::new();
    doc["version"] = value(VERSION);
    if let Some(mut key) = doc.key_mut("version") {
        key.leaf_decor_mut().set_prefix(format!("# {comment}\n"));
    }
    doc
}

fn invalid(why: &str) -> StateError {
    StateError::Invalid(why.to_owned())
}

/// A count or a position: a non-negative integer.
fn number(table: &Table, key: &str) -> Result<usize, StateError> {
    let number = table.get(key).and_then(Item::as_integer).ok_or_else(|| invalid(&format!("no {key}")))?;
    usize::try_from(number).map_err(|_| invalid(&format!("{key} is {number}")))
}

fn tab_to_table(tab: &TabState) -> Table {
    let mut table = Table::new();
    if let Some(document) = tab.document {
        table["document"] = value(document.to_string());
    }
    if let Some(path) = &tab.path {
        table["path"] = value(path.to_string_lossy().into_owned());
    }
    table["anchor"] = value(tab.selection.anchor as i64);
    table["head"] = value(tab.selection.head as i64);
    table
}

fn tab_from_table(table: &Table) -> Result<TabState, StateError> {
    let document = match table.get("document").map(|item| item.as_str()) {
        None => None,
        Some(Some(id)) => Some(id.parse().map_err(|()| invalid(&format!("the document {id} is not an id")))?),
        Some(None) => return Err(invalid("the document is not a string")),
    };
    let path = match table.get("path").map(|item| item.as_str()) {
        None => None,
        Some(Some(path)) => Some(PathBuf::from(path)),
        Some(None) => return Err(invalid("the path is not a string")),
    };
    let selection = Selection { anchor: number(table, "anchor")?, head: number(table, "head")? };
    Ok(TabState { document, path, selection })
}

/// The tables of the array `key` of `table`; none if it has no such key.
fn tables<'a>(table: &'a Table, key: &str) -> Result<Vec<&'a Table>, StateError> {
    match table.get(key) {
        None => Ok(Vec::new()),
        Some(item) => {
            let array = item.as_array_of_tables().ok_or_else(|| invalid(&format!("{key} is not a list of tables")))?;
            Ok(array.iter().collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn files_are_written_whole() {
        let dir = TempDir::new("state-write");
        let path = dir.0.join("state").join("session.toml");
        write_atomically(&path, "version = 1\n").unwrap();
        write_atomically(&path, "version = 1\nactive = 0\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "version = 1\nactive = 0\n");
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1, "no temporary file is left");
        assert_eq!(read(&dir.0.join("none.toml")).unwrap(), None);
    }

    #[test]
    fn files_of_another_version_are_not_read() {
        assert!(parse("version = 1").is_ok());
        assert!(matches!(parse("version = 2"), Err(StateError::Invalid(_))));
        assert!(matches!(parse("active = 0"), Err(StateError::Invalid(_))));
        assert!(matches!(parse("version = "), Err(StateError::Invalid(_))));
    }
}
