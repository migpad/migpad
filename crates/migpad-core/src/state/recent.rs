//! The files opened and saved lately, each with its encoding and where the selection was: the
//! menu File ▸ Recent Files — `state/recent.toml`.

use std::path::{Path, PathBuf};

use toml_edit::{ArrayOfTables, Item, Table, value};

use super::{StateError, invalid, new_document, number, parse, read, tables};
use crate::history::Selection;

/// A recent file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecentFile {
    pub path: PathBuf,
    /// The name of the encoding it was read or saved in, to open it in that one again.
    pub encoding: Option<String>,
    pub selection: Selection,
}

/// The recent files, the last first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecentFiles {
    files: Vec<RecentFile>,
}

impl RecentFiles {
    /// How many files the list keeps.
    pub const LIMIT: usize = 20;

    pub fn files(&self) -> &[RecentFile] {
        &self.files
    }

    /// What the list has of the file at `path`.
    pub fn get(&self, path: &Path) -> Option<&RecentFile> {
        self.files.iter().find(|file| same_file(&file.path, path))
    }

    /// `file` was opened or saved: it goes first, in place of what the list had of it; past
    /// [`RecentFiles::LIMIT`] the oldest goes.
    pub fn touch(&mut self, file: RecentFile) {
        self.remove(&file.path);
        self.files.insert(0, file);
        self.files.truncate(Self::LIMIT);
    }

    /// The file at `path` closed with `selection`, in `encoding`: it is remembered so, in its
    /// place. Nothing for a file the list has not.
    pub fn update(&mut self, path: &Path, encoding: Option<String>, selection: Selection) {
        if let Some(file) = self.files.iter_mut().find(|file| same_file(&file.path, path)) {
            file.encoding = encoding;
            file.selection = selection;
        }
    }

    /// Takes the file at `path` off the list; tells whether it was there.
    pub fn remove(&mut self, path: &Path) -> bool {
        let before = self.files.len();
        self.files.retain(|file| !same_file(&file.path, path));
        self.files.len() != before
    }

    pub fn clear(&mut self) {
        self.files.clear();
    }

    /// Takes the files that are not there any more off the list; tells whether there were any.
    pub fn forget_missing(&mut self) -> bool {
        let before = self.files.len();
        self.files.retain(|file| file.path.exists());
        self.files.len() != before
    }

    /// The list in the file at `path`; an empty one if there is no file.
    pub fn read(path: &Path) -> Result<RecentFiles, StateError> {
        read(path)?.map_or_else(|| Ok(RecentFiles::default()), |text| RecentFiles::from_toml(&text))
    }

    pub fn to_toml(&self) -> String {
        let mut doc = new_document("The files MigPad opened and saved lately, the last first.");
        let mut files = ArrayOfTables::new();
        for file in &self.files {
            let mut table = Table::new();
            table["path"] = value(file.path.to_string_lossy().into_owned());
            if let Some(encoding) = &file.encoding {
                table["encoding"] = value(encoding);
            }
            table["anchor"] = value(file.selection.anchor as i64);
            table["head"] = value(file.selection.head as i64);
            files.push(table);
        }
        doc.insert("file", Item::ArrayOfTables(files));
        doc.to_string()
    }

    pub fn from_toml(text: &str) -> Result<RecentFiles, StateError> {
        let doc = parse(text)?;
        let file = |table: &Table| -> Result<RecentFile, StateError> {
            let path = table.get("path").and_then(Item::as_str).ok_or_else(|| invalid("a file without a path"))?;
            let encoding = table.get("encoding").and_then(Item::as_str).map(str::to_owned);
            let selection = Selection { anchor: number(table, "anchor")?, head: number(table, "head")? };
            Ok(RecentFile { path: PathBuf::from(path), encoding, selection })
        };
        let mut files = tables(doc.as_table(), "file")?.into_iter().map(file).collect::<Result<Vec<_>, _>>()?;
        files.truncate(Self::LIMIT);
        Ok(RecentFiles { files })
    }
}

/// Whether two paths are the same file: the same path, or the same once links are resolved.
fn same_file(a: &Path, b: &Path) -> bool {
    a == b || std::fs::canonicalize(a).is_ok_and(|a| std::fs::canonicalize(b).is_ok_and(|b| a == b))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::state::write_atomically;
    use crate::testing::TempDir;

    fn file(path: &Path, caret: usize) -> RecentFile {
        RecentFile {
            path: path.to_owned(),
            encoding: Some("windows-1251".to_owned()),
            selection: Selection::caret(caret),
        }
    }

    fn paths(recent: &RecentFiles) -> Vec<PathBuf> {
        recent.files().iter().map(|file| file.path.clone()).collect()
    }

    #[test]
    fn the_last_opened_goes_first_and_the_oldest_go() {
        let mut recent = RecentFiles::default();
        let path = |i: usize| PathBuf::from(format!("/notes/{i}.txt"));
        for i in 0..RecentFiles::LIMIT + 3 {
            recent.touch(file(&path(i), 0));
        }
        assert_eq!(recent.files().len(), RecentFiles::LIMIT);
        assert_eq!(recent.files()[0].path, path(RecentFiles::LIMIT + 2));
        assert!(!paths(&recent).contains(&path(2)), "the oldest are gone");
        // Opened again: first, once.
        recent.touch(file(&path(10), 5));
        assert_eq!(recent.files()[0], file(&path(10), 5));
        assert_eq!(recent.get(&path(10)), Some(&file(&path(10), 5)));
        assert_eq!(recent.files().len(), RecentFiles::LIMIT);
        assert_eq!(paths(&recent).iter().filter(|p| **p == path(10)).count(), 1);
        // Closed: remembered in its place.
        recent.update(&path(20), None, Selection::caret(9));
        assert_eq!(recent.files()[3], RecentFile { path: path(20), encoding: None, selection: Selection::caret(9) });
        assert!(recent.remove(&path(20)));
        assert!(!recent.remove(&path(20)));
        recent.clear();
        assert!(recent.files().is_empty());
    }

    #[test]
    fn files_that_are_gone_are_forgotten() {
        let dir = TempDir::new("recent-gone");
        let (kept, gone) = (dir.0.join("kept.txt"), dir.0.join("gone.txt"));
        fs::write(&kept, b"").unwrap();
        let mut recent = RecentFiles::default();
        recent.touch(file(&kept, 1));
        recent.touch(file(&gone, 2));
        assert!(recent.forget_missing());
        assert_eq!(paths(&recent), std::slice::from_ref(&kept));
        assert!(!recent.forget_missing());
        // The same file by another path is one file.
        let linked = dir.0.join(".").join("kept.txt");
        recent.touch(file(&linked, 3));
        assert_eq!(paths(&recent), [linked]);
    }

    #[test]
    fn the_list_reads_back_as_it_was_written() {
        let mut recent = RecentFiles::default();
        recent.touch(RecentFile {
            path: PathBuf::from("/a/план.txt"),
            encoding: None,
            selection: Selection { anchor: 4, head: 1 },
        });
        recent.touch(file(Path::new("C:\\notes\\b.txt"), 7));
        let dir = TempDir::new("recent-file");
        let path = dir.0.join("recent.toml");
        assert_eq!(RecentFiles::read(&path).unwrap(), RecentFiles::default());
        write_atomically(&path, &recent.to_toml()).unwrap();
        assert_eq!(RecentFiles::read(&path).unwrap(), recent);
        assert!(matches!(
            RecentFiles::from_toml("version = 1\n[[file]]\nanchor = 0\nhead = 0\n"),
            Err(StateError::Invalid(_))
        ));
    }
}
