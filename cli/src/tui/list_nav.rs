//! Shared `ListState` navigation helpers used by the TUI's list widgets.

use ratatui::widgets::ListState;

/// Select the next item, wrapping around to the first. No-op when `len == 0`.
pub fn select_next(state: &mut ListState, len: usize) {
    if len == 0 {
        return;
    }
    let i = state.selected().map(|i| (i + 1) % len).unwrap_or(0);
    state.select(Some(i));
}

/// Select the previous item, wrapping around to the last. No-op when `len == 0`.
pub fn select_prev(state: &mut ListState, len: usize) {
    if len == 0 {
        return;
    }
    let i = state
        .selected()
        .map(|i| if i == 0 { len - 1 } else { i - 1 })
        .unwrap_or(0);
    state.select(Some(i));
}

/// Reset selection to the first item when the list is non-empty, or clear it
/// otherwise. Shared by every widget's `set_items` after it replaces its item
/// list — the one part of `set_items` that's identical across widgets
/// regardless of what other per-widget fields (`total`, `cwd`, ...) it also sets.
pub fn select_first_or_none(state: &mut ListState, len: usize) {
    state.select(if len > 0 { Some(0) } else { None });
}

/// The `items: Vec<T>` + `state: ListState` shape shared by every TUI list
/// widget, with the `set_items`/`next`/`prev`/`selected` behavior that goes
/// with it. Widgets needing extra bookkeeping (`total`, `cwd`, ...) hold a
/// `ListNav<T>` field alongside those, rather than reimplementing this shape.
///
/// `PackageListWidget` is the one exception: its `next`/`prev`/`selected`
/// operate over a search-filtered subset of `items`, so it drives `state`
/// directly through [`select_next`]/[`select_prev`] instead of delegating to
/// this type's methods.
pub struct ListNav<T> {
    pub items: Vec<T>,
    pub state: ListState,
}

// Not `#[derive(Default)]`: that would require `T: Default`, which none of
// the `ProjectDetection`/`PackageSummary`/`RegistryInfo` item types implement
// (an empty `items: Vec<T>` needs no bound on `T` at all).
impl<T> Default for ListNav<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            state: ListState::default(),
        }
    }
}

impl<T> ListNav<T> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_items(&mut self, items: Vec<T>) {
        self.items = items;
        select_first_or_none(&mut self.state, self.items.len());
    }

    pub fn next(&mut self) {
        select_next(&mut self.state, self.items.len());
    }

    pub fn prev(&mut self) {
        select_prev(&mut self.state, self.items.len());
    }

    pub fn selected(&self) -> Option<&T> {
        self.state.selected().and_then(|i| self.items.get(i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_next_on_empty_list_is_noop() {
        let mut state = ListState::default();
        select_next(&mut state, 0);
        assert_eq!(state.selected(), None);
    }

    #[test]
    fn select_prev_on_empty_list_is_noop() {
        let mut state = ListState::default();
        select_prev(&mut state, 0);
        assert_eq!(state.selected(), None);
    }

    #[test]
    fn select_next_on_single_item_list_stays_at_zero() {
        let mut state = ListState::default();
        select_next(&mut state, 1);
        assert_eq!(state.selected(), Some(0));
        select_next(&mut state, 1);
        assert_eq!(state.selected(), Some(0));
    }

    #[test]
    fn select_prev_on_single_item_list_stays_at_zero() {
        let mut state = ListState::default();
        select_prev(&mut state, 1);
        assert_eq!(state.selected(), Some(0));
        select_prev(&mut state, 1);
        assert_eq!(state.selected(), Some(0));
    }

    #[test]
    fn select_next_wraps_forward_past_last_index() {
        let mut state = ListState::default();
        state.select(Some(2));
        select_next(&mut state, 3);
        assert_eq!(state.selected(), Some(0));
    }

    #[test]
    fn select_prev_wraps_backward_past_first_index() {
        let mut state = ListState::default();
        state.select(Some(0));
        select_prev(&mut state, 3);
        assert_eq!(state.selected(), Some(2));
    }

    #[test]
    fn select_next_from_no_selection_selects_first() {
        let mut state = ListState::default();
        select_next(&mut state, 3);
        assert_eq!(state.selected(), Some(0));
    }

    #[test]
    fn select_prev_from_no_selection_selects_first() {
        let mut state = ListState::default();
        select_prev(&mut state, 3);
        assert_eq!(state.selected(), Some(0));
    }

    #[test]
    fn select_first_or_none_selects_zero_when_non_empty() {
        let mut state = ListState::default();
        state.select(Some(5));
        select_first_or_none(&mut state, 3);
        assert_eq!(state.selected(), Some(0));
    }

    #[test]
    fn select_first_or_none_clears_when_empty() {
        let mut state = ListState::default();
        state.select(Some(0));
        select_first_or_none(&mut state, 0);
        assert_eq!(state.selected(), None);
    }

    #[test]
    fn list_nav_set_items_selects_first_and_next_prev_wrap() {
        let mut nav: ListNav<i32> = ListNav::new();
        assert_eq!(nav.selected(), None);

        nav.set_items(vec![10, 20, 30]);
        assert_eq!(nav.selected(), Some(&10));

        nav.next();
        assert_eq!(nav.selected(), Some(&20));
        nav.prev();
        assert_eq!(nav.selected(), Some(&10));
        nav.prev();
        assert_eq!(nav.selected(), Some(&30));
    }

    #[test]
    fn list_nav_set_items_with_empty_vec_clears_selection() {
        let mut nav: ListNav<i32> = ListNav::new();
        nav.set_items(vec![1, 2]);
        nav.set_items(vec![]);
        assert_eq!(nav.selected(), None);
    }
}
