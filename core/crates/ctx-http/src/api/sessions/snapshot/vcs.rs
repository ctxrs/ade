use super::*;
use ctx_daemon::daemon::sessions::vcs::{
    SessionVcsApplyAction, SessionVcsDiff, SessionVcsDiffQuery, SessionVcsDiffSummary,
    SessionVcsError, SessionVcsGitStatus, SessionVcsGitStatusEntry,
};

#[path = "vcs/apply.rs"]
mod apply;
#[path = "vcs/diff.rs"]
mod diff;
#[path = "vcs/git_status.rs"]
mod git_status;

pub(crate) use apply::apply_session_diff_patch;
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
pub(crate) struct SessionDiffRouteQuery {
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
    pub(crate) entries: Vec<SessionGitStatusEntryResponse>,
    pub(crate) entries_truncated: bool,
    pub(crate) entries_total_count: i64,
}

#[derive(Debug, Serialize)]
pub(crate) struct SessionGitStatusEntryResponse {
    pub(crate) path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) orig_path: Option<String>,
    pub(crate) index_status: String,
    pub(crate) worktree_status: String,
}

fn parse_session_id(id: &str) -> Result<SessionId, (StatusCode, Json<ApiErrorResp>)> {
    Ok(SessionId(uuid::Uuid::parse_str(id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?))
}

fn session_diff_query(q: SessionDiffRouteQuery) -> SessionVcsDiffQuery {
    SessionVcsDiffQuery {
        base_commit_sha: q.base_commit_sha,
        target_branch: q.target_branch,
    }
}

fn parse_session_vcs_apply_action(
    action: &str,
) -> Result<SessionVcsApplyAction, (StatusCode, Json<ApiErrorResp>)> {
    match action.trim().to_lowercase().as_str() {
        "accept" => Ok(SessionVcsApplyAction::Accept),
        "reject" => Ok(SessionVcsApplyAction::Reject),
        _ => Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "action must be accept or reject".to_string(),
            }),
        )),
    }
}

fn map_session_vcs_error(err: SessionVcsError) -> (StatusCode, Json<ApiErrorResp>) {
    match err {
        SessionVcsError::NotFound => (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "workspace not found".to_string(),
            }),
        ),
        SessionVcsError::InvalidExplicitTarget(error) => {
            (StatusCode::BAD_REQUEST, Json(ApiErrorResp { error }))
        }
        SessionVcsError::BadPatch(err) => (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&err.to_string()),
            }),
        ),
        SessionVcsError::Internal(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&err.to_string()),
            }),
        ),
    }
}

fn session_diff_response(outcome: SessionVcsDiff) -> SessionDiffResponse {
    SessionDiffResponse {
        diff: outcome.diff,
        available: outcome.available,
        unavailable_reason: outcome.unavailable_reason,
    }
}

fn session_diff_summary_response(outcome: SessionVcsDiffSummary) -> SessionDiffSummaryResponse {
    SessionDiffSummaryResponse {
        base_commit_sha: outcome.base_commit_sha,
        head_commit_sha: outcome.head_commit_sha,
        file_count: outcome.file_count,
        line_additions: outcome.line_additions,
        line_deletions: outcome.line_deletions,
        available: outcome.available,
        unavailable_reason: outcome.unavailable_reason,
    }
}

fn session_git_status_response(status: SessionVcsGitStatus) -> SessionGitStatusResponse {
    SessionGitStatusResponse {
        raw: status.raw,
        summary_line: status.summary_line,
        branch: status.branch,
        upstream: status.upstream,
        ahead: status.ahead,
        behind: status.behind,
        detached: status.detached,
        staged: status.staged,
        unstaged: status.unstaged,
        untracked: status.untracked,
        entries: status
            .entries
            .into_iter()
            .map(session_git_status_entry_response)
            .collect(),
        entries_truncated: status.entries_truncated,
        entries_total_count: status.entries_total_count,
    }
}

fn session_git_status_entry_response(
    entry: SessionVcsGitStatusEntry,
) -> SessionGitStatusEntryResponse {
    SessionGitStatusEntryResponse {
        path: entry.path,
        orig_path: entry.orig_path,
        index_status: entry.index_status,
        worktree_status: entry.worktree_status,
    }
}
