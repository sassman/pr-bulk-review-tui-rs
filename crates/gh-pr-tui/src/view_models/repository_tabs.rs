/// View model for repository tabs - all presentation data pre-computed
#[derive(Debug, Clone)]
pub struct RepositoryTabsViewModel {
    /// Pre-formatted title with instructions and filter
    pub title: String,
    /// Tab items with display text
    pub tabs: Vec<TabItem>,
    /// Index of the selected tab
    pub selected_index: usize,
}

/// A single tab item
#[derive(Debug, Clone)]
pub struct TabItem {
    /// Pre-formatted display text: "⏳ 1 org/repo" or "2 org/repo"
    pub display_text: String,
}

impl RepositoryTabsViewModel {
    /// Build view model from repos state
    pub fn from_state(
        repos: &[crate::Repo],
        repo_data: &std::collections::HashMap<usize, crate::state::RepoData>,
        selected_repo: usize,
        filter_label: &str,
    ) -> Self {
        // Build tab items
        let tabs = repos
            .iter()
            .enumerate()
            .map(|(i, repo)| {
                // Check if this repo is currently loading
                let is_loading = repo_data
                    .get(&i)
                    .map(|data| matches!(data.loading_state, crate::state::LoadingState::Loading))
                    .unwrap_or(false);

                // Format number (only for first 9 repos)
                let number = if i < 9 {
                    format!("{} ", i + 1)
                } else {
                    String::new()
                };

                // Add sandglass prefix if loading
                let prefix = if is_loading { "⏳ " } else { "" };

                // Pre-format the display text
                let display_text = format!("{}{}{}/{}", prefix, number, repo.org, repo.repo);

                TabItem { display_text }
            })
            .collect();

        // Pre-format title
        let title = format!(
            "Projects [Tab/1-9: switch, /: cycle] | Filter: {} [f: cycle]",
            filter_label
        );

        Self {
            title,
            tabs,
            selected_index: selected_repo,
        }
    }
}
