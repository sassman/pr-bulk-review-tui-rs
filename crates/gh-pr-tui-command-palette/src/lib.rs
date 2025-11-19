//! Generic command palette infrastructure for TUI applications
//!
//! This crate provides a reusable command palette system with:
//! - Fuzzy search powered by nucleo-matcher
//! - Provider pattern for extensibility
//! - Context-aware filtering
//! - Generic over action types
//!
//! # Example
//!
//! ```rust,ignore
//! use gh_pr_tui_command_palette::{CommandItem, CommandProvider, CommandPalette};
//!
//! // Define your action type
//! #[derive(Clone)]
//! enum MyAction {
//!     Save,
//!     Quit,
//! }
//!
//! // Create a provider
//! struct MyCommandProvider;
//!
//! impl CommandProvider<MyAction, MyState> for MyCommandProvider {
//!     fn commands(&self, state: &MyState) -> Vec<CommandItem<MyAction>> {
//!         vec![
//!             CommandItem {
//!                 title: "Save".into(),
//!                 description: "Save the current file".into(),
//!                 category: "File".into(),
//!                 shortcut_hint: Some("Ctrl+S".into()),
//!                 action: MyAction::Save,
//!             },
//!         ]
//!     }
//!
//!     fn name(&self) -> &str {
//!         "MyCommands"
//!     }
//! }
//! ```

mod provider;
mod search;

pub use provider::{CommandItem, CommandProvider, CommandPalette};
pub use search::filter_commands;
