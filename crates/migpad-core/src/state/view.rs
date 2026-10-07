//! How the windows show, as the View menu leaves it: the zoom of the text, and on macOS the menu
//! bar in each window — `state/view.toml`.

use std::path::Path;

use toml_edit::value;

use super::{StateError, invalid, new_document, parse, read};

/// How the windows show text and menus.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ViewState {
    /// Steps the text is larger than its size; below zero, smaller.
    pub zoom: i64,
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
        doc["zoom"] = value(self.zoom);
        doc["menu_bar"] = value(self.menu_bar);
        doc.to_string()
    }

    fn from_toml(text: &str) -> Result<ViewState, StateError> {
        let doc = parse(text)?;
        let zoom = match doc.get("zoom") {
            None => 0,
            Some(item) => item.as_integer().ok_or_else(|| invalid("the zoom is not a number"))?,
        };
        let menu_bar = match doc.get("menu_bar") {
            None => false,
            Some(item) => item.as_bool().ok_or_else(|| invalid("menu_bar is neither true nor false"))?,
        };
        Ok(ViewState { zoom, menu_bar })
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
        let view = ViewState { zoom: -2, menu_bar: true };
        super::super::write_atomically(&path, &view.to_toml()).unwrap();
        assert_eq!(ViewState::read(&path).unwrap(), Some(view));
        // What is missing takes its default; what is wrong is told.
        assert_eq!(ViewState::from_toml("version = 1\n").unwrap(), ViewState::default());
        assert!(ViewState::from_toml("version = 1\nzoom = \"big\"\n").is_err());
        assert!(ViewState::from_toml("version = 1\nmenu_bar = 1\n").is_err());
    }
}
