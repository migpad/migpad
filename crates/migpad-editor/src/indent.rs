//! Indentation, for the indent guides: how far lines are indented, and the step of a level.

use std::collections::HashMap;
use std::ops::Range;

use migpad_core::text::{LineIndex, TextStore};

/// Leading whitespace is read no further than this.
const MAX_INDENT_BYTES: usize = 1024;
/// How far from a blank line its neighbours are looked for.
const MAX_BLANK_RUN: usize = 100;
/// Steps of indentation that guides use.
const STEPS: Range<usize> = 2..17;

/// The columns of leading spaces and tabs of the line with the bytes `range`; `None` for a line
/// with nothing else, blank.
pub fn indentation<S: TextStore>(text: &S, range: Range<usize>, tab_width: usize) -> Option<usize> {
    let mut column = 0;
    for pos in range.start..range.end.min(range.start + MAX_INDENT_BYTES) {
        match text.byte(pos) {
            b' ' => column += 1,
            b'\t' => column += tab_width - column % tab_width,
            _ => return Some(column),
        }
    }
    (range.len() > MAX_INDENT_BYTES).then_some(column)
}

/// The levels of guides for `visible` lines: the indentation of each line, and of a blank line the
/// smaller indentation of the lines around it, as a blank line inside a block belongs to it.
pub fn guide_levels<S: TextStore>(text: &S, lines: &LineIndex, visible: Range<usize>, tab_width: usize) -> Vec<usize> {
    let mut known: HashMap<usize, Option<usize>> = HashMap::new();
    let mut indent = |line: usize| {
        *known.entry(line).or_insert_with(|| indentation(text, lines.line_range(text, line).0, tab_width))
    };
    let count = lines.count();
    visible
        .map(|line| match indent(line) {
            Some(level) => level,
            None => {
                let above = (line.saturating_sub(MAX_BLANK_RUN)..line).rev().find_map(&mut indent);
                let below = (line + 1..count.min(line + 1 + MAX_BLANK_RUN)).find_map(&mut indent);
                above.unwrap_or(0).min(below.unwrap_or(0))
            }
        })
        .collect()
}

/// The step of a level of indentation: the most common increase from one line to the next among
/// `indents` (`None` for blank lines, skipped), else the smallest indentation, else `fallback`.
pub fn indent_step(indents: &[Option<usize>], fallback: usize) -> usize {
    let levels: Vec<usize> = indents.iter().flatten().copied().collect();
    let mut counts: HashMap<usize, usize> = HashMap::new();
    for pair in levels.windows(2) {
        if pair[1] > pair[0] {
            *counts.entry(pair[1] - pair[0]).or_default() += 1;
        }
    }
    // The most common increase; of equally common ones, the smallest.
    let common = counts.into_iter().max_by_key(|&(step, count)| (count, std::cmp::Reverse(step))).map(|(step, _)| step);
    let smallest = levels.iter().copied().filter(|&level| level > 0).min();
    common.or(smallest).filter(|step| STEPS.contains(step)).unwrap_or(fallback)
}

#[cfg(test)]
mod tests {
    use migpad_core::text::{GapBuffer, Indexer};

    use super::*;

    fn text(s: &str) -> (GapBuffer, LineIndex) {
        let text = GapBuffer::from_vec(s.as_bytes().to_vec());
        let lines = Indexer::index(&text).lines;
        (text, lines)
    }

    #[test]
    fn indentation_counts_spaces_and_tabs_to_their_stops() {
        let (text, lines) = text("x\n    y\n\t z\n  \n");
        let indent = |line| indentation(&text, lines.line_range(&text, line).0, 8);
        assert_eq!([0, 1, 2, 3, 4].map(indent), [Some(0), Some(4), Some(9), None, None]);
    }

    #[test]
    fn blank_lines_take_the_smaller_level_around_them() {
        let (text, lines) = text("a\n    b\n\n        c\n\n    d\n");
        assert_eq!(guide_levels(&text, &lines, 0..6, 8), [0, 4, 4, 8, 4, 4]);
    }

    #[test]
    fn the_step_is_the_most_common_increase() {
        let levels = [Some(0), Some(4), None, Some(8), Some(4), Some(8), Some(9), Some(0), Some(2)];
        assert_eq!(indent_step(&levels, 8), 4);
        assert_eq!(indent_step(&[Some(0), Some(0), Some(3)], 8), 3, "one increase");
        assert_eq!(indent_step(&[Some(2), Some(2)], 8), 2, "no increase: the smallest level");
        assert_eq!(indent_step(&[Some(0), None], 8), 8, "no indentation: the fallback");
        assert_eq!(indent_step(&[Some(0), Some(1)], 8), 8, "a step of one makes no guides");
    }
}
