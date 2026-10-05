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
