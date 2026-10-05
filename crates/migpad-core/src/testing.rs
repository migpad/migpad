//! Helpers for tests.

use std::fs;
use std::path::PathBuf;

/// A file in the temporary directory, removed when dropped.
pub struct TempFile(pub PathBuf);

impl TempFile {
    /// `name` must be unique among the tests.
    pub fn new(name: &str, bytes: &[u8]) -> Self {
        let path = std::env::temp_dir().join(format!("migpad-test-{}-{name}", std::process::id()));
        fs::write(&path, bytes).unwrap();
        TempFile(path)
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// A directory in the temporary directory, removed with its contents when dropped.
pub struct TempDir(pub PathBuf);

impl TempDir {
    /// `name` must be unique among the tests; a [`TempFile`] may have the same one.
    pub fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("migpad-test-{}-{name}.d", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }

    /// The files in the directory.
    pub fn files(&self) -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = fs::read_dir(&self.0).unwrap().map(|entry| entry.unwrap().path()).collect();
        files.sort();
        files
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
