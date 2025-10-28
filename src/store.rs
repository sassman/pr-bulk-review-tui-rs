use crate::{
    reducer::reduce,
    shortcuts::Action,
    state::AppState,
};

/// Redux-style Store that holds application state and dispatches actions
///
/// The Store follows the Redux pattern:
/// - Centralized state management
/// - Actions are dispatched to modify state
/// - Pure reducers handle state transitions
/// - State is immutable (replaced on each action)
pub struct Store {
    state: AppState,
}

impl Store {
    /// Create a new store with initial state
    pub fn new(initial_state: AppState) -> Self {
        Self {
            state: initial_state,
        }
    }

    /// Get immutable reference to current state
    pub fn state(&self) -> &AppState {
        &self.state
    }

    /// Get mutable reference to current state
    /// Note: Direct mutation should be avoided - prefer dispatch() for state changes
    pub fn state_mut(&mut self) -> &mut AppState {
        &mut self.state
    }

    /// Dispatch an action to update state
    ///
    /// This is the primary way to modify state. The action is passed to the
    /// root reducer which delegates to appropriate sub-reducers.
    ///
    /// # Example
    /// ```
    /// store.dispatch(Action::ToggleShortcutsPanel);
    /// ```
    pub fn dispatch(&mut self, action: Action) {
        // Apply reducer to get new state
        let new_state = reduce(self.state.clone(), &action);

        // Replace old state with new state
        self.state = new_state;
    }

    /// Dispatch an action by reference (useful when action should not be moved)
    pub fn dispatch_ref(&mut self, action: &Action) {
        let new_state = reduce(self.state.clone(), action);
        self.state = new_state;
    }

    /// Replace entire state (useful for initialization or testing)
    pub fn replace_state(&mut self, state: AppState) {
        self.state = state;
    }
}

impl Default for Store {
    fn default() -> Self {
        Self::new(AppState::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::*;

    #[test]
    fn test_store_dispatch_quit() {
        let mut store = Store::default();
        assert!(!store.state().ui.should_quit);

        store.dispatch(Action::Quit);
        assert!(store.state().ui.should_quit);
    }

    #[test]
    fn test_store_dispatch_toggle_shortcuts() {
        let mut store = Store::default();
        assert!(!store.state().ui.show_shortcuts);

        store.dispatch(Action::ToggleShortcutsPanel);
        assert!(store.state().ui.show_shortcuts);

        store.dispatch(Action::ToggleShortcutsPanel);
        assert!(!store.state().ui.show_shortcuts);
    }

    #[test]
    fn test_store_dispatch_spinner_tick() {
        let mut store = Store::default();
        let initial_frame = store.state().ui.spinner_frame;

        store.dispatch(Action::Tick);
        assert_eq!(store.state().ui.spinner_frame, initial_frame.wrapping_add(1));
    }
}
