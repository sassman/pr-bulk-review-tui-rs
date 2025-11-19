use crate::pr::{MergeableStatus, Pr};

/// Merge bot state machine for automated PR merging with rebase
/// Uses action dispatch system - doesn't perform operations directly
#[derive(Debug, Clone, PartialEq)]
pub enum MergeBotState {
    Idle,
    ProcessingQueue {
        queue: Vec<PrInQueue>,
        current_index: usize,
    },
    WaitingForOperation {
        queue: Vec<PrInQueue>,
        current_index: usize,
        operation: Operation,
    },
    Completed {
        merged: Vec<usize>,
        failed: Vec<(usize, String)>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Operation {
    Merge,
    Rebase,
    CheckCI,
    WaitForMergeConfirmation, // Waiting for API confirmation that PR is actually merged
}

/// PR in the merge bot queue
#[derive(Debug, Clone, PartialEq)]
pub struct PrInQueue {
    pub pr_number: usize,
    pub pr_index: usize, // Index in the prs vec
    pub status: PrQueueStatus,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PrQueueStatus {
    Pending,
    Merging,
    Rebasing,
    WaitingCI,
    Merged,
    Failed(String),
}

/// Merge bot orchestrator
#[derive(Debug, Clone)]
pub struct MergeBot {
    pub state: MergeBotState,
}

impl Default for MergeBot {
    fn default() -> Self {
        Self::new()
    }
}

impl MergeBot {
    pub fn new() -> Self {
        Self {
            state: MergeBotState::Idle,
        }
    }

    /// Start the merge bot with selected PR indices
    pub fn start(&mut self, pr_indices: Vec<(usize, usize)>) {
        // pr_indices is (pr_number, index_in_vec)
        let queue = pr_indices
            .into_iter()
            .map(|(pr_number, pr_index)| PrInQueue {
                pr_number,
                pr_index,
                status: PrQueueStatus::Pending,
            })
            .collect();

        self.state = MergeBotState::ProcessingQueue {
            queue,
            current_index: 0,
        };
    }

    /// Stop the merge bot
    pub fn stop(&mut self) {
        self.state = MergeBotState::Idle;
    }

    /// Check if merge bot is running
    pub fn is_running(&self) -> bool {
        !matches!(
            self.state,
            MergeBotState::Idle | MergeBotState::Completed { .. }
        )
    }

    /// Get current status message for UI
    pub fn status_message(&self) -> String {
        match &self.state {
            MergeBotState::Idle => "Merge bot idle".to_string(),
            MergeBotState::ProcessingQueue {
                queue,
                current_index,
            }
            | MergeBotState::WaitingForOperation {
                queue,
                current_index,
                ..
            } => {
                let completed = queue
                    .iter()
                    .filter(|p| matches!(p.status, PrQueueStatus::Merged))
                    .count();
                let failed = queue
                    .iter()
                    .filter(|p| matches!(p.status, PrQueueStatus::Failed(_)))
                    .count();
                format!(
                    "Merge bot: {}/{} merged, {} failed, processing PR #{}",
                    completed,
                    queue.len(),
                    failed,
                    queue
                        .get(*current_index)
                        .map(|p| p.pr_number)
                        .unwrap_or(0)
                )
            }
            MergeBotState::Completed { merged, failed } => {
                format!(
                    "Merge bot completed: {} merged, {} failed",
                    merged.len(),
                    failed.len()
                )
            }
        }
    }

    /// Process next PR in queue - returns action to dispatch
    pub fn process_next(&mut self, prs: &[Pr]) -> Option<MergeBotAction> {
        // Take ownership of state temporarily to avoid borrow checker issues
        let state = std::mem::replace(&mut self.state, MergeBotState::Idle);

        match state {
            MergeBotState::ProcessingQueue {
                queue,
                current_index,
            } => {
                if current_index >= queue.len() {
                    // All done
                    let merged: Vec<usize> = queue
                        .iter()
                        .filter_map(|p| {
                            if matches!(p.status, PrQueueStatus::Merged) {
                                Some(p.pr_number)
                            } else {
                                None
                            }
                        })
                        .collect();

                    let failed: Vec<(usize, String)> = queue
                        .iter()
                        .filter_map(|p| {
                            if let PrQueueStatus::Failed(reason) = &p.status {
                                Some((p.pr_number, reason.clone()))
                            } else {
                                None
                            }
                        })
                        .collect();

                    self.state = MergeBotState::Completed { merged, failed };
                    return Some(MergeBotAction::Completed);
                }

                // Extract data we need before any borrows
                let pr_number = queue[current_index].pr_number;
                let pr_index = queue[current_index].pr_index;

                let pr = match prs.get(pr_index) {
                    Some(pr) => pr,
                    None => {
                        // Restore state and return None
                        self.state = MergeBotState::ProcessingQueue {
                            queue,
                            current_index,
                        };
                        return None;
                    }
                };

                match pr.mergeable {
                    MergeableStatus::Ready => {
                        // Dispatch merge
                        self.state = MergeBotState::WaitingForOperation {
                            queue,
                            current_index,
                            operation: Operation::Merge,
                        };
                        Some(MergeBotAction::DispatchMerge(vec![pr_index]))
                    }
                    MergeableStatus::NeedsRebase | MergeableStatus::Conflicted => {
                        // Dispatch rebase for both cases:
                        // - NeedsRebase: PR is behind base branch
                        // - Conflicted: PR has conflicts (dependabot will use @dependabot recreate)
                        self.state = MergeBotState::WaitingForOperation {
                            queue,
                            current_index,
                            operation: Operation::Rebase,
                        };
                        Some(MergeBotAction::DispatchRebase(vec![pr_index]))
                    }
                    MergeableStatus::BuildInProgress => {
                        // Wait for CI
                        self.state = MergeBotState::WaitingForOperation {
                            queue,
                            current_index,
                            operation: Operation::CheckCI,
                        };
                        Some(MergeBotAction::WaitForCI(pr_number))
                    }
                    _ => {
                        // Skip this PR
                        let mut new_queue = queue;
                        new_queue[current_index].status =
                            PrQueueStatus::Failed(format!("Not mergeable: {:?}", pr.mergeable));
                        self.state = MergeBotState::ProcessingQueue {
                            queue: new_queue,
                            current_index: current_index + 1,
                        };
                        Some(MergeBotAction::PrSkipped(
                            pr_number,
                            "Not mergeable".to_string(),
                        ))
                    }
                }
            }
            MergeBotState::WaitingForOperation {
                queue,
                current_index,
                operation: Operation::WaitForMergeConfirmation,
            } => {
                // We're waiting for merge confirmation, return poll action
                // Use short sleep (2s) since merge is usually quick
                let pr_number = queue[current_index].pr_number;
                self.state = MergeBotState::WaitingForOperation {
                    queue,
                    current_index,
                    operation: Operation::WaitForMergeConfirmation,
                };
                Some(MergeBotAction::PollMergeStatus(pr_number, false))
            }
            MergeBotState::WaitingForOperation {
                queue,
                current_index,
                operation: Operation::CheckCI,
            } => {
                // We're waiting for CI to complete, return poll action
                // Use long sleep (15s) since CI can take 4-10 minutes
                let pr_number = queue[current_index].pr_number;
                self.state = MergeBotState::WaitingForOperation {
                    queue,
                    current_index,
                    operation: Operation::CheckCI,
                };
                Some(MergeBotAction::PollMergeStatus(pr_number, true))
            }
            _ => {
                // Not in processing state, restore and return None
                self.state = state;
                None
            }
        }
    }

    /// Handle merge complete - called when Action::MergeComplete is received
    /// Transitions to waiting for merge confirmation via polling
    pub fn handle_merge_complete(&mut self, success: bool) {
        if let MergeBotState::WaitingForOperation {
            queue,
            current_index,
            operation: Operation::Merge,
        } = self.state.clone()
        {
            if success {
                // Don't immediately mark as merged - wait for API confirmation
                // Transition to waiting for merge confirmation
                self.state = MergeBotState::WaitingForOperation {
                    queue,
                    current_index,
                    operation: Operation::WaitForMergeConfirmation,
                };
            } else {
                let mut new_queue = queue;
                new_queue[current_index].status = PrQueueStatus::Failed("Merge failed".to_string());
                self.state = MergeBotState::ProcessingQueue {
                    queue: new_queue,
                    current_index: current_index + 1,
                };
            }
        }
    }

    /// Handle rebase complete - called when Action::RebaseComplete is received
    pub fn handle_rebase_complete(&mut self, success: bool) {
        if let MergeBotState::WaitingForOperation {
            queue,
            current_index,
            operation: Operation::Rebase,
        } = self.state.clone()
        {
            if success {
                // After rebase, need to wait for CI and then check if PR is ready
                self.state = MergeBotState::WaitingForOperation {
                    queue,
                    current_index,
                    operation: Operation::CheckCI,
                };
            } else {
                let mut new_queue = queue;
                new_queue[current_index].status =
                    PrQueueStatus::Failed("Rebase failed".to_string());
                self.state = MergeBotState::ProcessingQueue {
                    queue: new_queue,
                    current_index: current_index + 1,
                };
            }
        }
    }

    /// Handle PR merge confirmation - called when Action::PRMergedConfirmed is received
    pub fn handle_pr_merged_confirmed(&mut self, pr_number: usize, is_merged: bool) {
        if let MergeBotState::WaitingForOperation {
            queue,
            current_index,
            operation: Operation::WaitForMergeConfirmation,
        } = &self.state
            && queue[*current_index].pr_number == pr_number {
                if is_merged {
                    // PR is confirmed merged, mark as complete and move to next
                    let mut new_queue = queue.clone();
                    new_queue[*current_index].status = PrQueueStatus::Merged;
                    self.state = MergeBotState::ProcessingQueue {
                        queue: new_queue,
                        current_index: current_index + 1,
                    };
                } else {
                    // Still waiting for merge to be confirmed, keep polling
                    // State remains the same
                }
            }
    }

    /// Handle PR status update - check if we can proceed after waiting for CI
    pub fn handle_status_update(&mut self, pr_number: usize, status: MergeableStatus) {
        if let MergeBotState::WaitingForOperation {
            queue,
            current_index,
            operation: Operation::CheckCI,
        } = &self.state
            && queue[*current_index].pr_number == pr_number {
                match status {
                    MergeableStatus::Ready => {
                        // CI passed, go back to processing to merge
                        self.state = MergeBotState::ProcessingQueue {
                            queue: queue.clone(),
                            current_index: *current_index,
                        };
                    }
                    MergeableStatus::BuildFailed => {
                        // CI failed, skip this PR
                        let mut new_queue = queue.clone();
                        new_queue[*current_index].status =
                            PrQueueStatus::Failed("Build failed".to_string());
                        self.state = MergeBotState::ProcessingQueue {
                            queue: new_queue,
                            current_index: current_index + 1,
                        };
                    }
                    MergeableStatus::NeedsRebase => {
                        // After merge, PR needs rebase again
                        self.state = MergeBotState::ProcessingQueue {
                            queue: queue.clone(),
                            current_index: *current_index,
                        };
                    }
                    _ => {
                        // Still waiting...
                    }
                }
            }
    }
}

/// Actions that the merge bot wants to dispatch
#[derive(Debug, Clone)]
pub enum MergeBotAction {
    DispatchMerge(Vec<usize>),      // PR indices to merge
    DispatchRebase(Vec<usize>),     // PR indices to rebase
    WaitForCI(usize),               // PR number
    PollMergeStatus(usize, bool),   // PR number, is_checking_ci - start polling to confirm merge
    PrSkipped(usize, String),       // PR number, reason
    Completed,
}
