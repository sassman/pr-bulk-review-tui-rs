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
    pub mergeable: MergeableStatus,  // Checked via background task
    pub needs_rebase: bool,          // True if PR is behind base branch
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MergeableStatus {
    Unknown,      // Not yet checked
    Checking,     // Background check in progress
    Mergeable,    // ✓ Can be merged
    Conflicted,   // ✗ Has conflicts
    Blocked,      // ✗ Blocked by checks/reviews
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
            mergeable: MergeableStatus::Unknown,  // Will be checked in background
            needs_rebase: false,                  // Will be checked in background
            created_at: pr.created_at.unwrap(),
            updated_at: pr.updated_at.unwrap(),
        }
    }
}

impl MergeableStatus {
    pub fn icon(&self) -> &str {
        match self {
            MergeableStatus::Unknown => "?",
            MergeableStatus::Checking => "⋯",
            MergeableStatus::Mergeable => "✓",
            MergeableStatus::Conflicted => "✗",
            MergeableStatus::Blocked => "⊗",
        }
    }

    pub fn color(&self) -> ratatui::style::Color {
        use ratatui::style::Color;
        match self {
            MergeableStatus::Unknown => Color::DarkGray,
            MergeableStatus::Checking => Color::Yellow,
            MergeableStatus::Mergeable => Color::Green,
            MergeableStatus::Conflicted => Color::Red,
            MergeableStatus::Blocked => Color::Red,
        }
    }
}

impl Into<Row<'static>> for &Pr {
    fn into(self) -> Row<'static> {
        use ratatui::widgets::Cell;
        use ratatui::style::Style;
        use ratatui::style::Color;

        let rebase_icon = if self.needs_rebase { "↻" } else { "" };
        let rebase_color = if self.needs_rebase { Color::Yellow } else { Color::DarkGray };

        Row::new(vec![
            Cell::from(self.number.to_string()),
            Cell::from(self.title.clone()),
            Cell::from(self.author.clone()),
            Cell::from(self.no_comments.to_string()),
            Cell::from(self.mergeable.icon().to_string())
                .style(Style::default().fg(self.mergeable.color())),
            Cell::from(rebase_icon.to_string())
                .style(Style::default().fg(rebase_color)),
        ])
    }
}
