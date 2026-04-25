use super::*;

#[path = "vcs/apply.rs"]
mod apply;
#[path = "vcs/base.rs"]
mod base;
#[path = "vcs/context.rs"]
mod context;
#[path = "vcs/diff.rs"]
mod diff;
#[path = "vcs/git_status.rs"]
mod git_status;

pub(crate) use apply::apply_session_diff_patch;
pub(crate) use base::{
    is_no_vcs_repo_error, resolve_diff_base_with_meta, resolve_session_diff_base,
    WorktreeDiffBaseResolution,
};
pub(crate) use diff::{get_session_diff, get_session_diff_summary};
pub(crate) use git_status::get_session_git_status;

fn is_true(v: &bool) -> bool {
    *v
}

#[derive(Debug, Deserialize)]
pub(crate) struct SessionDiffApplyReq {
    pub(crate) action: String,
    pub(crate) patch: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct SessionDiffResponse {
    pub(crate) diff: String,
    #[serde(skip_serializing_if = "is_true")]
    pub(crate) available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) unavailable_reason: Option<DiffUnavailableReason>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct SessionDiffQuery {
    pub(crate) base_commit_sha: Option<String>,
    pub(crate) target_branch: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SessionDiffSummaryResponse {
    pub(crate) base_commit_sha: String,
    pub(crate) head_commit_sha: String,
    pub(crate) file_count: i64,
    pub(crate) line_additions: i64,
    pub(crate) line_deletions: i64,
    #[serde(skip_serializing_if = "is_true")]
    pub(crate) available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) unavailable_reason: Option<DiffUnavailableReason>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SessionGitStatusResponse {
    pub(crate) raw: String,
    pub(crate) summary_line: String,
    pub(crate) branch: Option<String>,
    pub(crate) upstream: Option<String>,
    pub(crate) ahead: i64,
    pub(crate) behind: i64,
    pub(crate) detached: bool,
    pub(crate) staged: i64,
    pub(crate) unstaged: i64,
    pub(crate) untracked: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) entries: Vec<GitStatusEntry>,
    pub(crate) entries_truncated: bool,
    pub(crate) entries_total_count: i64,
}
