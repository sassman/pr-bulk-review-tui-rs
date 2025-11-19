//! Command provider trait and registry system

use std::fmt::Debug;

/// A command that can be executed from the command palette
///
/// Generic over `A` (the action type) to work with any application
#[derive(Debug, Clone)]
pub struct CommandItem<A> {
    /// Display title for the command (e.g., "Merge PR")
    pub title: String,

    /// Longer description (e.g., "Merge selected pull requests")
    pub description: String,

    /// Category for grouping (e.g., "PR Actions", "Navigation")
    pub category: String,

    /// Keyboard shortcut hint (e.g., "m" or "Ctrl+P")
    pub shortcut_hint: Option<String>,

    /// The action to dispatch when this command is executed
    pub action: A,
}

impl<A> CommandItem<A> {
    /// Get searchable text (title + description + category)
    pub fn searchable_text(&self) -> String {
        format!("{} {} {}", self.title, self.description, self.category)
    }
}

/// Trait for providing commands to the palette
///
/// Generic over:
/// - `A`: Action type (what gets dispatched when command is executed)
/// - `S`: State type (used for context-aware filtering)
///
/// Implementors provide a list of commands that are available in a given state.
pub trait CommandProvider<A, S>: Debug {
    /// Get all commands from this provider
    ///
    /// The provider can filter commands based on the current state to ensure
    /// only relevant commands are shown to the user.
    fn commands(&self, state: &S) -> Vec<CommandItem<A>>;

    /// Provider name for debugging
    fn name(&self) -> &str;
}

/// Registry of command providers
///
/// Collects commands from multiple providers and presents them as a unified list.
/// Generic over action type `A` and state type `S`.
pub struct CommandPalette<A, S> {
    providers: Vec<Box<dyn CommandProvider<A, S>>>,
}

impl<A, S> CommandPalette<A, S> {
    /// Create a new empty command palette
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    /// Register a command provider
    ///
    /// Providers are called in the order they were registered.
    pub fn register(&mut self, provider: Box<dyn CommandProvider<A, S>>) {
        self.providers.push(provider);
    }

    /// Get all commands from all providers
    ///
    /// This queries all registered providers and combines their commands
    /// into a single list. The state is passed to each provider to enable
    /// context-aware filtering.
    pub fn all_commands(&self, state: &S) -> Vec<CommandItem<A>> {
        self.providers
            .iter()
            .flat_map(|p| p.commands(state))
            .collect()
    }

    /// Get the number of registered providers
    pub fn provider_count(&self) -> usize {
        self.providers.len()
    }
}

impl<A, S> Default for CommandPalette<A, S> {
    fn default() -> Self {
        Self::new()
    }
}

impl<A, S> Debug for CommandPalette<A, S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CommandPalette")
            .field("provider_count", &self.providers.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    enum TestAction {
        Save,
        Quit,
    }

    #[derive(Debug)]
    struct TestState {
        can_save: bool,
    }

    #[derive(Debug)]
    struct TestProvider;

    impl CommandProvider<TestAction, TestState> for TestProvider {
        fn commands(&self, state: &TestState) -> Vec<CommandItem<TestAction>> {
            let mut commands = vec![
                CommandItem {
                    title: "Quit".into(),
                    description: "Exit the application".into(),
                    category: "General".into(),
                    shortcut_hint: Some("q".into()),
                    action: TestAction::Quit,
                },
            ];

            if state.can_save {
                commands.push(CommandItem {
                    title: "Save".into(),
                    description: "Save the current file".into(),
                    category: "File".into(),
                    shortcut_hint: Some("Ctrl+S".into()),
                    action: TestAction::Save,
                });
            }

            commands
        }

        fn name(&self) -> &str {
            "TestProvider"
        }
    }

    #[test]
    fn test_command_palette_basic() {
        let mut palette = CommandPalette::new();
        assert_eq!(palette.provider_count(), 0);

        palette.register(Box::new(TestProvider));
        assert_eq!(palette.provider_count(), 1);
    }

    #[test]
    fn test_context_aware_filtering() {
        let mut palette = CommandPalette::new();
        palette.register(Box::new(TestProvider));

        // State where saving is not allowed
        let state = TestState { can_save: false };
        let commands = palette.all_commands(&state);
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].action, TestAction::Quit);

        // State where saving is allowed
        let state = TestState { can_save: true };
        let commands = palette.all_commands(&state);
        assert_eq!(commands.len(), 2);
    }

    #[test]
    fn test_searchable_text() {
        let cmd = CommandItem {
            title: "Save".into(),
            description: "Save the current file".into(),
            category: "File".into(),
            shortcut_hint: Some("Ctrl+S".into()),
            action: TestAction::Save,
        };

        let searchable = cmd.searchable_text();
        assert!(searchable.contains("Save"));
        assert!(searchable.contains("current file"));
        assert!(searchable.contains("File"));
    }
}
