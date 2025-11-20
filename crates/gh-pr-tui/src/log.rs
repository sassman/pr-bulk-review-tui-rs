use std::time::Duration;

/// Job execution status (domain model - no presentation logic)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    Success,
    Failure,
    Cancelled,
    Skipped,
    InProgress,
    Unknown,
}

/// Metadata for a single build job
#[derive(Debug, Clone)]
pub struct JobMetadata {
    pub name: String,          // Full job name (e.g., "lint (macos-latest, clippy)")
    pub workflow_name: String, // Workflow name (e.g., "CI")
    pub status: JobStatus,     // Success/Failure/etc
    pub error_count: usize,    // Number of errors found
    pub duration: Option<Duration>, // Job duration
    pub html_url: String,      // GitHub URL to job
}

#[derive(Debug, Clone)]
pub struct LogPanel {
    /// Tree data from parser (WorkflowNode contains JobNode contains StepNode)
    pub workflows: Vec<gh_actions_log_parser::WorkflowNode>,

    /// Job metadata from GitHub API (indexed by workflow.name + job.name)
    pub job_metadata: std::collections::HashMap<String, JobMetadata>,

    /// UI state: which nodes are expanded
    /// Key: "workflow_idx" or "workflow_idx:job_idx" or "workflow_idx:job_idx:step_idx"
    pub expanded_nodes: std::collections::HashSet<String>,

    /// Current cursor position in tree (path from root)
    /// [workflow_idx, job_idx, step_idx]
    /// Length indicates depth: 1=workflow, 2=job, 3=step
    pub cursor_path: Vec<usize>,

    /// Scroll offset (which tree node is at top of viewport)
    pub scroll_offset: usize,

    /// Horizontal scroll (for long log lines)
    pub horizontal_scroll: usize,

    // UI state
    pub show_timestamps: bool,
    /// Viewport height (updated during rendering)
    pub viewport_height: usize,
    /// PR context for header
    pub pr_context: PrContext,
}

#[derive(Debug, Clone)]
pub struct PrContext {
    pub number: usize,
    pub title: String,
    pub author: String,
}

impl LogPanel {
    /// Navigate down to next visible tree node
    pub fn navigate_down(&mut self) {
        let visible = self.flatten_visible_nodes();
        if visible.is_empty() {
            return;
        }

        // Find current position in flattened list
        if let Some(current_idx) = visible.iter().position(|path| path == &self.cursor_path)
            && current_idx < visible.len() - 1
        {
            let new_idx = current_idx + 1;
            self.cursor_path = visible[new_idx].clone();

            // Auto-scroll to keep cursor visible
            let max_visible_idx = self.scroll_offset + self.viewport_height.saturating_sub(1);
            if new_idx > max_visible_idx {
                self.scroll_offset = new_idx.saturating_sub(self.viewport_height.saturating_sub(1));
            }
        }
    }

    /// Navigate up to previous visible tree node
    pub fn navigate_up(&mut self) {
        let visible = self.flatten_visible_nodes();
        if visible.is_empty() {
            return;
        }

        // Find current position in flattened list
        if let Some(current_idx) = visible.iter().position(|path| path == &self.cursor_path)
            && current_idx > 0
        {
            let new_idx = current_idx - 1;
            self.cursor_path = visible[new_idx].clone();

            // Auto-scroll to keep cursor visible
            if new_idx < self.scroll_offset {
                self.scroll_offset = new_idx;
            }
        }
    }

    /// Toggle expand/collapse at cursor
    pub fn toggle_at_cursor(&mut self) {
        let key = self.path_to_key(&self.cursor_path);

        if self.expanded_nodes.contains(&key) {
            self.expanded_nodes.remove(&key);
        } else {
            self.expanded_nodes.insert(key);
        }
    }

    /// Convert path to string key for expanded_nodes
    fn path_to_key(&self, path: &[usize]) -> String {
        path.iter()
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(":")
    }

    /// Check if a node is expanded
    pub fn is_expanded(&self, path: &[usize]) -> bool {
        self.expanded_nodes.contains(&self.path_to_key(path))
    }

    /// Flatten tree to list of visible node paths
    pub fn flatten_visible_nodes(&self) -> Vec<Vec<usize>> {
        let mut result = Vec::new();

        for (w_idx, workflow) in self.workflows.iter().enumerate() {
            result.push(vec![w_idx]);

            if self.is_expanded(&[w_idx]) {
                for (j_idx, job) in workflow.jobs.iter().enumerate() {
                    result.push(vec![w_idx, j_idx]);

                    if self.is_expanded(&[w_idx, j_idx]) {
                        for (s_idx, step) in job.steps.iter().enumerate() {
                            result.push(vec![w_idx, j_idx, s_idx]);

                            // If step is expanded, add all log lines
                            if self.is_expanded(&[w_idx, j_idx, s_idx]) {
                                for (l_idx, _line) in step.lines.iter().enumerate() {
                                    result.push(vec![w_idx, j_idx, s_idx, l_idx]);
                                }
                            }
                        }
                    }
                }
            }
        }

        result
    }

    /// Find next error across entire tree
    /// If in a step, first navigate through error lines within the step
    pub fn find_next_error(&mut self) {
        // Check if we're in a step (path length 3) or at a log line (path length 4)
        if self.cursor_path.len() >= 3 {
            // Try to find next error line in current step
            let step_path = &self.cursor_path[0..3]; // [workflow, job, step]

            // Get the step to check for error lines
            if let Some(workflow) = self.workflows.get(step_path[0])
                && let Some(job) = workflow.jobs.get(step_path[1])
                && let Some(step) = job.steps.get(step_path[2])
            {
                // Check if step is expanded (has visible lines)
                if self.is_expanded(step_path) {
                    let start_line_idx = if self.cursor_path.len() == 4 {
                        self.cursor_path[3] + 1 // Start after current line
                    } else {
                        0 // Start from first line if at step level
                    };

                    // Find next error line in this step
                    for (line_idx, line) in step.lines.iter().enumerate().skip(start_line_idx) {
                        let is_error = if let Some(ref cmd) = line.command {
                            matches!(cmd, gh_actions_log_parser::WorkflowCommand::Error { .. })
                        } else {
                            line.display_content.to_lowercase().contains("error:")
                        };

                        if is_error {
                            // Found error line in current step
                            let new_path = vec![step_path[0], step_path[1], step_path[2], line_idx];
                            let visible = self.flatten_visible_nodes();
                            if let Some(idx) = visible.iter().position(|path| path == &new_path) {
                                self.cursor_path = new_path;
                                // Auto-scroll to keep cursor visible
                                let max_visible_idx =
                                    self.scroll_offset + self.viewport_height.saturating_sub(1);
                                if idx > max_visible_idx {
                                    self.scroll_offset =
                                        idx.saturating_sub(self.viewport_height.saturating_sub(1));
                                } else if idx < self.scroll_offset {
                                    self.scroll_offset = idx;
                                }
                            }
                            return;
                        }
                    }
                }
            }
        }

        // No more error lines in current step, jump to next step/job with errors
        let error_paths = self.collect_error_paths();
        if error_paths.is_empty() {
            return;
        }

        // Find next error step/job after current cursor
        let visible = self.flatten_visible_nodes();
        if let Some(current_idx) = visible.iter().position(|path| path == &self.cursor_path) {
            // Look for next error path after current position
            for (idx, path) in visible.iter().enumerate().skip(current_idx + 1) {
                if error_paths.contains(path) {
                    self.cursor_path = path.clone();
                    // Auto-scroll to keep cursor visible
                    let max_visible_idx =
                        self.scroll_offset + self.viewport_height.saturating_sub(1);
                    if idx > max_visible_idx {
                        self.scroll_offset =
                            idx.saturating_sub(self.viewport_height.saturating_sub(1));
                    } else if idx < self.scroll_offset {
                        self.scroll_offset = idx;
                    }
                    return;
                }
            }
        }

        // Wrap to first error
        if let Some(first_error) = error_paths.first()
            && let Some(idx) = visible.iter().position(|path| path == first_error)
        {
            self.cursor_path = first_error.clone();
            // Auto-scroll to top when wrapping
            self.scroll_offset = idx;
        }
    }

    /// Find previous error across entire tree
    /// If in a step, first navigate through error lines within the step (backwards)
    pub fn find_prev_error(&mut self) {
        // Check if we're in a step (path length 3) or at a log line (path length 4)
        if self.cursor_path.len() >= 3 {
            // Try to find previous error line in current step
            let step_path = &self.cursor_path[0..3]; // [workflow, job, step]

            // Get the step to check for error lines
            if let Some(workflow) = self.workflows.get(step_path[0])
                && let Some(job) = workflow.jobs.get(step_path[1])
                && let Some(step) = job.steps.get(step_path[2])
            {
                // Check if step is expanded (has visible lines)
                if self.is_expanded(step_path) {
                    let end_line_idx = if self.cursor_path.len() == 4 {
                        self.cursor_path[3] // Current line (exclusive)
                    } else {
                        step.lines.len() // All lines if at step level
                    };

                    // Find previous error line in this step (iterate backwards)
                    for (line_idx, line) in step.lines.iter().enumerate().take(end_line_idx).rev() {
                        let is_error = if let Some(ref cmd) = line.command {
                            matches!(cmd, gh_actions_log_parser::WorkflowCommand::Error { .. })
                        } else {
                            line.display_content.to_lowercase().contains("error:")
                        };

                        if is_error {
                            // Found error line in current step
                            let new_path = vec![step_path[0], step_path[1], step_path[2], line_idx];
                            let visible = self.flatten_visible_nodes();
                            if let Some(idx) = visible.iter().position(|path| path == &new_path) {
                                self.cursor_path = new_path;
                                // Auto-scroll to keep cursor visible
                                let max_visible_idx =
                                    self.scroll_offset + self.viewport_height.saturating_sub(1);
                                if idx > max_visible_idx {
                                    self.scroll_offset =
                                        idx.saturating_sub(self.viewport_height.saturating_sub(1));
                                } else if idx < self.scroll_offset {
                                    self.scroll_offset = idx;
                                }
                            }
                            return;
                        }
                    }
                }
            }
        }

        // No more error lines in current step, jump to previous step/job with errors
        let error_paths = self.collect_error_paths();
        if error_paths.is_empty() {
            return;
        }

        // Find previous error before current cursor
        let visible = self.flatten_visible_nodes();
        if let Some(current_idx) = visible.iter().position(|path| path == &self.cursor_path) {
            // Look for previous error path before current position (iterate backwards)
            for (idx, path) in visible.iter().enumerate().take(current_idx).rev() {
                if error_paths.contains(path) {
                    self.cursor_path = path.clone();
                    // Auto-scroll to keep cursor visible
                    let max_visible_idx =
                        self.scroll_offset + self.viewport_height.saturating_sub(1);
                    if idx > max_visible_idx {
                        self.scroll_offset =
                            idx.saturating_sub(self.viewport_height.saturating_sub(1));
                    } else if idx < self.scroll_offset {
                        self.scroll_offset = idx;
                    }
                    return;
                }
            }
        }

        // Wrap to last error
        if let Some(last_error) = error_paths.last()
            && let Some(idx) = visible.iter().position(|path| path == last_error)
        {
            self.cursor_path = last_error.clone();
            // Auto-scroll when wrapping
            let max_visible_idx = self.scroll_offset + self.viewport_height.saturating_sub(1);
            if idx > max_visible_idx {
                self.scroll_offset = idx.saturating_sub(self.viewport_height.saturating_sub(1));
            } else if idx < self.scroll_offset {
                self.scroll_offset = idx;
            }
        }
    }

    /// Collect all tree paths that have errors
    fn collect_error_paths(&self) -> Vec<Vec<usize>> {
        let mut result = Vec::new();

        for (w_idx, workflow) in self.workflows.iter().enumerate() {
            for (j_idx, job) in workflow.jobs.iter().enumerate() {
                if job.error_count > 0 {
                    result.push(vec![w_idx, j_idx]);
                }

                for (s_idx, step) in job.steps.iter().enumerate() {
                    if step.error_count > 0 {
                        result.push(vec![w_idx, j_idx, s_idx]);
                    }
                }
            }
        }

        result
    }
}

/// Create LogPanel from parsed job logs (tree view)
/// Builds a hierarchical tree: Workflow → Job → Step
pub fn create_log_panel_from_jobs(
    jobs: Vec<(JobMetadata, gh_actions_log_parser::JobLog)>,
    pr_context: PrContext,
) -> LogPanel {
    use std::collections::HashMap;

    // Group jobs by workflow name and convert to tree using parser
    let mut workflows_map: HashMap<String, Vec<(JobMetadata, gh_actions_log_parser::JobNode)>> =
        HashMap::new();
    let mut job_metadata_map: HashMap<String, JobMetadata> = HashMap::new();

    for (metadata, job_log) in jobs {
        let job_node = gh_actions_log_parser::job_log_to_tree(job_log);

        // Filter out jobs with no logs AND "/system" in name
        let has_logs =
            !job_node.steps.is_empty() && job_node.steps.iter().any(|step| !step.lines.is_empty());
        let has_system = job_node.name.contains("/system");

        // Skip this job if it has no logs AND has /system in name
        if !has_logs && has_system {
            continue;
        }

        let key = format!("{}:{}", metadata.workflow_name, job_node.name);
        job_metadata_map.insert(key, metadata.clone());

        workflows_map
            .entry(metadata.workflow_name.clone())
            .or_default()
            .push((metadata, job_node));
    }

    // Build workflow nodes (jobs already filtered above)
    let mut workflows: Vec<gh_actions_log_parser::WorkflowNode> = workflows_map
        .into_iter()
        .map(|(workflow_name, jobs)| {
            let mut job_nodes: Vec<gh_actions_log_parser::JobNode> =
                jobs.into_iter().map(|(_, job)| job).collect();

            // Sort jobs alphabetically by name
            job_nodes.sort_by(|a, b| a.name.cmp(&b.name));

            let total_errors: usize = job_nodes.iter().map(|j| j.error_count).sum();
            let has_failures = total_errors > 0;

            gh_actions_log_parser::WorkflowNode {
                name: workflow_name,
                jobs: job_nodes,
                total_errors,
                has_failures,
            }
        })
        .collect();

    // Sort workflows: failed first, then by name
    workflows.sort_by(|a, b| {
        b.has_failures
            .cmp(&a.has_failures)
            .then(a.name.cmp(&b.name))
    });

    // Auto-expand workflows (top level) and nodes with errors
    let mut expanded_nodes = std::collections::HashSet::new();
    for (w_idx, workflow) in workflows.iter().enumerate() {
        // Always expand workflows (top level)
        expanded_nodes.insert(w_idx.to_string());

        // Auto-expand jobs and steps with errors
        for (j_idx, job) in workflow.jobs.iter().enumerate() {
            if job.error_count > 0 {
                expanded_nodes.insert(format!("{}:{}", w_idx, j_idx));

                for (s_idx, step) in job.steps.iter().enumerate() {
                    if step.error_count > 0 {
                        expanded_nodes.insert(format!("{}:{}:{}", w_idx, j_idx, s_idx));
                    }
                }
            }
        }
    }

    LogPanel {
        workflows,
        job_metadata: job_metadata_map,
        expanded_nodes,
        cursor_path: vec![0], // Start at first workflow
        scroll_offset: 0,
        horizontal_scroll: 0,
        show_timestamps: false,
        viewport_height: 20,
        pr_context,
    }
}
