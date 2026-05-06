mod cache;
mod diff_paths;
mod git_commands;
mod resolution;
mod runtime;
mod snapshot;

use serde::Serialize;

pub use cache::{
    hydrated_worktree_vcs_snapshot_cache_entry, pending_worktree_vcs_snapshot_cache_entry,
    publish_worktree_vcs_snapshot_cache_entry, published_worktree_vcs_snapshot_cache_entry,
    WorktreeVcsSnapshotCacheEntry, WorktreeVcsSnapshotPublishPolicy, WORKTREE_VCS_DEBOUNCE_MS,
    WORKTREE_VCS_MAX_INTERVAL_MS,
};
pub use diff_paths::{
    build_diff_path_states, count_diff_paths, load_diff_file_count_from_source,
    load_diff_touched_entries_from_source, WorktreeVcsDiffPathSource,
};
pub use git_commands::{
    parse_git_diff_name_status, parse_git_list_untracked, parse_git_refs, parse_git_single_ref,
    WorktreeVcsGitCommand,
};
pub use resolution::{is_no_vcs_repo_error, WorktreeDiffBaseResolution};
pub use runtime::{
    claim_next_worktree_vcs_job, finish_worktree_vcs_job, finish_worktree_vcs_refresh,
    mark_worktree_vcs_runtime_dirty, queue_worktree_vcs_refresh, worktree_vcs_enabled_from_env,
    worktree_vcs_scheduler_concurrency_from_env, WorktreeVcsDirtyBits, WorktreeVcsInvalidation,
    WorktreeVcsRuntimeState, WorktreeVcsSchedulerJob, WorktreeVcsSchedulerRuntime,
};
pub use snapshot::{
    build_git_status_entries, build_git_status_summary, build_large_change_set_touched_files,
    build_touched_files, build_worktree_vcs_snapshot, derive_worktree_vcs_freshness, now_epoch_ms,
    plan_worktree_vcs_commit_info, snapshot_fingerprint, snapshot_for_durable_cache,
    summary_from_file_count, summary_has_counts, WorktreeVcsCommitInfoPlan,
    WorktreeVcsCommitLookup, WorktreeVcsSnapshotBuildParts, WorktreeVcsSnapshotCommitInfo,
};

#[derive(Debug, Clone, Serialize)]
pub struct GitStatusSnapshot {
    pub raw: String,
    pub summary_line: String,
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: i64,
    pub behind: i64,
    pub detached: bool,
    pub staged: i64,
    pub unstaged: i64,
    pub untracked: i64,
    pub entries: Vec<GitStatusEntry>,
    pub entries_total_count: i64,
    pub entries_truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct GitStatusEntry {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orig_path: Option<String>,
    pub index_status: String,
    pub worktree_status: String,
}

pub const WORKTREE_VCS_TOUCHED_FILES_CAP: usize = 200;
// Above this count, the product surfaces an exact summary but does not compute
// or stream file-by-file review inventory.
pub const WORKTREE_VCS_REVIEWABLE_FILE_LIMIT: i64 = 300;
pub const WORKTREE_VCS_SNAPSHOT_SCHEMA_VERSION: i64 = 2;
pub const DEFAULT_WORKTREE_VCS_SCHEDULER_CONCURRENCY: usize = 2;
