use chrono::{DateTime, Utc};
use octocrab::Octocrab;
use ratatui::widgets::Row;

use crate::Repo;

pub struct Pr {
    pub number: usize,
    pub title: String,
    pub body: String,
    pub author: String,
    pub no_comments: usize,
    pub merge_state: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
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
            created_at: pr.created_at.unwrap(),
            updated_at: pr.updated_at.unwrap(),
        }
    }
}

impl<'a> Into<Row<'a>> for &Pr {
    fn into(self) -> Row<'a> {
        Row::new(vec![
            self.number.to_string(),
            self.title.clone(),
            self.author.clone(),
            self.no_comments.to_string(),
            self.created_at.to_rfc3339(),
            self.updated_at.to_rfc3339(),
        ])
    }
}
