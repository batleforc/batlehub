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
