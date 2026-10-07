//! How the windows show, as the View menu leaves it: on macOS, the menu bar in each window —
//! `state/view.toml`.

use std::path::Path;

use toml_edit::value;

use super::{StateError, invalid, new_document, parse, read};

/// How the windows show.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ViewState {
    /// Whether each window has the menu bar MigPad draws: on macOS, besides the menu bar of the
    /// system. Windows and Linux always have it.
    pub menu_bar: bool,
}

impl ViewState {
    /// What the file at `path` keeps; `None` if there is no file.
    pub fn read(path: &Path) -> Result<Option<ViewState>, StateError> {
        read(path)?.map(|text| ViewState::from_toml(&text)).transpose()
    }

    pub fn to_toml(&self) -> String {
        let mut doc = new_document("How the windows of MigPad show, as the View menu leaves it.");
        doc["menu_bar"] = value(self.menu_bar);
        doc.to_string()
    }

    fn from_toml(text: &str) -> Result<ViewState, StateError> {
        let doc = parse(text)?;
        let menu_bar = match doc.get("menu_bar") {
            None => false,
            Some(item) => item.as_bool().ok_or_else(|| invalid("menu_bar is neither true nor false"))?,
        };
        Ok(ViewState { menu_bar })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn the_view_is_kept_and_read_back() {
        let dir = TempDir::new("state-view");
        let path = dir.0.join("view.toml");
        assert_eq!(ViewState::read(&path).unwrap(), None);
        let view = ViewState { menu_bar: true };
        super::super::write_atomically(&path, &view.to_toml()).unwrap();
        assert_eq!(ViewState::read(&path).unwrap(), Some(view));
        // What is missing takes its default; what is wrong is told.
        assert_eq!(ViewState::from_toml("version = 1\n").unwrap(), ViewState::default());
        assert!(ViewState::from_toml("version = 1\nmenu_bar = 1\n").is_err());
    }
}
