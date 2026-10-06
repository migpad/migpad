//! The colors of the editor, light and dark: the theme of the application chooses which one every
//! view paints with.

use gpui::{App, Global};

/// The colors of the editor, as `0xRRGGBB`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EditorColors {
    pub background: u32,
    pub text: u32,
    pub gutter: u32,
    pub line_number: u32,
    pub track: u32,
    pub thumb: u32,
    pub selection: u32,
    /// The selection of a view without focus or in an inactive window.
    pub selection_inactive: u32,
    pub caret: u32,
    /// Marks of what is otherwise invisible: whitespace, control characters.
    pub mark: u32,
    pub guide: u32,
    /// Labels of line breaks: LF, CRLF, CR.
    pub label: u32,
    pub label_background: u32,
}

impl EditorColors {
    pub const LIGHT: EditorColors = EditorColors {
        background: 0xffffff,
        text: 0x1f1f1f,
        gutter: 0xf5f5f5,
        line_number: 0x6e6e6e,
        track: 0xf4f4f4,
        thumb: 0xc0c0c0,
        selection: 0xb4d5fe,
        selection_inactive: 0xdcdcdc,
        caret: 0x1f1f1f,
        mark: 0xb0b0b0,
        guide: 0xe2e2e2,
        label: 0xffffff,
        label_background: 0xbcbcbc,
    };

    pub const DARK: EditorColors = EditorColors {
        background: 0x1e1e1e,
        text: 0xd8d8d8,
        gutter: 0x252525,
        line_number: 0x909090,
        track: 0x232323,
        thumb: 0x4c4c4c,
        selection: 0x264f78,
        selection_inactive: 0x3a3d41,
        caret: 0xe0e0e0,
        mark: 0x5c5c5c,
        guide: 0x353535,
        label: 0x1e1e1e,
        label_background: 0x6a6a6a,
    };

    /// The colors views paint with: those the application chose, light until it does.
    pub fn current(cx: &App) -> EditorColors {
        cx.try_global::<EditorColors>().copied().unwrap_or(Self::LIGHT)
    }
}

impl Global for EditorColors {}
