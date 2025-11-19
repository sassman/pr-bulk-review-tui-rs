//! GitHub Actions Log Parser
//!
//! A library for parsing GitHub Actions workflow logs with ANSI color preservation
//! and GitHub Actions workflow command support (::group::, ::error::, ::warning::).
//!
//! # Example
//!
//! ```no_run
//! use gh_actions_log_parser::parse_workflow_logs;
//!
//! let zip_data: &[u8] = &[]; // ZIP file bytes from GitHub API
//! let parsed = parse_workflow_logs(zip_data)?;
//!
//! for job in &parsed.jobs {
//!     println!("Job: {}", job.name);
//!     for line in &job.lines {
//!         // Access styled segments, commands, group info, etc.
//!     }
//! }
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod ansi;
mod commands;
mod parser;
mod types;

pub use parser::{job_log_to_tree, parse_workflow_logs};
pub use types::*;

#[cfg(test)]
mod tests {

    #[test]
    fn test_basic_parsing() {
        // Add tests here
    }
}
