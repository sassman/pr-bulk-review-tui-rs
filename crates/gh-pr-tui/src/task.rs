/// Background task system for handling heavy operations without blocking UI
use crate::{
    PrFilter,
    gh::{comment, merge},
    log::PrContext,
    pr::{MergeableStatus, Pr},
    state::{Repo, TaskStatus},
};
use log::{debug, error};
use octocrab::Octocrab;
use std::path::PathBuf;
use tokio::sync::mpsc;

/// Results from background task execution
/// These are sent back to the main loop and converted to Actions
#[derive(Debug)]
pub enum TaskResult {
    /// Repository loading started (repo_index) - sent before fetch begins
    RepoLoadingStarted(usize),

    /// Repository data loaded (repo_index, result)
    RepoDataLoaded(usize, Result<Vec<Pr>, String>),

    /// Merge status updated for a PR
    MergeStatusUpdated(usize, usize, MergeableStatus), // repo_index, pr_number, status

    /// Rebase status updated for a PR
    RebaseStatusUpdated(usize, usize, bool), // repo_index, pr_number, needs_rebase

    /// Comment count updated for a PR
    CommentCountUpdated(usize, usize, usize), // repo_index, pr_number, comment_count

    /// Rebase operation completed
    RebaseComplete(Result<(), String>),

    /// Merge operation completed
    MergeComplete(Result<(), String>),

    /// Rerun failed jobs operation completed
    RerunJobsComplete(Result<(), String>),

    /// PR approval operation completed
    ApprovalComplete(Result<(), String>),

    /// Close PR operation completed
    ClosePrComplete(Result<(), String>),

    /// Build logs loaded - Vec of (metadata, logs) pairs
    BuildLogsLoaded(
        Vec<(crate::log::JobMetadata, gh_actions_log_parser::JobLog)>,
        crate::log::PrContext,
    ),

    /// IDE open operation completed
    IDEOpenComplete(Result<(), String>),

    /// PR merge status confirmed (for merge bot polling)
    PRMergedConfirmed(usize, usize, bool), // repo_index, pr_number, is_merged

    /// Task status update
    TaskStatusUpdate(Option<TaskStatus>),

    /// Auto-merge status check needed
    AutoMergeStatusCheck(usize, usize), // repo_index, pr_number

    /// Remove PR from auto-merge queue
    RemoveFromAutoMergeQueue(usize, usize), // repo_index, pr_number

    /// Operation monitor check needed (rebase/merge progress)
    OperationMonitorCheck(usize, usize), // repo_index, pr_number

    /// Remove PR from operation monitor queue
    RemoveFromOperationMonitor(usize, usize), // repo_index, pr_number

    /// Repo needs reload (e.g., after PR merged)
    RepoNeedsReload(usize), // repo_index
}

/// Background tasks that can be executed asynchronously
#[derive(Debug)]
pub enum BackgroundTask {
    LoadAllRepos {
        repos: Vec<(usize, Repo)>, // (repo_index, repo) pairs
        filter: PrFilter,
        octocrab: Octocrab,
    },
    LoadSingleRepo {
        repo_index: usize,
        repo: Repo,
        filter: PrFilter,
        octocrab: Octocrab,
    },
    CheckMergeStatus {
        repo_index: usize,
        repo: Repo,
        pr_numbers: Vec<usize>,
        octocrab: Octocrab,
    },
    CheckCommentCounts {
        repo_index: usize,
        repo: Repo,
        pr_numbers: Vec<usize>,
        octocrab: Octocrab,
    },
    Rebase {
        repo: Repo,
        prs: Vec<Pr>,
        selected_indices: Vec<usize>,
        octocrab: Octocrab,
    },
    Merge {
        repo: Repo,
        prs: Vec<Pr>,
        selected_indices: Vec<usize>,
        octocrab: Octocrab,
    },
    RerunFailedJobs {
        repo: Repo,
        pr_numbers: Vec<usize>,
        octocrab: Octocrab,
    },
    ApprovePrs {
        repo: Repo,
        pr_numbers: Vec<usize>,
        approval_message: String,
        octocrab: Octocrab,
    },
    ClosePrs {
        repo: Repo,
        pr_numbers: Vec<usize>,
        prs: Vec<Pr>, // Need full PR objects to check author
        comment: String,
        octocrab: Octocrab,
    },
    FetchBuildLogs {
        repo: Repo,
        pr_number: usize,
        head_sha: String,
        octocrab: Octocrab,
        pr_context: PrContext,
    },
    OpenPRInIDE {
        repo: Repo,
        pr_number: usize,
        ide_command: String,
        temp_dir: String,
    },
    /// Poll a PR to check if it's actually merged (for merge bot)
    PollPRMergeStatus {
        repo_index: usize,
        repo: Repo,
        pr_number: usize,
        octocrab: Octocrab,
        is_checking_ci: bool, // If true, use longer sleep (15s) for CI checks
    },
    /// Enable auto-merge on GitHub and monitor PR until ready
    EnableAutoMerge {
        repo_index: usize,
        repo: Repo,
        pr_number: usize,
        octocrab: Octocrab,
    },
    MonitorOperation {
        repo_index: usize,
        repo: Repo,
        pr_number: usize,
        operation: crate::state::OperationType,
        octocrab: Octocrab,
    },
    /// Generic delayed task wrapper - delays execution of any task
    DelayedTask {
        task: Box<BackgroundTask>,
        delay_ms: u64,
    },
}

/// Background task worker that processes heavy operations without blocking UI
pub fn start_task_worker(
    mut task_rx: mpsc::UnboundedReceiver<BackgroundTask>,
    mut result_tx: mpsc::UnboundedSender<TaskResult>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(task) = task_rx.recv().await {
            process_task(task, &mut result_tx).await;
        }
    })
}

async fn process_task(task: BackgroundTask, result_tx: &mut mpsc::UnboundedSender<TaskResult>) {
    match task {
        BackgroundTask::LoadAllRepos {
            repos,
            filter,
            octocrab,
        } => {
            // Spawn parallel tasks for each repo
            let mut tasks = Vec::new();
            for (repo_index, repo) in repos.iter() {
                // Signal that we're starting to load this repo (shows half progress)
                let _ = result_tx.send(TaskResult::RepoLoadingStarted(*repo_index));

                let octocrab = octocrab.clone();
                let repo = repo.clone();
                let filter = filter.clone();
                let index = *repo_index;

                let task = tokio::spawn(async move {
                    let result = crate::fetch_github_data(&octocrab, &repo, &filter)
                        .await
                        .map_err(|e| e.to_string());
                    (index, result)
                });
                tasks.push(task);

                // Small delay between starting each request to show incremental progress
                tokio::time::sleep(tokio::time::Duration::from_millis(30)).await;
            }

            // Collect results and send back to main loop
            // Add small delays between results to allow UI to update incrementally
            for task in tasks {
                if let Ok((index, result)) = task.await {
                    // Log success or error for each repo
                    match &result {
                        Ok(prs) => {
                            debug!("Loaded repo #{} successfully: {} PRs", index, prs.len());
                        }
                        Err(err) => {
                            debug!("Failed to load repo #{}: {}", index, err);
                        }
                    }
                    let _ = result_tx.send(TaskResult::RepoDataLoaded(index, result));
                    // Small delay to allow UI to redraw and show progress
                    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
                }
            }
        }
        BackgroundTask::LoadSingleRepo {
            repo_index,
            repo,
            filter,
            octocrab,
        } => {
            debug!(
                "Loading repo {}/{} (index: {})...",
                repo.org, repo.repo, repo_index
            );
            let result = crate::fetch_github_data(&octocrab, &repo, &filter)
                .await
                .map_err(|e| e.to_string());

            // Log success or error
            match &result {
                Ok(prs) => {
                    debug!(
                        "Successfully loaded {}/{}: {} PRs",
                        repo.org,
                        repo.repo,
                        prs.len()
                    );
                }
                Err(err) => {
                    error!("Failed to load {}/{}: {}", repo.org, repo.repo, err);
                }
            }

            let _ = result_tx.send(TaskResult::RepoDataLoaded(repo_index, result));
        }
        BackgroundTask::CheckMergeStatus {
            repo_index,
            repo,
            pr_numbers,
            octocrab,
        } => {
            // Check merge status for each PR in parallel
            let mut tasks = Vec::new();
            for pr_number in pr_numbers {
                let octocrab = octocrab.clone();
                let repo = repo.clone();
                let result_tx = result_tx.clone();

                let task = tokio::spawn(async move {
                    use crate::pr::MergeableStatus;

                    // Fetch detailed PR info to get mergeable status and rebase status
                    match octocrab
                        .pulls(&repo.org, &repo.repo)
                        .get(pr_number as u64)
                        .await
                    {
                        Ok(pr_detail) => {
                            // Check if PR needs rebase (Behind state means PR is behind base branch)
                            let needs_rebase = if let Some(ref state) = pr_detail.mergeable_state {
                                matches!(
                                    state,
                                    octocrab::models::pulls::MergeableState::Behind
                                        | octocrab::models::pulls::MergeableState::Blocked
                                        | octocrab::models::pulls::MergeableState::Unknown
                                        | octocrab::models::pulls::MergeableState::Unstable
                                        | octocrab::models::pulls::MergeableState::Dirty
                                )
                            } else {
                                false
                            };

                            // Check CI/build status by fetching check runs
                            let head_sha = pr_detail.head.sha.clone();

                            // Use the REST API directly to get check runs
                            let check_runs_url = format!(
                                "/repos/{}/{}/commits/{}/check-runs",
                                repo.org, repo.repo, head_sha
                            );

                            #[derive(Debug, serde::Deserialize)]
                            struct CheckRunsResponse {
                                check_runs: Vec<CheckRun>,
                            }

                            #[derive(Debug, serde::Deserialize)]
                            struct CheckRun {
                                status: String,
                                conclusion: Option<String>,
                            }

                            let (ci_failed, ci_in_progress) = match octocrab
                                .get::<CheckRunsResponse, _, ()>(&check_runs_url, None::<&()>)
                                .await
                            {
                                Ok(response) => {
                                    // Check if any check run failed
                                    let failed = response.check_runs.iter().any(|check| {
                                        check.status == "completed"
                                            && (check.conclusion.as_deref() == Some("failure")
                                                || check.conclusion.as_deref() == Some("cancelled")
                                                || check.conclusion.as_deref() == Some("timed_out"))
                                    });
                                    // Check if any check run is still in progress
                                    let in_progress = response.check_runs.iter().any(|check| {
                                        check.status == "queued" || check.status == "in_progress"
                                    });
                                    (failed, in_progress)
                                }
                                Err(_) => {
                                    // Fallback: use mergeable_state "unstable" as indicator
                                    let failed = if let Some(ref state) = pr_detail.mergeable_state
                                    {
                                        matches!(
                                            state,
                                            octocrab::models::pulls::MergeableState::Unstable
                                        )
                                    } else {
                                        false
                                    };
                                    (failed, false)
                                }
                            };

                            // Determine final status with priority:
                            // 1. Conflicted (mergeable=false && dirty)
                            // 2. BuildFailed (CI checks failed)
                            // 3. Checking (CI checks in progress)
                            // 4. NeedsRebase (branch is behind)
                            // 5. Blocked (other blocking reasons)
                            // 6. Ready (all good!)
                            let status = match pr_detail.mergeable {
                                Some(false) => {
                                    // Not mergeable - check why
                                    if let Some(ref state) = pr_detail.mergeable_state {
                                        match state {
                                            octocrab::models::pulls::MergeableState::Dirty => {
                                                MergeableStatus::Conflicted
                                            }
                                            octocrab::models::pulls::MergeableState::Blocked => {
                                                if ci_failed {
                                                    MergeableStatus::BuildFailed
                                                } else if ci_in_progress {
                                                    MergeableStatus::BuildInProgress
                                                } else {
                                                    MergeableStatus::Blocked
                                                }
                                            }
                                            _ => MergeableStatus::Blocked,
                                        }
                                    } else {
                                        MergeableStatus::Conflicted
                                    }
                                }
                                Some(true) => {
                                    // Mergeable, but check for other issues
                                    if ci_failed {
                                        MergeableStatus::BuildFailed
                                    } else if ci_in_progress {
                                        MergeableStatus::BuildInProgress
                                    } else if needs_rebase {
                                        MergeableStatus::NeedsRebase
                                    } else {
                                        MergeableStatus::Ready
                                    }
                                }
                                None => {
                                    // mergeable status unknown - check if CI is running
                                    if ci_in_progress {
                                        MergeableStatus::BuildInProgress
                                    } else {
                                        MergeableStatus::Unknown
                                    }
                                }
                            };

                            let _ = result_tx.send(TaskResult::MergeStatusUpdated(
                                repo_index, pr_number, status,
                            ));
                            let _ = result_tx.send(TaskResult::RebaseStatusUpdated(
                                repo_index,
                                pr_number,
                                needs_rebase,
                            ));
                        }
                        Err(_) => {
                            // Failed to fetch, keep as unknown
                        }
                    }
                });
                tasks.push(task);
            }

            // Wait for all checks to complete
            for task in tasks {
                let _ = task.await;
            }
        }
        BackgroundTask::CheckCommentCounts {
            repo_index,
            repo,
            pr_numbers,
            octocrab,
        } => {
            // Check comment counts for each PR in parallel
            let mut tasks = Vec::new();
            for pr_number in pr_numbers {
                let octocrab = octocrab.clone();
                let repo = repo.clone();
                let result_tx = result_tx.clone();

                let task = tokio::spawn(async move {
                    // Fetch detailed PR info to get accurate comment count
                    match octocrab
                        .pulls(&repo.org, &repo.repo)
                        .get(pr_number as u64)
                        .await
                    {
                        Ok(pr_detail) => {
                            // Get total comment count (includes review comments + issue comments)
                            let comment_count = pr_detail.comments.unwrap_or(0) as usize;

                            let _ = result_tx.send(TaskResult::CommentCountUpdated(
                                repo_index,
                                pr_number,
                                comment_count,
                            ));
                        }
                        Err(_) => {
                            // Failed to fetch, keep existing count
                        }
                    }
                });
                tasks.push(task);
            }

            // Wait for all checks to complete
            for task in tasks {
                let _ = task.await;
            }
        }
        BackgroundTask::Rebase {
            repo,
            prs,
            selected_indices,
            octocrab,
        } => {
            use crate::pr::MergeableStatus;

            let mut success = true;
            for &idx in &selected_indices {
                if let Some(pr) = prs.get(idx) {
                    // For dependabot PRs, use comment-based rebase
                    if pr.author.starts_with("dependabot") {
                        // If PR has conflicts, use "@dependabot recreate" to rebuild the PR
                        // Otherwise use "@dependabot rebase" for normal rebase
                        let comment_text = if pr.mergeable == MergeableStatus::Conflicted {
                            "@dependabot recreate"
                        } else {
                            "@dependabot rebase"
                        };

                        debug!(
                            "Posting comment '{}' to dependabot PR #{}",
                            comment_text, pr.number
                        );
                        match comment(&octocrab, &repo, pr, comment_text).await {
                            Ok(_) => {
                                debug!(
                                    "Successfully posted comment to dependabot PR #{}",
                                    pr.number
                                );
                            }
                            Err(e) => {
                                debug!(
                                    "Failed to comment on dependabot PR #{}: {:?}",
                                    pr.number, e
                                );
                                success = false;
                            }
                        }
                    } else {
                        // For regular PRs, use GitHub's update_branch API
                        // This performs a rebase/merge to bring the PR branch up to date with base
                        debug!(
                            "Attempting to update branch for PR #{} in {}/{}",
                            pr.number, repo.org, repo.repo
                        );
                        let update_result = octocrab
                            .pulls(&repo.org, &repo.repo)
                            .update_branch(pr.number as u64)
                            .await;

                        match update_result {
                            Ok(_) => {
                                debug!(
                                    "Successfully triggered update_branch for PR #{}",
                                    pr.number
                                );
                            }
                            Err(e) => {
                                debug!("Failed to update_branch for PR #{}: {:?}", pr.number, e);
                                success = false;
                            }
                        }
                    }
                }
            }
            let result = if success {
                Ok(())
            } else {
                Err("Some rebases failed".to_string())
            };
            let _ = result_tx.send(TaskResult::RebaseComplete(result));
        }
        BackgroundTask::Merge {
            repo,
            prs,
            selected_indices,
            octocrab,
        } => {
            let mut success = true;
            for &idx in &selected_indices {
                if let Some(pr) = prs.get(idx)
                    && let Err(_) = merge(&octocrab, &repo, pr).await
                {
                    success = false;
                }
            }
            let result = if success {
                Ok(())
            } else {
                Err("Some merges failed".to_string())
            };
            let _ = result_tx.send(TaskResult::MergeComplete(result));
        }
        BackgroundTask::RerunFailedJobs {
            repo,
            pr_numbers,
            octocrab,
        } => {
            let mut all_success = true;
            let mut rerun_count = 0;

            for pr_number in pr_numbers {
                // Get PR details to find head SHA
                let pr = match octocrab
                    .pulls(&repo.org, &repo.repo)
                    .get(pr_number as u64)
                    .await
                {
                    Ok(pr) => pr,
                    Err(_) => {
                        all_success = false;
                        continue;
                    }
                };

                let head_sha = &pr.head.sha;

                // Get workflow runs for this PR using REST API
                let url = format!(
                    "/repos/{}/{}/actions/runs?head_sha={}",
                    repo.org, repo.repo, head_sha
                );

                #[derive(Debug, serde::Deserialize)]
                struct WorkflowRunsResponse {
                    workflow_runs: Vec<octocrab::models::workflows::Run>,
                }

                let workflow_response: WorkflowRunsResponse =
                    match octocrab.get(&url, None::<&()>).await {
                        Ok(response) => response,
                        Err(_) => {
                            all_success = false;
                            continue;
                        }
                    };

                let runs = workflow_response.workflow_runs;

                // Find failed runs and rerun them
                for run in runs {
                    let is_failed = run.conclusion.as_deref() == Some("failure");
                    if is_failed {
                        // Rerun failed jobs for this run
                        let url = format!(
                            "https://api.github.com/repos/{}/{}/actions/runs/{}/rerun-failed-jobs",
                            repo.org, repo.repo, run.id
                        );

                        // Use serde_json::Value as response type for POST requests
                        match octocrab
                            .post::<(), serde_json::Value>(&url, None::<&()>)
                            .await
                        {
                            Ok(_) => {
                                rerun_count += 1;
                            }
                            Err(_) => {
                                all_success = false;
                            }
                        }
                    }
                }
            }

            let result = if all_success && rerun_count > 0 {
                Ok(())
            } else if rerun_count == 0 {
                Err("No failed jobs found to rerun".to_string())
            } else {
                Err("Some jobs failed to rerun".to_string())
            };
            let _ = result_tx.send(TaskResult::RerunJobsComplete(result));
        }
        BackgroundTask::ApprovePrs {
            repo,
            pr_numbers,
            approval_message,
            octocrab,
        } => {
            // Approve PRs using GitHub's review API
            let mut all_success = true;
            let mut approval_count = 0;

            for pr_number in &pr_numbers {
                // Create a review with APPROVE event using the REST API directly
                #[derive(serde::Serialize)]
                struct ReviewBody {
                    body: String,
                    event: String,
                }

                let review_body = ReviewBody {
                    body: approval_message.clone(),
                    event: "APPROVE".to_string(),
                };

                let url = format!(
                    "/repos/{}/{}/pulls/{}/reviews",
                    repo.org, repo.repo, pr_number
                );
                let result: Result<serde_json::Value, _> =
                    octocrab.post(&url, Some(&review_body)).await;

                match result {
                    Ok(_) => {
                        approval_count += 1;
                        debug!("Successfully approved PR #{}", pr_number);
                    }
                    Err(e) => {
                        all_success = false;
                        debug!("Failed to approve PR #{}: {}", pr_number, e);
                    }
                }
            }

            let result = if all_success && approval_count > 0 {
                Ok(())
            } else if approval_count == 0 {
                Err("Failed to approve any PRs".to_string())
            } else {
                Err(format!(
                    "Approved {}/{} PRs",
                    approval_count,
                    pr_numbers.len()
                ))
            };
            let _ = result_tx.send(TaskResult::ApprovalComplete(result));
        }
        BackgroundTask::ClosePrs {
            repo,
            pr_numbers,
            prs,
            comment,
            octocrab,
        } => {
            // Close PRs with comment (use @dependabot close for dependabot PRs)
            let mut all_success = true;
            let mut close_count = 0;

            for pr_number in &pr_numbers {
                // Find the full PR object to check author
                let pr = prs.iter().find(|p| p.number == *pr_number);
                let is_dependabot = pr
                    .map(|p| p.author.to_lowercase().contains("dependabot"))
                    .unwrap_or(false);

                let actual_comment = if is_dependabot {
                    "@dependabot close".to_string()
                } else {
                    comment.clone()
                };

                // First, add a comment using octocrab issues API
                if let Err(e) = octocrab
                    .issues(&repo.org, &repo.repo)
                    .create_comment(*pr_number as _, &actual_comment)
                    .await
                {
                    debug!("Failed to add comment to PR #{}: {}", pr_number, e);
                    all_success = false;
                    continue;
                }

                // For dependabot PRs, just the comment is enough
                if is_dependabot {
                    close_count += 1;
                    debug!("Added '@dependabot close' comment to PR #{}", pr_number);
                } else {
                    // For regular PRs, close the PR via API
                    #[derive(serde::Serialize)]
                    struct UpdatePrBody {
                        state: String,
                    }

                    let update_body = UpdatePrBody {
                        state: "closed".to_string(),
                    };

                    let url = format!("/repos/{}/{}/pulls/{}", repo.org, repo.repo, pr_number);
                    let result: Result<serde_json::Value, _> =
                        octocrab.patch(&url, Some(&update_body)).await;

                    match result {
                        Ok(_) => {
                            close_count += 1;
                            debug!("Successfully closed PR #{}", pr_number);
                        }
                        Err(e) => {
                            all_success = false;
                            debug!("Failed to close PR #{}: {}", pr_number, e);
                        }
                    }
                }
            }

            let result = if all_success && close_count > 0 {
                Ok(())
            } else if close_count == 0 {
                Err("Failed to close any PRs".to_string())
            } else {
                Err(format!("Closed {}/{} PRs", close_count, pr_numbers.len()))
            };
            let _ = result_tx.send(TaskResult::ClosePrComplete(result));
        }
        BackgroundTask::FetchBuildLogs {
            repo,
            pr_number,
            head_sha: _,
            octocrab,
            pr_context,
        } => {
            // First, get the PR details to get the actual head SHA
            let pr_details = match octocrab
                .pulls(&repo.org, &repo.repo)
                .get(pr_number as u64)
                .await
            {
                Ok(pr) => pr,
                Err(_) => {
                    let _ = result_tx.send(TaskResult::BuildLogsLoaded(vec![], pr_context));
                    return;
                }
            };

            let head_sha = pr_details.head.sha.clone();

            // Get workflow runs for this commit using the REST API directly
            let url = format!(
                "/repos/{}/{}/actions/runs?head_sha={}",
                repo.org, repo.repo, head_sha
            );

            #[derive(Debug, serde::Deserialize)]
            struct WorkflowRunsResponse {
                workflow_runs: Vec<octocrab::models::workflows::Run>,
            }

            let workflow_runs: WorkflowRunsResponse = match octocrab.get(&url, None::<&()>).await {
                Ok(runs) => runs,
                Err(_) => {
                    let _ = result_tx.send(TaskResult::BuildLogsLoaded(vec![], pr_context));
                    return;
                }
            };

            let mut log_sections = Vec::new();

            // Process each workflow run and download its logs
            for workflow_run in workflow_runs.workflow_runs {
                let conclusion_str = workflow_run.conclusion.as_deref().unwrap_or("in_progress");
                let workflow_name = workflow_run.name.clone();

                // Skip successful runs unless there are no failures
                let is_failed = matches!(
                    conclusion_str,
                    "failure" | "timed_out" | "action_required" | "cancelled"
                );

                if !is_failed && !log_sections.is_empty() {
                    continue;
                }

                let mut metadata_lines = Vec::new();
                metadata_lines.push(format!("Workflow: {}", workflow_name));
                metadata_lines.push(format!("Run ID: {}", workflow_run.id));
                metadata_lines.push(format!("Run URL: {}", workflow_run.html_url));
                metadata_lines.push(format!("Conclusion: {}", conclusion_str));
                metadata_lines.push(format!("Started: {}", workflow_run.created_at));
                metadata_lines.push(format!("Updated: {}", workflow_run.updated_at));
                metadata_lines.push("".to_string());

                // Fetch jobs for this workflow run to get job IDs and URLs
                let jobs_url = format!(
                    "/repos/{}/{}/actions/runs/{}/jobs",
                    repo.org, repo.repo, workflow_run.id
                );

                #[derive(Debug, serde::Deserialize)]
                struct JobsResponse {
                    jobs: Vec<WorkflowJob>,
                }

                #[derive(Debug, serde::Deserialize)]
                struct WorkflowJob {
                    #[allow(dead_code)]
                    id: u64,
                    name: String,
                    html_url: String,
                    conclusion: Option<String>,
                    started_at: String,
                    completed_at: Option<String>,
                }

                let jobs_response: Result<JobsResponse, _> =
                    octocrab.get(&jobs_url, None::<&()>).await;

                // Try to download the workflow run logs (they come as a zip file)
                match octocrab
                    .actions()
                    .download_workflow_run_logs(&repo.org, &repo.repo, workflow_run.id)
                    .await
                {
                    Ok(log_data) => {
                        // The log_data is a zip file as bytes
                        // Parse using the gh-actions-log-parser crate
                        match gh_actions_log_parser::parse_workflow_logs(&log_data) {
                            Ok(parsed_log) => {
                                // Process each job's logs and build metadata
                                for job_log in parsed_log.jobs {
                                    // Try to find matching GitHub API job by name
                                    let github_job = if let Ok(ref jobs) = jobs_response {
                                        jobs.jobs.iter().find(|j| job_log.name.contains(&j.name))
                                    } else {
                                        None
                                    };

                                    // Extract real job name from log content (look for "Complete job name:" line)
                                    let mut display_name = job_log.name.clone();
                                    for line in &job_log.lines {
                                        if line.content.contains("Complete job name:") {
                                            // Extract: "2025-11-15T19:56:48.3220210Z Complete job name: lint (macos-latest, clippy)"
                                            if let Some(name_part) =
                                                line.content.split("Complete job name:").nth(1)
                                            {
                                                display_name = name_part.trim().to_string();
                                                break;
                                            }
                                        }
                                    }

                                    // Count errors in this job
                                    let error_count = job_log.lines.iter().filter(|line| {
                                        // Count lines with error workflow command OR containing "error:"
                                        if let Some(ref cmd) = line.command {
                                            matches!(cmd, gh_actions_log_parser::WorkflowCommand::Error { .. })
                                        } else {
                                            line.content.to_lowercase().contains("error:")
                                        }
                                    }).count();

                                    // Parse job status from GitHub API
                                    let status = if let Some(job) = github_job {
                                        match job.conclusion.as_deref() {
                                            Some("success") => crate::log::JobStatus::Success,
                                            Some("failure") => crate::log::JobStatus::Failure,
                                            Some("cancelled") => crate::log::JobStatus::Cancelled,
                                            Some("skipped") => crate::log::JobStatus::Skipped,
                                            None => crate::log::JobStatus::InProgress,
                                            _ => crate::log::JobStatus::Unknown,
                                        }
                                    } else {
                                        // Infer from error count if no API data
                                        if error_count > 0 {
                                            crate::log::JobStatus::Failure
                                        } else {
                                            crate::log::JobStatus::Success
                                        }
                                    };

                                    // Calculate duration from GitHub API
                                    let duration = if let Some(job) = github_job {
                                        if let Some(ref completed) = job.completed_at {
                                            // Parse timestamps and calculate duration
                                            use chrono::DateTime;
                                            if let (Ok(started), Ok(completed)) = (
                                                DateTime::parse_from_rfc3339(&job.started_at),
                                                DateTime::parse_from_rfc3339(completed),
                                            ) {
                                                let duration =
                                                    completed.signed_duration_since(started);
                                                Some(std::time::Duration::from_secs(
                                                    duration.num_seconds().max(0) as u64,
                                                ))
                                            } else {
                                                None
                                            }
                                        } else {
                                            None
                                        }
                                    } else {
                                        None
                                    };

                                    // Build job metadata
                                    let metadata = crate::log::JobMetadata {
                                        name: display_name,
                                        workflow_name: workflow_name.clone(),
                                        status,
                                        error_count,
                                        duration,
                                        html_url: github_job
                                            .map(|j| j.html_url.clone())
                                            .unwrap_or_default(),
                                    };

                                    // Add to jobs list (preserve full JobLog from parser)
                                    log_sections.push((metadata, job_log));
                                }
                            }
                            Err(_err) => {
                                // Parser error - skip this workflow run
                                // User will see error in the PR list or can open in browser
                            }
                        }
                    }
                    Err(_) => {
                        // Download error - skip this workflow run
                        // Common if logs expired or auth issues
                    }
                }
            }

            // Sort jobs: failed first, then successful
            // UI will display them in order with failed at top
            log_sections.sort_by_key(|(metadata, _)| match metadata.status {
                crate::log::JobStatus::Failure => 0,
                crate::log::JobStatus::Cancelled => 1,
                crate::log::JobStatus::InProgress => 2,
                crate::log::JobStatus::Unknown => 3,
                crate::log::JobStatus::Skipped => 4,
                crate::log::JobStatus::Success => 5,
            });

            let _ = result_tx.send(TaskResult::BuildLogsLoaded(log_sections, pr_context));
        }
        BackgroundTask::OpenPRInIDE {
            repo,
            pr_number,
            ide_command,
            temp_dir,
        } => {
            use std::process::Command;

            // Create temp directory if it doesn't exist
            if let Err(err) = std::fs::create_dir_all(&temp_dir) {
                let _ = result_tx.send(TaskResult::IDEOpenComplete(Err(format!(
                    "Failed to create temp directory: {}",
                    err
                ))));
                return;
            }

            // Create unique directory for this PR or main branch
            let dir_name = if pr_number == 0 {
                format!("{}-{}-main", repo.org, repo.repo)
            } else {
                format!("{}-{}-pr-{}", repo.org, repo.repo, pr_number)
            };
            let pr_dir = PathBuf::from(&temp_dir).join(dir_name);

            // Remove existing directory if present
            if pr_dir.exists()
                && let Err(err) = std::fs::remove_dir_all(&pr_dir)
            {
                let _ = result_tx.send(TaskResult::IDEOpenComplete(Err(format!(
                    "Failed to remove existing directory: {}",
                    err
                ))));
                return;
            }

            // Clone the repository using gh repo clone (uses SSH by default)
            let clone_output = Command::new("gh")
                .args([
                    "repo",
                    "clone",
                    &format!("{}/{}", repo.org, repo.repo),
                    &pr_dir.to_string_lossy(),
                ])
                .output();

            if let Err(err) = clone_output {
                let _ = result_tx.send(TaskResult::IDEOpenComplete(Err(format!(
                    "Failed to run gh repo clone: {}",
                    err
                ))));
                return;
            }

            let clone_output = clone_output.unwrap();
            if !clone_output.status.success() {
                let stderr = String::from_utf8_lossy(&clone_output.stderr);
                let _ = result_tx.send(TaskResult::IDEOpenComplete(Err(format!(
                    "gh repo clone failed: {}",
                    stderr
                ))));
                return;
            }

            // Checkout PR branch or main branch
            if pr_number == 0 {
                // Checkout main branch and pull latest
                let checkout_output = Command::new("git")
                    .args(["checkout", "main"])
                    .current_dir(&pr_dir)
                    .output();

                if let Err(err) = checkout_output {
                    let _ = result_tx.send(TaskResult::IDEOpenComplete(Err(format!(
                        "Failed to run git checkout main: {}",
                        err
                    ))));
                    return;
                }

                let checkout_output = checkout_output.unwrap();
                if !checkout_output.status.success() {
                    let stderr = String::from_utf8_lossy(&checkout_output.stderr);
                    let _ = result_tx.send(TaskResult::IDEOpenComplete(Err(format!(
                        "git checkout main failed: {}",
                        stderr
                    ))));
                    return;
                }

                // Pull latest changes
                let pull_output = Command::new("git")
                    .args(["pull"])
                    .current_dir(&pr_dir)
                    .output();

                if let Err(err) = pull_output {
                    let _ = result_tx.send(TaskResult::IDEOpenComplete(Err(format!(
                        "Failed to run git pull: {}",
                        err
                    ))));
                    return;
                }

                let pull_output = pull_output.unwrap();
                if !pull_output.status.success() {
                    let stderr = String::from_utf8_lossy(&pull_output.stderr);
                    let _ = result_tx.send(TaskResult::IDEOpenComplete(Err(format!(
                        "git pull failed: {}",
                        stderr
                    ))));
                    return;
                }
            } else {
                // Checkout the PR using gh pr checkout
                let checkout_output = Command::new("gh")
                    .args(["pr", "checkout", &pr_number.to_string()])
                    .current_dir(&pr_dir)
                    .output();

                if let Err(err) = checkout_output {
                    let _ = result_tx.send(TaskResult::IDEOpenComplete(Err(format!(
                        "Failed to run gh pr checkout: {}",
                        err
                    ))));
                    return;
                }

                let checkout_output = checkout_output.unwrap();
                if !checkout_output.status.success() {
                    let stderr = String::from_utf8_lossy(&checkout_output.stderr);
                    let _ = result_tx.send(TaskResult::IDEOpenComplete(Err(format!(
                        "gh pr checkout failed: {}",
                        stderr
                    ))));
                    return;
                }
            }

            // Set origin URL to SSH (gh checkout doesn't do this)
            let ssh_url = format!("git@github.com:{}/{}.git", repo.org, repo.repo);
            let set_url_output = Command::new("git")
                .args(["remote", "set-url", "origin", &ssh_url])
                .current_dir(&pr_dir)
                .output();

            if let Err(err) = set_url_output {
                let _ = result_tx.send(TaskResult::IDEOpenComplete(Err(format!(
                    "Failed to set SSH origin URL: {}",
                    err
                ))));
                return;
            }

            let set_url_output = set_url_output.unwrap();
            if !set_url_output.status.success() {
                let stderr = String::from_utf8_lossy(&set_url_output.stderr);
                let _ = result_tx.send(TaskResult::IDEOpenComplete(Err(format!(
                    "Failed to set SSH origin URL: {}",
                    stderr
                ))));
                return;
            }

            // Open in IDE
            let ide_output = Command::new(&ide_command).arg(&pr_dir).spawn();

            match ide_output {
                Ok(_) => {
                    let _ = result_tx.send(TaskResult::IDEOpenComplete(Ok(())));
                }
                Err(err) => {
                    let _ = result_tx.send(TaskResult::IDEOpenComplete(Err(format!(
                        "Failed to open IDE '{}': {}",
                        ide_command, err
                    ))));
                }
            }
        }
        BackgroundTask::PollPRMergeStatus {
            repo_index,
            repo,
            pr_number,
            octocrab,
            is_checking_ci,
        } => {
            // Poll the PR to check status
            // Wait before polling to give GitHub time to process
            // Use longer sleep (15s) when checking CI, shorter (2s) for merge confirmation
            let sleep_duration = if is_checking_ci {
                tokio::time::Duration::from_secs(15) // CI can take 4-10 minutes
            } else {
                tokio::time::Duration::from_secs(2) // Merge is usually quick
            };
            tokio::time::sleep(sleep_duration).await;

            match octocrab
                .pulls(&repo.org, &repo.repo)
                .get(pr_number as u64)
                .await
            {
                Ok(pr_detail) => {
                    if is_checking_ci {
                        // When checking CI, use GitHub's mergeable field which considers branch protection
                        // This properly handles PRs with failed non-required checks
                        use crate::pr::MergeableStatus;

                        // Check if PR needs rebase
                        let needs_rebase = if let Some(ref state) = pr_detail.mergeable_state {
                            matches!(state, octocrab::models::pulls::MergeableState::Behind)
                        } else {
                            false
                        };

                        // Check CI/build status
                        let head_sha = pr_detail.head.sha.clone();
                        let check_runs_url = format!(
                            "/repos/{}/{}/commits/{}/check-runs",
                            repo.org, repo.repo, head_sha
                        );

                        #[derive(Debug, serde::Deserialize)]
                        struct CheckRunsResponse {
                            check_runs: Vec<CheckRun>,
                        }

                        #[derive(Debug, serde::Deserialize)]
                        struct CheckRun {
                            status: String,
                            conclusion: Option<String>,
                        }

                        let (ci_failed, ci_in_progress) = match octocrab
                            .get::<CheckRunsResponse, _, ()>(&check_runs_url, None::<&()>)
                            .await
                        {
                            Ok(response) => {
                                let failed = response.check_runs.iter().any(|check| {
                                    check.status == "completed"
                                        && matches!(
                                            check.conclusion.as_deref(),
                                            Some("failure") | Some("cancelled") | Some("timed_out")
                                        )
                                });
                                let in_progress = response.check_runs.iter().any(|check| {
                                    check.status == "queued" || check.status == "in_progress"
                                });
                                (failed, in_progress)
                            }
                            Err(_) => {
                                // Fallback: use mergeable_state as indicator
                                let failed = if let Some(ref state) = pr_detail.mergeable_state {
                                    matches!(
                                        state,
                                        octocrab::models::pulls::MergeableState::Unstable
                                    )
                                } else {
                                    false
                                };
                                (failed, false)
                            }
                        };

                        // Determine status using GitHub's mergeable field
                        // This properly handles required vs optional check failures
                        let status = match pr_detail.mergeable {
                            Some(false) => {
                                // Not mergeable - check why
                                if let Some(ref state) = pr_detail.mergeable_state {
                                    match state {
                                        octocrab::models::pulls::MergeableState::Dirty => {
                                            MergeableStatus::Conflicted
                                        }
                                        octocrab::models::pulls::MergeableState::Blocked => {
                                            if ci_failed {
                                                MergeableStatus::BuildFailed
                                            } else if ci_in_progress {
                                                MergeableStatus::BuildInProgress
                                            } else {
                                                MergeableStatus::Blocked
                                            }
                                        }
                                        _ => MergeableStatus::Blocked,
                                    }
                                } else {
                                    MergeableStatus::Conflicted
                                }
                            }
                            Some(true) => {
                                // PR is mergeable according to GitHub (required checks passed)
                                // Even if some non-required checks failed, we can merge
                                if ci_in_progress {
                                    MergeableStatus::BuildInProgress
                                } else if needs_rebase {
                                    MergeableStatus::NeedsRebase
                                } else {
                                    MergeableStatus::Ready
                                }
                            }
                            None => {
                                // mergeable status unknown - check if CI is running
                                if ci_in_progress {
                                    MergeableStatus::BuildInProgress
                                } else {
                                    MergeableStatus::Unknown
                                }
                            }
                        };

                        let _ = result_tx.send(TaskResult::MergeStatusUpdated(
                            repo_index, pr_number, status,
                        ));
                    } else {
                        // When checking merge confirmation, just check if PR is merged
                        let is_merged = pr_detail.merged_at.is_some();
                        let _ = result_tx.send(TaskResult::PRMergedConfirmed(
                            repo_index, pr_number, is_merged,
                        ));
                    }
                }
                Err(_) => {
                    if is_checking_ci {
                        // Can't fetch PR, send unknown status
                        let _ = result_tx.send(TaskResult::MergeStatusUpdated(
                            repo_index,
                            pr_number,
                            crate::pr::MergeableStatus::Unknown,
                        ));
                    } else {
                        // Can't fetch PR, assume not merged yet
                        let _ = result_tx
                            .send(TaskResult::PRMergedConfirmed(repo_index, pr_number, false));
                    }
                }
            }
        }
        BackgroundTask::EnableAutoMerge {
            repo_index,
            repo,
            pr_number,
            octocrab,
        } => {
            // Enable auto-merge on GitHub using GraphQL API
            let result = enable_github_auto_merge(&octocrab, &repo, pr_number).await;

            match result {
                Ok(_) => {
                    // Success - schedule periodic status checks
                    let _ = result_tx.send(TaskResult::TaskStatusUpdate(Some(
                        crate::state::TaskStatus {
                            message: format!(
                                "Auto-merge enabled for PR #{}, monitoring...",
                                pr_number
                            ),
                            status_type: crate::state::TaskStatusType::Success,
                        },
                    )));

                    // Spawn a task to periodically check PR status
                    let result_tx_clone = result_tx.clone();
                    let repo_clone = repo.clone();
                    let octocrab_clone = octocrab.clone();
                    tokio::spawn(async move {
                        for _ in 0..20 {
                            // Wait 1 minute between checks
                            tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;

                            // Send status check result
                            let _ = result_tx_clone
                                .send(TaskResult::AutoMergeStatusCheck(repo_index, pr_number));

                            // Check merge status to update PR state
                            if let Ok(pr_detail) = octocrab_clone
                                .pulls(&repo_clone.org, &repo_clone.repo)
                                .get(pr_number as u64)
                                .await
                            {
                                use crate::pr::MergeableStatus;

                                // Determine mergeable status
                                let mergeable_status = if pr_detail.merged_at.is_some() {
                                    // PR has been merged - stop monitoring
                                    let _ = result_tx_clone.send(
                                        TaskResult::RemoveFromAutoMergeQueue(repo_index, pr_number),
                                    );
                                    let _ = result_tx_clone.send(TaskResult::TaskStatusUpdate(
                                        Some(crate::state::TaskStatus {
                                            message: format!(
                                                "PR #{} successfully merged!",
                                                pr_number
                                            ),
                                            status_type: crate::state::TaskStatusType::Success,
                                        }),
                                    ));
                                    break;
                                } else {
                                    // Check CI status
                                    match get_pr_ci_status(
                                        &octocrab_clone,
                                        &repo_clone,
                                        &pr_detail.head.sha,
                                    )
                                    .await
                                    {
                                        Ok((_, build_status)) => match build_status.as_str() {
                                            "success" | "neutral" | "skipped" => {
                                                MergeableStatus::Ready
                                            }
                                            "failure" | "cancelled" | "timed_out"
                                            | "action_required" => MergeableStatus::BuildFailed,
                                            _ => MergeableStatus::BuildInProgress,
                                        },
                                        Err(_) => MergeableStatus::Unknown,
                                    }
                                };

                                // Update PR status
                                let _ = result_tx_clone.send(TaskResult::MergeStatusUpdated(
                                    repo_index,
                                    pr_number,
                                    mergeable_status,
                                ));
                            }
                        }
                    });
                }
                Err(e) => {
                    // Failed to enable auto-merge
                    let _ =
                        result_tx.send(TaskResult::RemoveFromAutoMergeQueue(repo_index, pr_number));
                    let _ = result_tx.send(TaskResult::TaskStatusUpdate(Some(
                        crate::state::TaskStatus {
                            message: format!(
                                "Failed to enable auto-merge for PR #{}: {}",
                                pr_number, e
                            ),
                            status_type: crate::state::TaskStatusType::Error,
                        },
                    )));
                }
            }
        }
        BackgroundTask::MonitorOperation {
            repo_index,
            repo,
            pr_number,
            operation,
            octocrab,
        } => {
            // Spawn a task to periodically monitor the operation
            let result_tx_clone = result_tx.clone();
            let repo_clone = repo.clone();
            let octocrab_clone = octocrab.clone();

            tokio::spawn(async move {
                use crate::pr::MergeableStatus;
                use crate::state::OperationType;

                debug!(
                    "Starting operation monitor for PR #{} ({:?})",
                    pr_number, operation
                );

                // Get initial PR state to track SHA for rebase detection
                let mut last_head_sha = None;
                if let Ok(pr_detail) = octocrab_clone
                    .pulls(&repo_clone.org, &repo_clone.repo)
                    .get(pr_number as u64)
                    .await
                {
                    last_head_sha = Some(pr_detail.head.sha.clone());
                    debug!("Initial SHA for PR #{}: {}", pr_number, pr_detail.head.sha);
                }

                // Track consecutive failures to avoid infinite loops
                let mut consecutive_failures = 0;
                const MAX_CONSECUTIVE_FAILURES: u32 = 5;

                // Monitor for up to 40 checks (20 minutes at 30s intervals)
                for check_num in 0..40 {
                    // Wait between checks (30 seconds)
                    tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;

                    debug!(
                        "Operation monitor check #{} for PR #{}",
                        check_num + 1,
                        pr_number
                    );

                    // Send periodic check action
                    let _ = result_tx_clone
                        .send(TaskResult::OperationMonitorCheck(repo_index, pr_number));

                    // Fetch current PR state
                    let pr_detail = match octocrab_clone
                        .pulls(&repo_clone.org, &repo_clone.repo)
                        .get(pr_number as u64)
                        .await
                    {
                        Ok(pr) => {
                            consecutive_failures = 0; // Reset on success
                            pr
                        }
                        Err(e) => {
                            consecutive_failures += 1;
                            debug!(
                                "Failed to fetch PR #{} (attempt {}/{}): {}",
                                pr_number, consecutive_failures, MAX_CONSECUTIVE_FAILURES, e
                            );

                            if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                                debug!(
                                    "Too many consecutive failures for PR #{}, stopping monitor",
                                    pr_number
                                );
                                let _ = result_tx_clone.send(
                                    TaskResult::RemoveFromOperationMonitor(repo_index, pr_number),
                                );
                                let _ = result_tx_clone.send(TaskResult::TaskStatusUpdate(Some(
                                    crate::state::TaskStatus {
                                        message: format!(
                                            "Monitoring stopped for PR #{} due to API errors",
                                            pr_number
                                        ),
                                        status_type: crate::state::TaskStatusType::Error,
                                    },
                                )));
                                break;
                            }
                            continue; // Skip this check if API fails
                        }
                    };

                    match operation {
                        OperationType::Rebase => {
                            // Check if head SHA changed (rebase completed)
                            let current_sha = pr_detail.head.sha.clone();
                            let sha_changed = if let Some(ref prev_sha) = last_head_sha {
                                if &current_sha != prev_sha {
                                    debug!(
                                        "PR #{} SHA changed: {} -> {}",
                                        pr_number, prev_sha, current_sha
                                    );
                                    true
                                } else {
                                    false
                                }
                            } else {
                                debug!("PR #{} first check, SHA: {}", pr_number, current_sha);
                                false
                            };

                            // Update last SHA
                            last_head_sha = Some(current_sha.clone());

                            // Check CI status (always check after initial rebasing time)
                            if sha_changed || check_num > 2 {
                                debug!(
                                    "Checking CI status for PR #{} at SHA {}",
                                    pr_number, current_sha
                                );

                                match get_pr_ci_status(&octocrab_clone, &repo_clone, &current_sha)
                                    .await
                                {
                                    Ok((_, build_status)) => {
                                        debug!("PR #{} CI status: {}", pr_number, build_status);

                                        let new_status = match build_status.as_str() {
                                            "success" | "neutral" | "skipped" => {
                                                MergeableStatus::Ready
                                            }
                                            "failure" | "cancelled" | "timed_out"
                                            | "action_required" => MergeableStatus::BuildFailed,
                                            "pending" | "in_progress" | "queued" => {
                                                MergeableStatus::BuildInProgress
                                            }
                                            "unknown" => {
                                                // No CI configured - treat as ready after rebase completes
                                                if sha_changed {
                                                    debug!(
                                                        "No CI found for PR #{}, treating as ready",
                                                        pr_number
                                                    );
                                                    MergeableStatus::Ready
                                                } else {
                                                    MergeableStatus::Rebasing
                                                }
                                            }
                                            _ => {
                                                debug!(
                                                    "Unknown CI status '{}' for PR #{}, treating as in progress",
                                                    build_status, pr_number
                                                );
                                                MergeableStatus::BuildInProgress
                                            }
                                        };

                                        // Update status
                                        let _ =
                                            result_tx_clone.send(TaskResult::MergeStatusUpdated(
                                                repo_index, pr_number, new_status,
                                            ));

                                        // If CI is done (or no CI), stop monitoring
                                        if matches!(
                                            new_status,
                                            MergeableStatus::Ready | MergeableStatus::BuildFailed
                                        ) {
                                            debug!(
                                                "PR #{} monitoring complete with status {:?}",
                                                pr_number, new_status
                                            );
                                            let _ = result_tx_clone.send(
                                                TaskResult::RemoveFromOperationMonitor(
                                                    repo_index, pr_number,
                                                ),
                                            );
                                            break;
                                        }
                                    }
                                    Err(e) => {
                                        consecutive_failures += 1;
                                        debug!(
                                            "Failed to get CI status for PR #{} (attempt {}/{}): {}",
                                            pr_number,
                                            consecutive_failures,
                                            MAX_CONSECUTIVE_FAILURES,
                                            e
                                        );

                                        if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                                            debug!(
                                                "Too many CI status failures for PR #{}, stopping monitor",
                                                pr_number
                                            );
                                            let _ = result_tx_clone.send(
                                                TaskResult::RemoveFromOperationMonitor(
                                                    repo_index, pr_number,
                                                ),
                                            );
                                            let _ = result_tx_clone.send(
                                                TaskResult::MergeStatusUpdated(
                                                    repo_index,
                                                    pr_number,
                                                    MergeableStatus::Unknown,
                                                ),
                                            );
                                            break;
                                        }

                                        // Set to building while we retry
                                        let _ =
                                            result_tx_clone.send(TaskResult::MergeStatusUpdated(
                                                repo_index,
                                                pr_number,
                                                MergeableStatus::BuildInProgress,
                                            ));
                                    }
                                }
                            }
                        }
                        OperationType::Merge => {
                            // Check if PR is merged
                            if pr_detail.merged_at.is_some() {
                                // Merge successful!
                                debug!("PR #{} successfully merged!", pr_number);
                                let _ = result_tx_clone.send(
                                    TaskResult::RemoveFromOperationMonitor(repo_index, pr_number),
                                );
                                let _ = result_tx_clone.send(TaskResult::TaskStatusUpdate(Some(
                                    crate::state::TaskStatus {
                                        message: format!("PR #{} successfully merged!", pr_number),
                                        status_type: crate::state::TaskStatusType::Success,
                                    },
                                )));
                                // Trigger repo reload to remove merged PR from list
                                let _ =
                                    result_tx_clone.send(TaskResult::RepoNeedsReload(repo_index));
                                break;
                            } else if matches!(
                                pr_detail.state,
                                Some(octocrab::models::IssueState::Closed)
                            ) {
                                // PR was closed without merging
                                debug!("PR #{} was closed without merging", pr_number);
                                let _ = result_tx_clone.send(
                                    TaskResult::RemoveFromOperationMonitor(repo_index, pr_number),
                                );
                                let _ = result_tx_clone.send(TaskResult::TaskStatusUpdate(Some(
                                    crate::state::TaskStatus {
                                        message: format!(
                                            "PR #{} was closed without merging",
                                            pr_number
                                        ),
                                        status_type: crate::state::TaskStatusType::Error,
                                    },
                                )));
                                break;
                            }

                            // Update status to show we're still merging
                            debug!("PR #{} still merging (check #{})", pr_number, check_num + 1);
                            let _ = result_tx_clone.send(TaskResult::MergeStatusUpdated(
                                repo_index,
                                pr_number,
                                MergeableStatus::Merging,
                            ));
                        }
                    }
                }

                // If we exit the loop without completing, it's a timeout
                debug!(
                    "Operation monitor timed out for PR #{} after 20 minutes",
                    pr_number
                );
                let _ = result_tx_clone.send(TaskResult::RemoveFromOperationMonitor(
                    repo_index, pr_number,
                ));
                let _ = result_tx_clone.send(TaskResult::TaskStatusUpdate(Some(
                    crate::state::TaskStatus {
                        message: format!(
                            "Monitoring timed out for PR #{} after 20 minutes",
                            pr_number
                        ),
                        status_type: crate::state::TaskStatusType::Warning,
                    },
                )));
            });
        }
        BackgroundTask::DelayedTask { task, delay_ms } => {
            // Sleep for the specified delay, then execute the wrapped task
            tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
            debug!("Delayed task triggered after {}ms", delay_ms);
            Box::pin(process_task(*task, result_tx)).await;
        }
    }
}

/// Enable auto-merge on GitHub using GraphQL API
async fn enable_github_auto_merge(
    octocrab: &Octocrab,
    repo: &Repo,
    pr_number: usize,
) -> anyhow::Result<()> {
    // First, get the PR's node_id (needed for GraphQL)
    let pr = octocrab
        .pulls(&repo.org, &repo.repo)
        .get(pr_number as u64)
        .await?;

    let node_id = pr
        .node_id
        .ok_or_else(|| anyhow::anyhow!("PR does not have a node_id"))?;

    // GraphQL mutation to enable auto-merge
    let query = format!(
        r#"mutation {{
            enablePullRequestAutoMerge(input: {{
                pullRequestId: "{}",
                mergeMethod: SQUASH
            }}) {{
                pullRequest {{
                    autoMergeRequest {{
                        enabledAt
                    }}
                }}
            }}
        }}"#,
        node_id
    );

    // Execute GraphQL query
    let response: serde_json::Value = octocrab.graphql(&query).await?;

    // Check for errors in response
    if let Some(errors) = response.get("errors") {
        return Err(anyhow::anyhow!("GraphQL error: {}", errors));
    }

    Ok(())
}

/// Get PR CI status by checking commit status
async fn get_pr_ci_status(
    octocrab: &Octocrab,
    repo: &Repo,
    head_sha: &str,
) -> anyhow::Result<(String, String)> {
    // Check commit status via check-runs API
    let check_runs_url = format!(
        "/repos/{}/{}/commits/{}/check-runs",
        repo.org, repo.repo, head_sha
    );

    let response: serde_json::Value = octocrab.get(&check_runs_url, None::<&()>).await?;

    let empty_vec = vec![];
    let check_runs = response["check_runs"].as_array().unwrap_or(&empty_vec);

    // Determine overall status
    let mut has_failure = false;
    let mut has_pending = false;
    let mut has_success = false;

    for check in check_runs {
        if let Some(conclusion) = check["conclusion"].as_str() {
            match conclusion {
                "success" | "neutral" | "skipped" => has_success = true,
                "failure" | "cancelled" | "timed_out" | "action_required" => has_failure = true,
                _ => has_pending = true,
            }
        } else if let Some(status) = check["status"].as_str()
            && (status == "in_progress" || status == "queued")
        {
            has_pending = true;
        }
    }

    let overall_status = if has_failure {
        ("completed".to_string(), "failure".to_string())
    } else if has_pending {
        ("in_progress".to_string(), "pending".to_string())
    } else if has_success {
        ("completed".to_string(), "success".to_string())
    } else {
        ("unknown".to_string(), "unknown".to_string())
    };

    Ok(overall_status)
}
