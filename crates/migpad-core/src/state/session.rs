//! The session: the windows of MigPad and their tabs as they were when it last wrote them, to
//! open again at the next start — `state/session.toml`.

use std::path::Path;

use toml_edit::{ArrayOfTables, Item, Table, value};

use super::{StateError, TabState, invalid, new_document, number, parse, read, tab_from_table, tab_to_table, tables};

/// The windows and their tabs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Session {
    pub windows: Vec<WindowState>,
    /// The window that was active.
    pub active: usize,
}

/// A window and its tabs.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowState {
    /// Where the window was, in logical pixels: the corner of its frame and the size of its
    /// content; for a zoomed or full-screen window, where it goes back to.
    pub bounds: Rect,
    pub mode: WindowMode,
    /// The screen the window was on, which its place is counted from: the identifier of the system.
    pub display: Option<String>,
    pub tabs: Vec<TabState>,
    /// The tab that was active.
    pub active: usize,
}

/// A rectangle on the screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// How a window was shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WindowMode {
    #[default]
    Normal,
    /// Zoomed to fill the screen, on macOS; maximized on Windows and Linux.
    Maximized,
    FullScreen,
}

impl WindowMode {
    fn name(self) -> &'static str {
        match self {
            WindowMode::Normal => "normal",
            WindowMode::Maximized => "maximized",
            WindowMode::FullScreen => "full screen",
        }
    }

    fn named(name: &str) -> Option<WindowMode> {
        [WindowMode::Normal, WindowMode::Maximized, WindowMode::FullScreen].into_iter().find(|mode| mode.name() == name)
    }
}

impl Session {
    /// The session in the file at `path`; `None` if there is none.
    pub fn read(path: &Path) -> Result<Option<Session>, StateError> {
        read(path)?.map(|text| Session::from_toml(&text)).transpose()
    }

    pub fn to_toml(&self) -> String {
        let mut doc = new_document("The windows and tabs of MigPad, to open again at the next start.");
        doc["active"] = value(self.active as i64);
        let mut windows = ArrayOfTables::new();
        for window in &self.windows {
            let mut table = Table::new();
            table["x"] = value(window.bounds.x);
            table["y"] = value(window.bounds.y);
            table["width"] = value(window.bounds.width);
            table["height"] = value(window.bounds.height);
            table["mode"] = value(window.mode.name());
            if let Some(display) = &window.display {
                table["display"] = value(display);
            }
            table["active"] = value(window.active as i64);
            let tabs: ArrayOfTables = window.tabs.iter().map(tab_to_table).collect();
            table.insert("tab", Item::ArrayOfTables(tabs));
            windows.push(table);
        }
        doc.insert("window", Item::ArrayOfTables(windows));
        doc.to_string()
    }

    pub fn from_toml(text: &str) -> Result<Session, StateError> {
        let doc = parse(text)?;
        let windows = tables(doc.as_table(), "window")?.into_iter().map(window_from_table).collect::<Result<_, _>>()?;
        Ok(Session { windows, active: number(doc.as_table(), "active")? })
    }
}

fn window_from_table(table: &Table) -> Result<WindowState, StateError> {
    let float = |key: &str| {
        let item = table.get(key).ok_or_else(|| invalid(&format!("no {key}")))?;
        let number = item.as_float().or_else(|| item.as_integer().map(|n| n as f64));
        number.filter(|n| n.is_finite()).ok_or_else(|| invalid(&format!("{key} is not a number")))
    };
    let bounds = Rect { x: float("x")?, y: float("y")?, width: float("width")?, height: float("height")? };
    let mode = table.get("mode").and_then(Item::as_str).and_then(WindowMode::named).unwrap_or_default();
    let display = table.get("display").and_then(Item::as_str).map(str::to_owned);
    let tabs = tables(table, "tab")?.into_iter().map(tab_from_table).collect::<Result<_, _>>()?;
    Ok(WindowState { bounds, mode, display, tabs, active: number(table, "active")? })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::document::DocumentId;
    use crate::history::Selection;
    use crate::state::write_atomically;
    use crate::testing::TempDir;

    fn session() -> Session {
        let tab = |path: Option<&str>, anchor, head| TabState {
            document: Some(DocumentId::random()),
            path: path.map(PathBuf::from),
            selection: Selection { anchor, head },
        };
        Session {
            windows: vec![
                WindowState {
                    bounds: Rect { x: 120.0, y: 80.5, width: 1000.0, height: 700.0 },
                    mode: WindowMode::Normal,
                    display: Some("69733F6E-86C8-4F2A-9C9C-4A7B9DA4D9A1".to_owned()),
                    tabs: vec![tab(Some("/notes/план.txt"), 3, 7), tab(None, 0, 0)],
                    active: 1,
                },
                WindowState {
                    bounds: Rect { x: -1500.0, y: 0.0, width: 640.0, height: 480.0 },
                    mode: WindowMode::FullScreen,
                    display: None,
                    tabs: vec![TabState { document: None, ..tab(Some("C:\\work\\\"quoted\".txt"), 9, 2) }],
                    active: 0,
                },
            ],
            active: 1,
        }
    }

    #[test]
    fn a_session_reads_back_as_it_was_written() {
        let session = session();
        let text = session.to_toml();
        assert!(text.starts_with("# The windows and tabs of MigPad"), "{text}");
        assert_eq!(Session::from_toml(&text).unwrap(), session);
        let dir = TempDir::new("session-file");
        let path = dir.0.join("session.toml");
        assert_eq!(Session::read(&path).unwrap(), None);
        write_atomically(&path, &text).unwrap();
        assert_eq!(Session::read(&path).unwrap(), Some(session));
        assert_eq!(Session::from_toml(&Session::default().to_toml()).unwrap(), Session::default());
    }

    #[test]
    fn a_broken_session_is_an_error() {
        let text = session().to_toml();
        for broken in [
            text.replace("version = 1", "version = 7"),
            text.replace("anchor = 3", "anchor = -3"),
            text.replace("width = 1000.0", "width = \"wide\""),
            text.replace("\nactive = 1\n", "\n"),
            text[..text.len() / 2].to_owned(),
            "\u{0}garbage".to_owned(),
        ] {
            assert!(matches!(Session::from_toml(&broken), Err(StateError::Invalid(_))), "{broken}");
        }
        // A mode MigPad does not know is the usual one.
        let unknown = text.replace("mode = \"full screen\"", "mode = \"minimized\"");
        assert_eq!(Session::from_toml(&unknown).unwrap().windows[1].mode, WindowMode::Normal);
    }
}
