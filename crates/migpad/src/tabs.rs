//! The tabs of a window as a list, apart from GPUI: their order, the active one, moving and
//! closing them; and the tabs closed lately, to open again.

use std::collections::VecDeque;

/// The tabs of a window, never none, and which one is active.
#[derive(Debug)]
pub struct Tabs<T> {
    items: Vec<T>,
    active: usize,
}

impl<T> Tabs<T> {
    /// One tab, active.
    pub fn new(first: T) -> Self {
        Tabs { items: vec![first], active: 0 }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, T> {
        self.items.iter()
    }

    pub fn active_index(&self) -> usize {
        self.active
    }

    pub fn active(&self) -> &T {
        &self.items[self.active]
    }

    /// The first tab that `matches`.
    pub fn position(&self, matches: impl FnMut(&T) -> bool) -> Option<usize> {
        self.items.iter().position(matches)
    }

    /// Adds a tab at the end and makes it active; returns its index.
    pub fn push(&mut self, item: T) -> usize {
        self.items.push(item);
        self.active = self.items.len() - 1;
        self.active
    }

    /// Makes the tab at `index` active, if there is one.
    pub fn activate(&mut self, index: usize) -> bool {
        let exists = index < self.items.len();
        if exists {
            self.active = index;
        }
        exists
    }

    /// Makes the next tab active, the first after the last.
    pub fn activate_next(&mut self) {
        self.active = (self.active + 1) % self.items.len();
    }

    /// Makes the previous tab active, the last before the first.
    pub fn activate_previous(&mut self) {
        self.active = (self.active + self.items.len() - 1) % self.items.len();
    }

    /// Closes the tab at `index` and returns it, unless it is the last one: a window has a tab.
    /// The tab to the right of a closed active one becomes active, or the one to the left at
    /// the end.
    pub fn remove(&mut self, index: usize) -> Option<T> {
        if index >= self.items.len() || self.items.len() == 1 {
            return None;
        }
        let item = self.items.remove(index);
        if index < self.active || self.active == self.items.len() {
            self.active -= 1;
        }
        Some(item)
    }

    /// Moves the tab at `from` to `to`, shifting the tabs between; the active tab stays active.
    pub fn move_tab(&mut self, from: usize, to: usize) {
        let len = self.items.len();
        if from >= len || from == to {
            return;
        }
        let to = to.min(len - 1);
        let item = self.items.remove(from);
        self.items.insert(to, item);
        self.active = if self.active == from {
            to
        } else if from < self.active && to >= self.active {
            self.active - 1
        } else if from > self.active && to <= self.active {
            self.active + 1
        } else {
            self.active
        };
    }
}

/// Tabs closed lately, the last one first; the oldest are forgotten past [`ClosedTabs::LIMIT`].
#[derive(Debug)]
pub struct ClosedTabs<T> {
    items: VecDeque<T>,
}

impl<T> Default for ClosedTabs<T> {
    fn default() -> Self {
        ClosedTabs { items: VecDeque::new() }
    }
}

impl<T> ClosedTabs<T> {
    pub const LIMIT: usize = 20;

    pub fn push(&mut self, item: T) {
        if self.items.len() == Self::LIMIT {
            self.items.pop_back();
        }
        self.items.push_front(item);
    }

    /// The tab closed last.
    pub fn pop(&mut self) -> Option<T> {
        self.items.pop_front()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// The lowest number from 1 that `used` does not have: untitled documents are "Untitled",
/// "Untitled 2", and so on, the gaps filled first.
pub fn lowest_free(used: impl IntoIterator<Item = u32>) -> u32 {
    let mut used: Vec<u32> = used.into_iter().collect();
    used.sort_unstable();
    used.dedup();
    let mut free = 1;
    for number in used {
        if number == free {
            free += 1;
        } else if number > free {
            break;
        }
    }
    free
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tabs(names: &[&'static str]) -> Tabs<&'static str> {
        let mut tabs = Tabs::new(names[0]);
        for &name in &names[1..] {
            tabs.push(name);
        }
        tabs
    }

    fn names(tabs: &Tabs<&'static str>) -> Vec<&'static str> {
        tabs.iter().copied().collect()
    }

    #[test]
    fn new_tabs_go_to_the_end_and_become_active() {
        let mut tabs = Tabs::new("a");
        assert_eq!((tabs.len(), *tabs.active()), (1, "a"));
        assert_eq!(tabs.push("b"), 1);
        assert_eq!(tabs.push("c"), 2);
        assert_eq!(names(&tabs), ["a", "b", "c"]);
        assert_eq!(*tabs.active(), "c");
        assert!(tabs.activate(0));
        assert!(!tabs.activate(3));
        assert_eq!(*tabs.active(), "a");
    }

    #[test]
    fn next_and_previous_go_round() {
        let mut tabs = tabs(&["a", "b", "c"]);
        tabs.activate_next();
        assert_eq!(*tabs.active(), "a");
        tabs.activate_previous();
        assert_eq!(*tabs.active(), "c");
        tabs.activate_previous();
        assert_eq!(*tabs.active(), "b");
    }

    #[test]
    fn closing_the_active_tab_activates_the_one_to_the_right() {
        let mut tabs = tabs(&["a", "b", "c", "d"]);
        tabs.activate(1);
        assert_eq!(tabs.remove(1), Some("b"));
        assert_eq!((names(&tabs), *tabs.active()), (vec!["a", "c", "d"], "c"));
        // The last one: the one to the left.
        tabs.activate(2);
        assert_eq!(tabs.remove(2), Some("d"));
        assert_eq!(*tabs.active(), "c");
        // Another tab: the active one stays.
        assert_eq!(tabs.remove(0), Some("a"));
        assert_eq!((names(&tabs), *tabs.active()), (vec!["c"], "c"));
        // The only tab stays.
        assert_eq!(tabs.remove(0), None);
        assert_eq!(tabs.remove(5), None);
    }

    #[test]
    fn closing_a_tab_before_the_active_one_keeps_it_active() {
        let mut tabs = tabs(&["a", "b", "c"]);
        tabs.activate(2);
        tabs.remove(0);
        assert_eq!(*tabs.active(), "c");
    }

    #[test]
    fn moved_tabs_keep_the_active_one() {
        let mut tabs = tabs(&["a", "b", "c", "d"]);
        tabs.activate(1);
        tabs.move_tab(1, 3);
        assert_eq!((names(&tabs), *tabs.active()), (vec!["a", "c", "d", "b"], "b"));
        tabs.move_tab(0, 2);
        assert_eq!((names(&tabs), *tabs.active()), (vec!["c", "d", "a", "b"], "b"));
        tabs.move_tab(3, 0);
        assert_eq!((names(&tabs), *tabs.active()), (vec!["b", "c", "d", "a"], "b"));
        tabs.move_tab(2, 0);
        assert_eq!((names(&tabs), *tabs.active()), (vec!["d", "b", "c", "a"], "b"));
        // Past the end: to the end.
        tabs.move_tab(0, 9);
        assert_eq!((names(&tabs), *tabs.active()), (vec!["b", "c", "a", "d"], "b"));
    }

    #[test]
    fn closed_tabs_come_back_last_first() {
        let mut closed = ClosedTabs::default();
        assert!(closed.is_empty());
        for i in 0..ClosedTabs::<usize>::LIMIT + 5 {
            closed.push(i);
        }
        assert_eq!(closed.pop(), Some(ClosedTabs::<usize>::LIMIT + 4));
        let rest: Vec<usize> = std::iter::from_fn(|| closed.pop()).collect();
        assert_eq!(rest.len(), ClosedTabs::<usize>::LIMIT - 1);
        assert_eq!(rest.last(), Some(&5), "the oldest are forgotten");
    }

    #[test]
    fn untitled_numbers_fill_the_gaps() {
        assert_eq!(lowest_free([]), 1);
        assert_eq!(lowest_free([1, 2]), 3);
        assert_eq!(lowest_free([2, 3]), 1);
        assert_eq!(lowest_free([3, 1, 1, 5]), 2);
    }
}
