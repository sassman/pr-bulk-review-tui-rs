use crate::{actions::Action, effect::Effect, reducer::reduce, state::AppState};

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
    /// Returns a vector of effects to be executed by the caller.
    ///
    /// # Example
    /// ```
    /// let effects = store.dispatch(Action::ToggleShortcutsPanel);
    /// // Execute effects...
    /// ```
    pub fn dispatch(&mut self, action: Action) -> Vec<Effect> {
        // Apply reducer to get new state and effects
        let (new_state, effects) = reduce(self.state.clone(), &action);

        // Replace old state with new state
        self.state = new_state;

        // Return effects to be executed by caller
        effects
    }

    /// Dispatch an action by reference (useful when action should not be moved)
    /// Returns a vector of effects to be executed by the caller.
    pub fn dispatch_ref(&mut self, action: &Action) -> Vec<Effect> {
        let (new_state, effects) = reduce(self.state.clone(), action);
        self.state = new_state;
        effects
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

    #[test]
    fn test_store_dispatch_quit() {
        let mut store = Store::default();
        assert!(!store.state().ui.should_quit);

        let _effects = store.dispatch(Action::Quit);
        assert!(store.state().ui.should_quit);
    }

    #[test]
    fn test_store_dispatch_toggle_shortcuts() {
        let mut store = Store::default();
        assert!(!store.state().ui.show_shortcuts);

        let _effects = store.dispatch(Action::ToggleShortcuts);
        assert!(store.state().ui.show_shortcuts);

        let _effects = store.dispatch(Action::ToggleShortcuts);
        assert!(!store.state().ui.show_shortcuts);
    }
}
