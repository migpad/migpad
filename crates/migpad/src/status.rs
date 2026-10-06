//! What the status bar shows: where the caret is, how long the selection is, how many lines the
//! document has, its encoding and line breaks, and how much of a large file has loaded.

use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use gpui::{EntityId, Task};
use migpad_core::document::{Document, Format, Text};
use migpad_core::line_ending::LineEnding;
use migpad_core::text::TextStore;
use migpad_ui::StatusBar;

use crate::notices::Notice;
use crate::strings::{Key, fill, number, percent};

/// A selection of up to this many bytes is counted at once; a longer one in parts of this size, a
/// part a frame, so that counting gigabytes does not stop the window.
pub const COUNT_STEP: usize = 4 << 20;

/// How far a large file has loaded in the background: bytes read, of how many; whether the
/// loading has stopped, and why, if it stopped short.
#[derive(Clone, Debug)]
pub struct Loading {
    pub read: Arc<AtomicU64>,
    pub total: u64,
    pub stopped: Rc<Cell<bool>>,
    /// What to tell over the text of a load that stopped short: taken once it is told.
    pub failure: Rc<RefCell<Option<Notice>>>,
}

/// The characters of a selection, counted or being counted.
pub struct SelectionCount {
    /// What is counted: the document, the selection, and the revision of the documents of the
    /// window; an edit anywhere makes the count stale.
    pub key: (EntityId, Range<usize>, u64),
    /// How far the selection is counted, and how many characters were there.
    pub counted_to: usize,
    pub chars: usize,
    /// Counts the rest a part a frame; dropped, it stops.
    pub _task: Option<Task<()>>,
}

impl SelectionCount {
    /// The count, once it is done.
    pub fn done(&self) -> Option<usize> {
        (self.counted_to == self.key.1.end).then_some(self.chars)
    }

    /// Counts the next part of the selection in `text`; returns whether the count is done.
    pub fn step(&mut self, text: &Text) -> bool {
        let end = self.key.1.end.min(text.len());
        let mut to = (self.counted_to + COUNT_STEP).min(end);
        if to < end {
            to = text.floor_char_boundary(to, self.counted_to);
        }
        self.chars += count_chars(text, self.counted_to..to);
        self.counted_to = to;
        to >= end
    }
}

/// The characters of `range` as they show: an invalid sequence is one character, U+FFFD.
pub fn count_chars(text: &Text, range: Range<usize>) -> usize {
    let bytes = text.to_vec(range);
    bytes.utf8_chunks().map(|chunk| chunk.valid().chars().count() + usize::from(!chunk.invalid().is_empty())).sum()
}

/// The fields of the status bar for a document: the caret at `(line, column)`, both from zero —
/// the column `None` while it is found — the characters of the selection, `None` while they are
/// counted, and how far the file has loaded, if it is loading.
pub fn status_bar(
    doc: &Document,
    caret: (usize, Option<usize>),
    selection: Option<Option<usize>>,
    loading: Option<&Loading>,
) -> StatusBar {
    let (line, column) = caret;
    let column = column.map_or_else(|| "…".to_owned(), |column| number(column as u64 + 1));
    let position = fill(Key::StatusPosition, &[("line", &number(line as u64 + 1)), ("column", &column)]);
    let mut bar = StatusBar::new().left(position);
    if let Some(chars) = selection {
        let count = chars.map_or_else(|| "…".to_owned(), |chars| number(chars as u64));
        bar = bar.left(fill(Key::StatusSelection, &[("count", &count)]));
    }
    let lines = match loading {
        Some(loading) if doc.is_preview() && !loading.stopped.get() => {
            let read = loading.read.load(Ordering::Relaxed).min(loading.total);
            let share = (read * 100).checked_div(loading.total).unwrap_or(100);
            fill(Key::StatusLoading, &[("percent", &percent(share))])
        }
        _ => fill(Key::StatusLines, &[("count", &number(doc.lines().count() as u64))]),
    };
    bar.right(lines).right(encoding(doc.format)).right(line_ending(doc.format.line_ending))
}

/// The encoding as the status bar names it: `windows-1251`, `UTF-8 with BOM`.
fn encoding(format: Format) -> String {
    let name = format.encoding.name();
    if format.bom { fill(Key::StatusWithBom, &[("encoding", name)]) } else { name.to_owned() }
}

fn line_ending(line_ending: LineEnding) -> &'static str {
    match line_ending {
        LineEnding::Lf => "LF",
        LineEnding::CrLf => "CRLF",
        LineEnding::Cr => "CR",
    }
}

#[cfg(test)]
mod tests {
    use migpad_core::encoding::Encoding;

    use super::*;
    use crate::strings::{LANGUAGE_LOCK, Language, set_language};

    fn text(bytes: &[u8]) -> Text {
        Text::from_vec(bytes.to_vec())
    }

    #[test]
    fn characters_are_counted_as_they_show() {
        let t = text("ab\tжё\n€".as_bytes());
        assert_eq!(count_chars(&t, 0..t.len()), 7);
        assert_eq!(count_chars(&t, 3..7), 2);
        // An invalid sequence shows as one U+FFFD, a lone continuation byte too.
        let t = text(b"a\xE2\x82b\x80c");
        assert_eq!(count_chars(&t, 0..t.len()), 5);
    }

    #[test]
    fn a_long_selection_is_counted_in_parts_at_character_boundaries() {
        let line = "жёлтый €uro ".repeat(COUNT_STEP / 8);
        let t = text(line.as_bytes());
        let range = 1..t.len();
        let mut count =
            SelectionCount { key: (EntityId::from(1u64), range.clone(), 0), counted_to: 1, chars: 0, _task: None };
        let mut steps = 0;
        while !count.step(&t) {
            steps += 1;
            assert!(count.done().is_none());
        }
        assert!(steps >= 1, "more than one part");
        assert_eq!(count.done(), Some(count_chars(&t, range)));
        assert_eq!(count.done(), Some(line.chars().count() - 1 + 1), "the first byte splits ж: one U+FFFD");
    }

    #[test]
    fn fields_follow_the_language() {
        let _lock = LANGUAGE_LOCK.lock();
        let mut doc = Document::new();
        doc.format = Format { encoding: Encoding::UTF_8, bom: true, line_ending: LineEnding::CrLf };
        let fields = |bar: StatusBar| bar.fields().map(str::to_owned).collect::<Vec<_>>();
        set_language(Language::English);
        let english = fields(status_bar(&doc, (1233, Some(4)), Some(Some(15000)), None));
        set_language(Language::Russian);
        let russian = fields(status_bar(&doc, (1233, None), Some(None), None));
        set_language(Language::English);
        assert_eq!(english, ["Ln 1,234, Col 5", "Selected: 15,000", "Lines: 1", "UTF-8 with BOM", "CRLF"]);
        assert_eq!(russian, ["Стр 1\u{202f}234, стлб …", "Выделено: …", "Строк: 1", "UTF-8 с BOM", "CRLF"]);
    }
}
