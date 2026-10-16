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

/// Takes the pinned tabs out of the closed windows — they open again at the next start — which keep
/// their other tabs, or go if they had none. Returns what stays closed, and the pinned tabs of each
/// window in their order, with the one of them to show: the one that was active, or the first.
pub fn take_pinned(closed: Vec<ClosedState>) -> (Vec<ClosedState>, Vec<(Vec<TabState>, usize)>) {
    let mut pinned = Vec::new();
    let closed = closed
        .into_iter()
        .filter_map(|entry| match entry {
            ClosedState::Window { tabs, active } if tabs.iter().any(|tab| tab.pinned) => {
                // Where the active tab is among the tabs of its kind.
                let active_pinned = tabs.get(active).is_some_and(|tab| tab.pinned);
                let place = tabs.iter().take(active).filter(|tab| tab.pinned == active_pinned).count();
                let (pins, rest): (Vec<TabState>, Vec<TabState>) = tabs.into_iter().partition(|tab| tab.pinned);
                pinned.push((pins, if active_pinned { place } else { 0 }));
                let active = if active_pinned { 0 } else { place };
                (!rest.is_empty()).then(|| ClosedState::Window { active: active.min(rest.len() - 1), tabs: rest })
            }
            entry => Some(entry),
        })
        .collect();
    (closed, pinned)
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
        let tab = |path: &str| TabState {
            document: None,
            path: Some(PathBuf::from(path)),
            encoding: Some(crate::encoding::Encoding::UTF_16LE),
            selection: Selection::caret(4),
            pinned: false,
            following: false,
            at_end: false,
        };
        let kept = TabState {
            document: Some(DocumentId::random()),
            path: None,
            encoding: None,
            selection: Selection { anchor: 1, head: 0 },
            pinned: false,
            following: false,
            at_end: false,
        };
        let pinned = TabState { pinned: true, ..tab("/pinned.txt") };
        let closed = vec![
            ClosedState::Tab(kept),
            ClosedState::Window { tabs: vec![pinned, tab("/a.txt"), tab("/b.txt")], active: 1 },
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
    fn pinned_tabs_leave_the_closed_windows() {
        let tab = |path: &str, pinned: bool| TabState {
            document: None,
            path: Some(PathBuf::from(path)),
            encoding: None,
            selection: Selection::caret(0),
            pinned,
            following: false,
            at_end: false,
        };
        let closed = vec![
            ClosedState::Tab(tab("/a.txt", false)),
            // The active tab is not pinned: it stays active among the closed ones.
            ClosedState::Window {
                tabs: vec![tab("/b.txt", true), tab("/c.txt", true), tab("/d.txt", false), tab("/e.txt", false)],
                active: 3,
            },
            // The active tab is pinned: it is shown among the pinned ones.
            ClosedState::Window {
                tabs: vec![tab("/f.txt", true), tab("/g.txt", true), tab("/h.txt", false)],
                active: 1,
            },
            // Only pinned tabs: the window goes.
            ClosedState::Window { tabs: vec![tab("/i.txt", true)], active: 0 },
            ClosedState::Window { tabs: vec![tab("/j.txt", false), tab("/k.txt", false)], active: 1 },
        ];
        let (closed, pinned) = take_pinned(closed);
        assert_eq!(
            closed,
            [
                ClosedState::Tab(tab("/a.txt", false)),
                ClosedState::Window { tabs: vec![tab("/d.txt", false), tab("/e.txt", false)], active: 1 },
                ClosedState::Window { tabs: vec![tab("/h.txt", false)], active: 0 },
                ClosedState::Window { tabs: vec![tab("/j.txt", false), tab("/k.txt", false)], active: 1 },
            ]
        );
        assert_eq!(
            pinned,
            [
                (vec![tab("/b.txt", true), tab("/c.txt", true)], 0),
                (vec![tab("/f.txt", true), tab("/g.txt", true)], 1),
                (vec![tab("/i.txt", true)], 0),
            ]
        );
        assert_eq!(take_pinned(Vec::new()), (Vec::new(), Vec::new()));
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
