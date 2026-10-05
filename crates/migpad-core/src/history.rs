//! Undo history: transactions of edits, merged while the user keeps typing or deleting, and
//! limited in depth.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Undo keeps at most this many steps.
pub const MAX_STEPS: usize = 1000;

/// Undo keeps at most this many bytes of deleted and inserted text; the last step is kept anyway.
pub const MAX_BYTES: usize = 256 << 20;

/// A pause this long ends a step of typing or deleting.
pub const MERGE_PAUSE: Duration = Duration::from_secs(1);

/// One replacement: `deleted` at `pos` became `inserted`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Edit {
    pub pos: usize,
    pub deleted: Vec<u8>,
    pub inserted: Vec<u8>,
}

impl Edit {
    fn breaks_line(&self) -> bool {
        let breaks = |bytes: &[u8]| bytes.iter().any(|&b| b == b'\n' || b == b'\r');
        breaks(&self.deleted) || breaks(&self.inserted)
    }

    fn size(&self) -> usize {
        self.deleted.len() + self.inserted.len()
    }
}

/// The selection, from `anchor` to `head` where the caret is; byte offsets. An empty selection
/// is just the caret.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
}

impl Selection {
    /// The caret at `pos`, nothing selected.
    pub fn caret(pos: usize) -> Self {
        Selection { anchor: pos, head: pos }
    }
}

/// What a transaction does, for merging typing and deleting into single undo steps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKind {
    /// Typing text, possibly over a selection.
    Typing,
    /// Backspace or Delete.
    Deleting,
    /// Anything else: paste, replace, IME composition; never merged.
    Other,
}

/// Edits undone and redone as one step, applied in order, each to the text the previous one left;
/// and the selections before and after them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transaction {
    pub edits: Vec<Edit>,
    pub before: Selection,
    pub after: Selection,
}

impl Transaction {
    fn size(&self) -> usize {
        self.edits.iter().map(Edit::size).sum()
    }
}

/// Undo and redo steps of a document.
#[derive(Debug)]
pub struct History {
    done: VecDeque<Step>,
    /// Undone steps; the last one is redone first.
    undone: Vec<Step>,
    /// Bytes of edits in `done` and `undone`.
    bytes: usize,
    /// The last step takes no more edits.
    sealed: bool,
    next_id: u64,
    /// The state of the text before the oldest step kept, see [`History::state`].
    bottom: u64,
    max_steps: usize,
    max_bytes: usize,
}

#[derive(Debug)]
struct Step {
    /// Tells steps apart; the state of the text after the step is `id + 1`, see [`History::state`].
    id: u64,
    transaction: Transaction,
    kind: EditKind,
    /// When an edit last joined the step.
    time: Instant,
}

impl History {
    pub fn new() -> Self {
        Self::with_limits(MAX_STEPS, MAX_BYTES)
    }

    fn with_limits(max_steps: usize, max_bytes: usize) -> Self {
        History {
            done: VecDeque::new(),
            undone: Vec::new(),
            bytes: 0,
            sealed: true,
            next_id: 0,
            bottom: 0,
            max_steps,
            max_bytes,
        }
    }

    /// Records a transaction that has just been applied, and drops what can no longer be redone.
    /// Typing and deleting continue the last step if they directly follow it, see
    /// [`History::can_merge`]; returns whether the transaction joined the last step.
    pub fn record(&mut self, transaction: Transaction, kind: EditKind, now: Instant) -> bool {
        let merged = self.can_merge(&transaction, kind, now);
        // A line break ends the step: the next edit starts a new one.
        let line_break = breaks_line(&transaction);
        self.push(transaction, kind, now, merged);
        self.sealed = line_break;
        merged
    }

    /// Whether [`History::record`] would join `transaction` to the last step: the step is not
    /// sealed, neither of them breaks a line, they are of the same kind, typing or deleting, with
    /// no pause of [`MERGE_PAUSE`] between them, and the transaction continues where the step
    /// left the selection.
    pub fn can_merge(&self, transaction: &Transaction, kind: EditKind, now: Instant) -> bool {
        let Some(step) = self.done.back() else { return false };
        !self.sealed
            && !breaks_line(transaction)
            && kind == step.kind
            && now.saturating_duration_since(step.time) < MERGE_PAUSE
            && transaction.before == step.transaction.after
            && joinable(&step.transaction, transaction, kind)
    }

    /// Records a transaction again from the journal, where `merged` tells whether it joined
    /// the last step; time plays no part. Returns `false` if it cannot join that step.
    pub fn replay(&mut self, transaction: Transaction, kind: EditKind, merged: bool) -> bool {
        if merged && !self.done.back().is_some_and(|step| joinable(&step.transaction, &transaction, kind)) {
            return false;
        }
        self.push(transaction, kind, Instant::now(), merged);
        self.sealed = true;
        true
    }

    /// Ends the last step: the next edit starts a new one. The editor calls it when the caret
    /// moves on its own or the document loses focus.
    pub fn seal(&mut self) {
        self.sealed = true;
    }

    pub fn can_undo(&self) -> bool {
        !self.done.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }

    /// Identifies the state of the text: equal states mean equal text, through undo and redo.
    /// The document compares it with the state it was saved in. A step joined by typing changes
    /// the text but not the state, so a saved state must be sealed.
    pub fn state(&self) -> u64 {
        self.done.back().map_or(self.bottom, |step| step.id + 1)
    }

    /// The position of the current state in the linear history — the states before the oldest
    /// step, after each done step, after each step that can be redone: the number of done steps.
    pub fn position(&self) -> u64 {
        self.done.len() as u64
    }

    /// The state at a position of the linear history, see [`History::position`].
    pub fn state_at(&self, position: u64) -> Option<u64> {
        let position = usize::try_from(position).ok()?;
        if position == 0 {
            return Some(self.bottom);
        }
        let step = match self.done.get(position - 1) {
            Some(step) => step,
            None => self.undone.iter().rev().nth(position - 1 - self.done.len())?,
        };
        Some(step.id + 1)
    }

    /// The position of a state in the linear history, if it can still be reached.
    pub fn position_of(&self, state: u64) -> Option<u64> {
        (0..=(self.done.len() + self.undone.len()) as u64).find(|&position| self.state_at(position) == Some(state))
    }

    /// Moves the last step to the redo list and returns it; the caller reverts its edits,
    /// last first.
    pub fn undo(&mut self) -> Option<&Transaction> {
        let step = self.done.pop_back()?;
        self.sealed = true;
        self.undone.push(step);
        self.undone.last().map(|step| &step.transaction)
    }

    /// Moves the last undone step back and returns it; the caller applies its edits again.
    pub fn redo(&mut self) -> Option<&Transaction> {
        let step = self.undone.pop()?;
        self.sealed = true;
        self.done.push_back(step);
        self.done.back().map(|step| &step.transaction)
    }

    /// The steps that can be undone, oldest first.
    pub fn done_steps(&self) -> impl Iterator<Item = (&Transaction, EditKind)> {
        self.done.iter().map(|step| (&step.transaction, step.kind))
    }

    /// The steps that can be redone, in the order they would be: recording them in this order and
    /// then undoing as many recreates the redo list.
    pub fn undone_steps(&self) -> impl Iterator<Item = (&Transaction, EditKind)> {
        self.undone.iter().rev().map(|step| (&step.transaction, step.kind))
    }

    /// Adds a new step, or joins the transaction to the last one; drops the redo list.
    fn push(&mut self, transaction: Transaction, kind: EditKind, now: Instant, merged: bool) {
        self.bytes -= self.undone.drain(..).map(|step| step.transaction.size()).sum::<usize>();
        self.bytes += transaction.size();
        if merged {
            let step = self.done.back_mut().expect("a step to join");
            join(&mut step.transaction, &transaction, kind);
            step.time = now;
        } else {
            let id = self.next_id;
            self.next_id += 1;
            self.done.push_back(Step { id, transaction, kind, time: now });
        }
        self.trim();
    }

    /// Drops the oldest steps beyond the limits, but never the last one.
    fn trim(&mut self) {
        while self.done.len() > 1 && (self.done.len() > self.max_steps || self.bytes > self.max_bytes) {
            let step = self.done.pop_front().expect("more than one step");
            self.bytes -= step.transaction.size();
            self.bottom = step.id + 1;
        }
    }
}

fn breaks_line(transaction: &Transaction) -> bool {
    transaction.edits.iter().any(Edit::breaks_line)
}

/// Whether the single edit of `transaction` continues typing or deleting where the single edit
/// of `step` left off.
fn joinable(step: &Transaction, transaction: &Transaction, kind: EditKind) -> bool {
    let ([edit], [last]) = (transaction.edits.as_slice(), step.edits.as_slice()) else {
        return false;
    };
    match kind {
        // Typing right after the text typed so far.
        EditKind::Typing => edit.deleted.is_empty() && edit.pos == last.pos + last.inserted.len(),
        // Backspace right before the text deleted so far, or Delete at the same place.
        EditKind::Deleting => {
            edit.inserted.is_empty()
                && last.inserted.is_empty()
                && (edit.pos + edit.deleted.len() == last.pos || edit.pos == last.pos)
        }
        // Other edits never join.
        EditKind::Other => false,
    }
}

/// Appends the single edit of `transaction` to that of `step`; they must be [`joinable`].
fn join(step: &mut Transaction, transaction: &Transaction, kind: EditKind) {
    let ([edit], [last]) = (transaction.edits.as_slice(), step.edits.as_mut_slice()) else {
        unreachable!("joinable transactions have one edit each");
    };
    match kind {
        EditKind::Typing => last.inserted.extend_from_slice(&edit.inserted),
        // Delete at the same place grows to the right.
        EditKind::Deleting if edit.pos == last.pos => last.deleted.extend_from_slice(&edit.deleted),
        // Backspace grows to the left.
        EditKind::Deleting => {
            last.deleted.splice(0..0, edit.deleted.iter().copied());
            last.pos = edit.pos;
        }
        EditKind::Other => unreachable!("other edits never join"),
    }
    step.after = transaction.after;
}

impl Default for History {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn insert(pos: usize, text: &str) -> Transaction {
        let edit = Edit { pos, deleted: Vec::new(), inserted: text.into() };
        let end = pos + text.len();
        Transaction { edits: vec![edit], before: Selection::caret(pos), after: Selection::caret(end) }
    }

    fn backspace(pos: usize, deleted: &str) -> Transaction {
        let start = pos - deleted.len();
        let edit = Edit { pos: start, deleted: deleted.into(), inserted: Vec::new() };
        Transaction { edits: vec![edit], before: Selection::caret(pos), after: Selection::caret(start) }
    }

    fn delete(pos: usize, deleted: &str) -> Transaction {
        let edit = Edit { pos, deleted: deleted.into(), inserted: Vec::new() };
        Transaction { edits: vec![edit], before: Selection::caret(pos), after: Selection::caret(pos) }
    }

    /// Records transactions 100 ms apart; returns which ones merged.
    fn record_all(history: &mut History, steps: Vec<(Transaction, EditKind)>) -> Vec<bool> {
        let start = Instant::now();
        steps
            .into_iter()
            .enumerate()
            .map(|(i, (t, kind))| history.record(t, kind, start + Duration::from_millis(100 * i as u64)))
            .collect()
    }

    fn undo_edits(history: &mut History) -> Vec<Edit> {
        history.undo().unwrap().edits.clone()
    }

    #[test]
    fn typing_merges_into_one_step() {
        let mut history = History::new();
        let typed = vec![
            (insert(0, "a"), EditKind::Typing),
            (insert(1, "b"), EditKind::Typing),
            (insert(2, "c"), EditKind::Typing),
        ];
        assert_eq!(record_all(&mut history, typed), [false, true, true]);
        assert_eq!(undo_edits(&mut history), [Edit { pos: 0, deleted: vec![], inserted: b"abc".to_vec() }]);
        assert!(!history.can_undo());
    }

    #[test]
    fn typing_over_a_selection_then_typing_is_one_step() {
        let mut history = History::new();
        let over = Transaction {
            edits: vec![Edit { pos: 2, deleted: b"sel".to_vec(), inserted: b"x".to_vec() }],
            before: Selection { anchor: 2, head: 5 },
            after: Selection::caret(3),
        };
        assert_eq!(
            record_all(&mut history, vec![(over, EditKind::Typing), (insert(3, "y"), EditKind::Typing)]),
            [false, true]
        );
        assert_eq!(undo_edits(&mut history), [Edit { pos: 2, deleted: b"sel".to_vec(), inserted: b"xy".to_vec() }]);
    }

    #[test]
    fn backspace_and_delete_merge_with_their_own_kind() {
        let mut history = History::new();
        let steps = vec![(backspace(5, "e"), EditKind::Deleting), (backspace(4, "d"), EditKind::Deleting)];
        assert_eq!(record_all(&mut history, steps), [false, true]);
        assert_eq!(undo_edits(&mut history), [Edit { pos: 3, deleted: b"de".to_vec(), inserted: vec![] }]);

        let mut history = History::new();
        let steps = vec![(delete(2, "c"), EditKind::Deleting), (delete(2, "d"), EditKind::Deleting)];
        assert_eq!(record_all(&mut history, steps), [false, true]);
        assert_eq!(undo_edits(&mut history), [Edit { pos: 2, deleted: b"cd".to_vec(), inserted: vec![] }]);
    }

    #[test]
    fn steps_end_on_line_breaks_kind_changes_and_caret_moves() {
        let mut history = History::new();
        let steps = vec![
            (insert(0, "a"), EditKind::Typing),
            (insert(1, "\n"), EditKind::Typing),     // a line break starts a step
            (insert(2, "b"), EditKind::Typing),      // and ends it
            (backspace(3, "b"), EditKind::Deleting), // another kind
            (insert(5, "c"), EditKind::Typing),      // the caret moved
            (insert(6, "d"), EditKind::Other),       // never merged
        ];
        assert_eq!(record_all(&mut history, steps), [false, false, false, false, false, false]);
    }

    #[test]
    fn steps_end_after_a_pause_or_when_sealed() {
        let mut history = History::new();
        let start = Instant::now();
        assert!(!history.record(insert(0, "a"), EditKind::Typing, start));
        assert!(!history.record(insert(1, "b"), EditKind::Typing, start + MERGE_PAUSE));
        history.seal();
        assert!(!history.record(insert(2, "c"), EditKind::Typing, start + MERGE_PAUSE));
    }

    #[test]
    fn undo_and_redo_move_steps_between_the_lists() {
        let mut history = History::new();
        let steps = vec![(insert(0, "a"), EditKind::Other), (insert(1, "b"), EditKind::Other)];
        record_all(&mut history, steps);
        assert_eq!(history.undo().unwrap().after, Selection::caret(2));
        assert_eq!(history.undo().unwrap().after, Selection::caret(1));
        assert!(history.undo().is_none());
        assert_eq!(history.redo().unwrap().after, Selection::caret(1));
        assert!(history.can_redo());
        // A new edit drops what is left to redo, and does not merge into the redone step.
        assert!(!history.record(insert(1, "x"), EditKind::Typing, Instant::now()));
        assert!(!history.can_redo());
        assert_eq!(history.bytes, 2);
    }

    #[test]
    fn states_follow_undo_redo_and_trimming() {
        let mut history = History::with_limits(2, 1000);
        let start = history.state();
        history.record(insert(0, "a"), EditKind::Other, Instant::now());
        let after_a = history.state();
        history.record(insert(1, "b"), EditKind::Other, Instant::now());
        assert_ne!(history.state(), after_a);
        history.undo();
        assert_eq!(history.state(), after_a);
        history.redo();
        let after_b = history.state();
        assert_eq!((history.position(), history.state_at(1), history.state_at(2)), (2, Some(after_a), Some(after_b)));
        history.undo();
        assert_eq!(
            (history.position(), history.state_at(2), history.position_of(after_b)),
            (1, Some(after_b), Some(2))
        );
        history.redo();
        // A third step drops the first one: the state before it can no longer be reached.
        history.record(insert(2, "c"), EditKind::Other, Instant::now());
        history.undo();
        history.undo();
        assert_eq!(history.state(), after_a, "all undone, the text keeps the dropped step");
        assert_ne!(history.state(), start);
        assert_eq!(history.position_of(start), None);
    }

    #[test]
    fn depth_is_limited_by_steps_and_bytes() {
        let mut history = History::with_limits(3, 1000);
        for i in 0..5 {
            history.record(insert(i, "x"), EditKind::Other, Instant::now());
        }
        assert_eq!(history.done.len(), 3);
        assert_eq!(history.done.front().unwrap().transaction.edits[0].pos, 2);

        let mut history = History::with_limits(100, 10);
        history.record(insert(0, "12345"), EditKind::Other, Instant::now());
        history.record(insert(5, "67890"), EditKind::Other, Instant::now());
        assert_eq!(history.done.len(), 2);
        history.record(insert(10, "x"), EditKind::Other, Instant::now());
        assert_eq!((history.done.len(), history.bytes), (2, 6));
        // The last step stays even when it alone is over the limit.
        history.record(insert(11, "a long insertion"), EditKind::Other, Instant::now());
        assert_eq!((history.done.len(), history.bytes), (1, 16));
    }
}
