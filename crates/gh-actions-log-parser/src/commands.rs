//! GitHub Actions workflow command parsing
//!
//! Parses workflow commands like ::group::, ::error::, ::warning::, etc.

use crate::types::{CommandParams, WorkflowCommand};
use regex::Regex;
use std::sync::OnceLock;

/// Parse a line for GitHub Actions workflow commands
///
/// Supports both formats:
/// - Legacy: `::command params::message` or `::command::message`
/// - Modern: `##[command]message` or `[command]message`
///
/// Returns `Some((command, cleaned_line))` if a command is found, where `cleaned_line`
/// is the line with the command syntax removed. Returns `None` if no command is present.
pub fn parse_command(line: &str) -> Option<(WorkflowCommand, String)> {
    // Try modern ##[...] syntax first
    if let Some(result) = parse_hash_bracket_command(line) {
        return Some(result);
    }

    // Try [command] prefix
    if let Some(result) = parse_command_prefix(line) {
        return Some(result);
    }

    // Fall back to legacy ::command:: syntax
    parse_legacy_command(line)
}

/// Parse legacy ::command:: syntax
fn parse_legacy_command(line: &str) -> Option<(WorkflowCommand, String)> {
    static COMMAND_REGEX: OnceLock<Regex> = OnceLock::new();

    let re = COMMAND_REGEX.get_or_init(|| {
        // Match ::command params::message or ::command::message
        Regex::new(r"^::([a-zA-Z-]+)(?:\s+([^:]+?))?::(.*)$").unwrap()
    });

    let captures = re.captures(line.trim())?;
    let command_name = captures.get(1)?.as_str();
    let params_str = captures.get(2).map(|m| m.as_str());
    let message = captures.get(3)?.as_str().to_string();

    let command = match command_name.to_lowercase().as_str() {
        "group" => WorkflowCommand::GroupStart {
            title: message.clone(),
        },
        "endgroup" => WorkflowCommand::GroupEnd,
        "error" => {
            let params = parse_params(params_str.unwrap_or(""));
            WorkflowCommand::Error {
                message: message.clone(),
                params,
            }
        }
        "warning" => {
            let params = parse_params(params_str.unwrap_or(""));
            WorkflowCommand::Warning {
                message: message.clone(),
                params,
            }
        }
        "debug" => WorkflowCommand::Debug {
            message: message.clone(),
        },
        "notice" => {
            let params = parse_params(params_str.unwrap_or(""));
            WorkflowCommand::Notice {
                message: message.clone(),
                params,
            }
        }
        _ => return None, // Unknown command
    };

    // Return command and the message part (cleaned of command syntax)
    Some((command, message))
}

/// Parse modern ##[command]message syntax
fn parse_hash_bracket_command(line: &str) -> Option<(WorkflowCommand, String)> {
    static HASH_BRACKET_REGEX: OnceLock<Regex> = OnceLock::new();

    let re = HASH_BRACKET_REGEX.get_or_init(|| {
        // Match ##[command]message
        Regex::new(r"^##\[([a-zA-Z-]+)\](.*)$").unwrap()
    });

    let captures = re.captures(line.trim())?;
    let command_name = captures.get(1)?.as_str();
    let message = captures.get(2)?.as_str().trim().to_string();

    let command = match command_name.to_lowercase().as_str() {
        "group" => WorkflowCommand::GroupStart {
            title: message.clone(),
        },
        "endgroup" => WorkflowCommand::GroupEnd,
        "error" => WorkflowCommand::Error {
            message: message.clone(),
            params: CommandParams::default(),
        },
        "warning" => WorkflowCommand::Warning {
            message: message.clone(),
            params: CommandParams::default(),
        },
        "debug" => WorkflowCommand::Debug {
            message: message.clone(),
        },
        "notice" => WorkflowCommand::Notice {
            message: message.clone(),
            params: CommandParams::default(),
        },
        _ => return None, // Unknown command
    };

    Some((command, message))
}

/// Parse [command] prefix (strips the prefix but doesn't create a command object)
fn parse_command_prefix(line: &str) -> Option<(WorkflowCommand, String)> {
    static COMMAND_PREFIX_REGEX: OnceLock<Regex> = OnceLock::new();

    let re = COMMAND_PREFIX_REGEX.get_or_init(|| {
        // Match [command]actual_command_line
        Regex::new(r"^\[command\](.*)$").unwrap()
    });

    let captures = re.captures(line.trim())?;
    let command_line = captures.get(1)?.as_str().trim().to_string();

    // [command] is just a marker, not a workflow command
    // Return a Debug command with the cleaned line
    Some((
        WorkflowCommand::Debug {
            message: command_line.clone(),
        },
        command_line,
    ))
}

/// Parse command parameters like "file=foo.rs,line=42,col=10"
fn parse_params(params_str: &str) -> CommandParams {
    let mut params = CommandParams::default();

    for param in params_str.split(',') {
        let param = param.trim();
        if param.is_empty() {
            continue;
        }

        if let Some((key, value)) = param.split_once('=') {
            let key = key.trim();
            let value = value.trim();

            match key {
                "file" => params.file = Some(value.to_string()),
                "line" => params.line = value.parse().ok(),
                "col" => params.col = value.parse().ok(),
                "endColumn" => params.end_column = value.parse().ok(),
                "endLine" => params.end_line = value.parse().ok(),
                "title" => params.title = Some(value.to_string()),
                _ => {} // Ignore unknown parameters
            }
        }
    }

    params
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_group_start() {
        let result = parse_command("::group::Build artifacts");
        assert!(result.is_some());
        let (cmd, msg) = result.unwrap();
        assert!(matches!(cmd, WorkflowCommand::GroupStart { .. }));
        if let WorkflowCommand::GroupStart { title } = cmd {
            assert_eq!(title, "Build artifacts");
        }
        assert_eq!(msg, "Build artifacts");
    }

    #[test]
    fn test_group_end() {
        let result = parse_command("::endgroup::");
        assert!(result.is_some());
        let (cmd, msg) = result.unwrap();
        assert!(matches!(cmd, WorkflowCommand::GroupEnd));
        assert_eq!(msg, "");
    }

    #[test]
    fn test_error_with_params() {
        let result = parse_command("::error file=app.js,line=10,col=15::Something went wrong");
        assert!(result.is_some());
        let (cmd, msg) = result.unwrap();

        if let WorkflowCommand::Error { message, params } = cmd {
            assert_eq!(message, "Something went wrong");
            assert_eq!(params.file, Some("app.js".to_string()));
            assert_eq!(params.line, Some(10));
            assert_eq!(params.col, Some(15));
        } else {
            panic!("Expected Error command");
        }
        assert_eq!(msg, "Something went wrong");
    }

    #[test]
    fn test_warning_simple() {
        let result = parse_command("::warning::This is a warning");
        assert!(result.is_some());
        let (cmd, msg) = result.unwrap();

        if let WorkflowCommand::Warning { message, .. } = cmd {
            assert_eq!(message, "This is a warning");
        } else {
            panic!("Expected Warning command");
        }
        assert_eq!(msg, "This is a warning");
    }

    #[test]
    fn test_debug() {
        let result = parse_command("::debug::Debug information");
        assert!(result.is_some());
        let (cmd, msg) = result.unwrap();

        if let WorkflowCommand::Debug { message } = cmd {
            assert_eq!(message, "Debug information");
        } else {
            panic!("Expected Debug command");
        }
        assert_eq!(msg, "Debug information");
    }

    #[test]
    fn test_not_a_command() {
        let result = parse_command("This is just regular log output");
        assert!(result.is_none());
    }

    #[test]
    fn test_malformed_command() {
        let result = parse_command("::incomplete");
        assert!(result.is_none());
    }

    #[test]
    fn test_hash_bracket_group_start() {
        let result = parse_command("##[group]Initializing the repository");
        assert!(result.is_some());
        let (cmd, msg) = result.unwrap();
        assert!(matches!(cmd, WorkflowCommand::GroupStart { .. }));
        if let WorkflowCommand::GroupStart { title } = cmd {
            assert_eq!(title, "Initializing the repository");
        }
        assert_eq!(msg, "Initializing the repository");
    }

    #[test]
    fn test_hash_bracket_endgroup() {
        let result = parse_command("##[endgroup]");
        assert!(result.is_some());
        let (cmd, msg) = result.unwrap();
        assert!(matches!(cmd, WorkflowCommand::GroupEnd));
        assert_eq!(msg, "");
    }

    #[test]
    fn test_hash_bracket_error() {
        let result = parse_command("##[error]Something went wrong");
        assert!(result.is_some());
        let (cmd, msg) = result.unwrap();
        if let WorkflowCommand::Error { message, .. } = cmd {
            assert_eq!(message, "Something went wrong");
        } else {
            panic!("Expected Error command");
        }
        assert_eq!(msg, "Something went wrong");
    }

    #[test]
    fn test_command_prefix() {
        let result = parse_command("[command]/opt/homebrew/bin/git init /Users/runner/work/repo");
        assert!(result.is_some());
        let (cmd, msg) = result.unwrap();
        if let WorkflowCommand::Debug { message } = cmd {
            assert_eq!(
                message,
                "/opt/homebrew/bin/git init /Users/runner/work/repo"
            );
        } else {
            panic!("Expected Debug command (for [command] prefix)");
        }
        assert_eq!(msg, "/opt/homebrew/bin/git init /Users/runner/work/repo");
    }
}
