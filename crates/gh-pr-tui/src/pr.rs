use chrono::{DateTime, Utc};
use octocrab::Octocrab;
use ratatui::widgets::Row;

use crate::Repo;

#[derive(Debug, Clone)]
pub struct Pr {
    pub number: usize,
    pub title: String,
    pub body: String,
    pub author: String,
    pub no_comments: usize,
    pub merge_state: String,
    pub mergeable: MergeableStatus, // Checked via background task
    pub needs_rebase: bool,         // True if PR is behind base branch
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MergeableStatus {
    Unknown,         // Not yet checked
    BuildInProgress, // Background check in progress
    Ready,           // ✓ Ready to merge (no issues)
    NeedsRebase,     // ↻ Branch is behind, needs rebase
    BuildFailed,     // ✗ CI/build checks failed
    Conflicted,      // ✗ Has merge conflicts
    Blocked,         // ⊗ Blocked by reviews or other checks
    Rebasing,        // ⟳ Currently rebasing (transient state)
    Merging,         // ⇒ Currently merging (transient state)
}

impl Pr {
    pub async fn from_pull_request(
        pr: &octocrab::models::pulls::PullRequest,
        repo: &Repo,
        octocrab: &Octocrab,
    ) -> Self {
        let (mergeable_state, merge_commit) = if pr.mergeable_state.is_none() {
            let pr_no = pr.number;
            let pr_details = octocrab.pulls(&repo.org, &repo.repo).get(pr_no).await.ok();
            if let Some(pr_details) = pr_details {
                let merge_commit = pr_details.merge_commit_sha;
                (
                    Some(
                        pr_details
                            .mergeable_state
                            .unwrap_or(octocrab::models::pulls::MergeableState::Unknown),
                    ),
                    merge_commit,
                )
            } else {
                (Some(octocrab::models::pulls::MergeableState::Unknown), None)
            }
        } else {
            (Some(octocrab::models::pulls::MergeableState::Unknown), None)
        };

        Self {
            number: pr.number as usize,
            title: pr.title.clone().unwrap_or_default(),
            body: pr.body.clone().unwrap_or_default(),
            author: pr.user.clone().unwrap().login,
            no_comments: pr.comments.unwrap_or_default() as usize,
            merge_state: pr
                .mergeable_state
                .clone()
                .or(mergeable_state)
                .map(|merge_state| match merge_state {
                    octocrab::models::pulls::MergeableState::Behind => "n".to_string(),
                    octocrab::models::pulls::MergeableState::Blocked => "n".to_string(),
                    octocrab::models::pulls::MergeableState::Clean => match merge_commit {
                        Some(merge_commit) => format!("y:{merge_commit}"),
                        None => "y".to_string(),
                    },
                    octocrab::models::pulls::MergeableState::Dirty => "n".to_string(),
                    octocrab::models::pulls::MergeableState::Draft => "n".to_string(),
                    octocrab::models::pulls::MergeableState::HasHooks => "n".to_string(),
                    octocrab::models::pulls::MergeableState::Unknown => "na".to_string(),
                    octocrab::models::pulls::MergeableState::Unstable => "n".to_string(),
                    _ => todo!(),
                })
                .unwrap(),
            mergeable: MergeableStatus::Unknown, // Will be checked in background
            needs_rebase: false,                 // Will be checked in background
            created_at: pr.created_at.unwrap(),
            updated_at: pr.updated_at.unwrap(),
        }
    }
}

impl MergeableStatus {
    pub fn icon(&self) -> &str {
        match self {
            MergeableStatus::Unknown => "?",
            MergeableStatus::BuildInProgress => "⋯",
            MergeableStatus::Ready => "✓",
            MergeableStatus::NeedsRebase => "↻",
            MergeableStatus::BuildFailed => "✗",
            MergeableStatus::Conflicted => "✗",
            MergeableStatus::Blocked => "⊗",
            MergeableStatus::Rebasing => "⟳",
            MergeableStatus::Merging => "⇒",
        }
    }

    pub fn color(&self) -> ratatui::style::Color {
        use ratatui::style::Color;
        match self {
            MergeableStatus::Unknown => Color::DarkGray,
            MergeableStatus::BuildInProgress => Color::Yellow,
            MergeableStatus::Ready => Color::Green,
            MergeableStatus::NeedsRebase => Color::Yellow,
            MergeableStatus::BuildFailed => Color::Red,
            MergeableStatus::Conflicted => Color::Red,
            MergeableStatus::Blocked => Color::Red,
            MergeableStatus::Rebasing => Color::Cyan,
            MergeableStatus::Merging => Color::Cyan,
        }
    }

    pub fn label(&self) -> &str {
        match self {
            MergeableStatus::Unknown => "Unknown",
            MergeableStatus::BuildInProgress => "Building",
            MergeableStatus::Ready => "Ready",
            MergeableStatus::NeedsRebase => "Needs Rebase",
            MergeableStatus::BuildFailed => "Build Failed",
            MergeableStatus::Conflicted => "Conflicted",
            MergeableStatus::Blocked => "Blocked",
            MergeableStatus::Rebasing => "Rebasing...",
            MergeableStatus::Merging => "Merging...",
        }
    }
}

impl From<&Pr> for Row<'static> {
    fn from(val: &Pr) -> Self {
        use ratatui::style::Style;
        use ratatui::widgets::Cell;

        // Show status with icon and label (e.g., "✓ Ready", "✗ Build Failed")
        let status_text = format!("{} {}", val.mergeable.icon(), val.mergeable.label());

        Row::new(vec![
            Cell::from(val.number.to_string()),
            Cell::from(val.title.clone()),
            Cell::from(val.author.clone()),
            Cell::from(val.no_comments.to_string()),
            Cell::from(status_text).style(Style::default().fg(val.mergeable.color())),
        ])
    }
}
