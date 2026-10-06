//! The tabs and windows closed lately, to open again after a restart too: `state/closed.toml`.

use std::path::Path;

use toml_edit::{ArrayOfTables, Item, Table, value};

use super::{StateError, TabState, invalid, new_document, number, parse, read, tab_from_table, tab_to_table, tables};

/// A tab or a window closed lately.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClosedState {
    Tab(TabState),
    /// A window with its tabs, and the one that was active: it comes back whole.
    Window {
        tabs: Vec<TabState>,
        active: usize,
    },
}

/// Reads the closed tabs and windows from the file at `path`, the last closed first; none if
/// there is no file.
pub fn read_closed(path: &Path) -> Result<Vec<ClosedState>, StateError> {
    match read(path)? {
        Some(text) => from_toml(&text),
        None => Ok(Vec::new()),
    }
}

pub fn to_toml(closed: &[ClosedState]) -> String {
    let mut doc = new_document("The tabs and windows of MigPad closed lately, the last first.");
    let mut entries = ArrayOfTables::new();
    for entry in closed {
        let mut table = Table::new();
        let tabs = match entry {
            ClosedState::Tab(tab) => std::slice::from_ref(tab),
            ClosedState::Window { tabs, active } => {
                table["window"] = value(true);
                table["active"] = value(*active as i64);
                tabs.as_slice()
            }
        };
        table.insert("tab", Item::ArrayOfTables(tabs.iter().map(tab_to_table).collect()));
        entries.push(table);
    }
    doc.insert("closed", Item::ArrayOfTables(entries));
    doc.to_string()
}

pub fn from_toml(text: &str) -> Result<Vec<ClosedState>, StateError> {
    let doc = parse(text)?;
    let entry = |table: &Table| -> Result<ClosedState, StateError> {
        let mut tabs = tables(table, "tab")?.into_iter().map(tab_from_table).collect::<Result<Vec<_>, _>>()?;
        if table.get("window").and_then(Item::as_bool).unwrap_or(false) {
            return Ok(ClosedState::Window { tabs, active: number(table, "active")? });
        }
        match tabs.pop() {
            Some(tab) if tabs.is_empty() => Ok(ClosedState::Tab(tab)),
            _ => Err(invalid("a closed tab is not one tab")),
        }
    };
    tables(doc.as_table(), "closed")?.into_iter().map(entry).collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::document::DocumentId;
    use crate::history::Selection;
    use crate::state::write_atomically;
    use crate::testing::TempDir;

    #[test]
    fn closed_tabs_and_windows_read_back_as_they_were_written() {
        let tab =
            |path: &str| TabState { document: None, path: Some(PathBuf::from(path)), selection: Selection::caret(4) };
        let kept =
            TabState { document: Some(DocumentId::random()), path: None, selection: Selection { anchor: 1, head: 0 } };
        let closed = vec![
            ClosedState::Tab(kept),
            ClosedState::Window { tabs: vec![tab("/a.txt"), tab("/b.txt")], active: 1 },
            ClosedState::Tab(tab("/c.txt")),
        ];
        let dir = TempDir::new("closed-file");
        let path = dir.0.join("closed.toml");
        assert_eq!(read_closed(&path).unwrap(), []);
        write_atomically(&path, &to_toml(&closed)).unwrap();
        assert_eq!(read_closed(&path).unwrap(), closed);
        assert_eq!(from_toml(&to_toml(&[])).unwrap(), []);
    }

    #[test]
    fn a_broken_list_is_an_error() {
        let two_tabs =
            "version = 1\n[[closed]]\n[[closed.tab]]\nanchor = 0\nhead = 0\n[[closed.tab]]\nanchor = 0\nhead = 0\n";
        assert!(matches!(from_toml(two_tabs), Err(StateError::Invalid(_))));
        assert!(matches!(from_toml("version = 1\nclosed = 3\n"), Err(StateError::Invalid(_))));
        assert!(matches!(from_toml("[[closed]"), Err(StateError::Invalid(_))));
    }
}
