use ctx_core::ids::SessionId;
use ctx_core::models::DiffUnavailableReason;
use ctx_observability::logs;
use serde::{Deserialize, Serialize};

use crate::daemon::sessions::route_contract::parse_session_route_id;
use crate::daemon::sessions::vcs::{
    SessionVcsApplyAction, SessionVcsDiff, SessionVcsDiffQuery, SessionVcsDiffSummary,
    SessionVcsError, SessionVcsGitStatus, SessionVcsGitStatusEntry,
};
use crate::daemon::{SessionRouteParams, SessionsHandle};

fn is_true(v: &bool) -> bool {
    *v
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq, Eq)]
pub struct SessionVcsRouteQuery {
    pub base_commit_sha: Option<String>,
    pub target_branch: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ApplySessionVcsDiffPatchRouteRequest {
    action: String,
    patch: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SessionVcsDiffRouteResponse {
    pub diff: String,
    #[serde(skip_serializing_if = "is_true")]
    pub available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<DiffUnavailableReason>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SessionVcsDiffSummaryRouteResponse {
    pub base_commit_sha: String,
    pub head_commit_sha: String,
    pub file_count: i64,
    pub line_additions: i64,
    pub line_deletions: i64,
    #[serde(skip_serializing_if = "is_true")]
    pub available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<DiffUnavailableReason>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SessionVcsGitStatusRouteResponse {
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<SessionVcsGitStatusEntryRouteResponse>,
    pub entries_truncated: bool,
    pub entries_total_count: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SessionVcsGitStatusEntryRouteResponse {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orig_path: Option<String>,
    pub index_status: String,
    pub worktree_status: String,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum SessionVcsRouteErrorKind {
    BadRequest,
    NotFound,
    Internal,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SessionVcsRouteError {
    kind: SessionVcsRouteErrorKind,
    message: String,
}

impl SessionVcsRouteError {
    fn new(kind: SessionVcsRouteErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    fn bad_request(message: impl Into<String>) -> Self {
        Self::new(SessionVcsRouteErrorKind::BadRequest, message)
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self::new(SessionVcsRouteErrorKind::NotFound, message)
    }

    fn internal(message: impl Into<String>) -> Self {
        Self::new(SessionVcsRouteErrorKind::Internal, message)
    }

    pub fn kind(&self) -> SessionVcsRouteErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl SessionsHandle {
    pub async fn get_session_vcs_diff_for_route(
        &self,
        params: SessionRouteParams,
        query: SessionVcsRouteQuery,
    ) -> Result<SessionVcsDiffRouteResponse, SessionVcsRouteError> {
        let session_id = parse_session_vcs_route_id(params)?;
        self.get_session_vcs_diff_for_request(session_id, session_vcs_diff_query(query))
            .await
            .map(session_vcs_diff_response)
            .map_err(session_vcs_route_error)
    }

    pub async fn get_session_vcs_diff_summary_for_route(
        &self,
        params: SessionRouteParams,
        query: SessionVcsRouteQuery,
    ) -> Result<SessionVcsDiffSummaryRouteResponse, SessionVcsRouteError> {
        let session_id = parse_session_vcs_route_id(params)?;
        self.get_session_vcs_diff_summary_for_request(session_id, session_vcs_diff_query(query))
            .await
            .map(session_vcs_diff_summary_response)
            .map_err(session_vcs_route_error)
    }

    pub async fn apply_session_vcs_diff_patch_for_route(
        &self,
        params: SessionRouteParams,
        request: ApplySessionVcsDiffPatchRouteRequest,
    ) -> Result<SessionVcsDiffRouteResponse, SessionVcsRouteError> {
        let session_id = parse_session_vcs_route_id(params)?;
        let action = parse_session_vcs_apply_action(&request)?;
        self.apply_session_vcs_diff_patch_for_request(session_id, action, &request.patch)
            .await
            .map(session_vcs_diff_response)
            .map_err(session_vcs_route_error)
    }

    pub async fn get_session_vcs_git_status_for_route(
        &self,
        params: SessionRouteParams,
    ) -> Result<SessionVcsGitStatusRouteResponse, SessionVcsRouteError> {
        let session_id = parse_session_vcs_route_id(params)?;
        self.get_session_vcs_git_status_for_request(session_id)
            .await
            .map(session_vcs_git_status_response)
            .map_err(session_vcs_route_error)
    }
}

fn parse_session_vcs_route_id(
    params: SessionRouteParams,
) -> Result<SessionId, SessionVcsRouteError> {
    parse_session_route_id(params.session_id())
        .map_err(|_| SessionVcsRouteError::bad_request("invalid session id"))
}

fn session_vcs_diff_query(query: SessionVcsRouteQuery) -> SessionVcsDiffQuery {
    SessionVcsDiffQuery {
        base_commit_sha: query.base_commit_sha,
        target_branch: query.target_branch,
    }
}

fn parse_session_vcs_apply_action(
    request: &ApplySessionVcsDiffPatchRouteRequest,
) -> Result<SessionVcsApplyAction, SessionVcsRouteError> {
    if request.patch.trim().is_empty() {
        return Err(SessionVcsRouteError::bad_request("patch is empty"));
    }
    match request.action.trim().to_lowercase().as_str() {
        "accept" => Ok(SessionVcsApplyAction::Accept),
        "reject" => Ok(SessionVcsApplyAction::Reject),
        _ => Err(SessionVcsRouteError::bad_request(
            "action must be accept or reject",
        )),
    }
}

fn session_vcs_route_error(error: SessionVcsError) -> SessionVcsRouteError {
    match error {
        SessionVcsError::NotFound => SessionVcsRouteError::not_found("workspace not found"),
        SessionVcsError::InvalidExplicitTarget(error) => SessionVcsRouteError::bad_request(error),
        SessionVcsError::BadPatch(error) => {
            SessionVcsRouteError::bad_request(logs::redact_sensitive(&error.to_string()))
        }
        SessionVcsError::Internal(error) => {
            SessionVcsRouteError::internal(logs::redact_sensitive(&error.to_string()))
        }
    }
}

fn session_vcs_diff_response(outcome: SessionVcsDiff) -> SessionVcsDiffRouteResponse {
    SessionVcsDiffRouteResponse {
        diff: outcome.diff,
        available: outcome.available,
        unavailable_reason: outcome.unavailable_reason,
    }
}

fn session_vcs_diff_summary_response(
    outcome: SessionVcsDiffSummary,
) -> SessionVcsDiffSummaryRouteResponse {
    SessionVcsDiffSummaryRouteResponse {
        base_commit_sha: outcome.base_commit_sha,
        head_commit_sha: outcome.head_commit_sha,
        file_count: outcome.file_count,
        line_additions: outcome.line_additions,
        line_deletions: outcome.line_deletions,
        available: outcome.available,
        unavailable_reason: outcome.unavailable_reason,
    }
}

fn session_vcs_git_status_response(
    status: SessionVcsGitStatus,
) -> SessionVcsGitStatusRouteResponse {
    SessionVcsGitStatusRouteResponse {
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
            .map(session_vcs_git_status_entry_response)
            .collect(),
        entries_truncated: status.entries_truncated,
        entries_total_count: status.entries_total_count,
    }
}

fn session_vcs_git_status_entry_response(
    entry: SessionVcsGitStatusEntry,
) -> SessionVcsGitStatusEntryRouteResponse {
    SessionVcsGitStatusEntryRouteResponse {
        path: entry.path,
        orig_path: entry.orig_path,
        index_status: entry.index_status,
        worktree_status: entry.worktree_status,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;
    use serde_json::json;

    fn apply_request(action: &str, patch: &str) -> ApplySessionVcsDiffPatchRouteRequest {
        ApplySessionVcsDiffPatchRouteRequest {
            action: action.to_string(),
            patch: patch.to_string(),
        }
    }

    #[test]
    fn route_query_and_body_preserve_current_serde_shape() {
        let query: SessionVcsRouteQuery = serde_json::from_value(json!({
            "base_commit_sha": "base",
            "target_branch": "main",
            "ignored": true
        }))
        .unwrap();
        assert_eq!(query.base_commit_sha.as_deref(), Some("base"));
        assert_eq!(query.target_branch.as_deref(), Some("main"));

        let query_defaults: SessionVcsRouteQuery =
            serde_json::from_value(json!({ "ignored": true })).unwrap();
        assert_eq!(query_defaults.base_commit_sha, None);
        assert_eq!(query_defaults.target_branch, None);

        let request: ApplySessionVcsDiffPatchRouteRequest = serde_json::from_value(json!({
            "action": "accept",
            "patch": "diff --git a/file b/file",
            "ignored": true
        }))
        .unwrap();
        assert_eq!(request.action, "accept");
        assert_eq!(request.patch, "diff --git a/file b/file");
    }

    #[test]
    fn diff_response_preserves_available_serde_contract() {
        let available = session_vcs_diff_response(SessionVcsDiff {
            diff: "diff".to_string(),
            available: true,
            unavailable_reason: None,
        });
        assert_eq!(
            serde_json::to_value(available).unwrap(),
            json!({ "diff": "diff" })
        );

        let unavailable = session_vcs_diff_response(SessionVcsDiff {
            diff: String::new(),
            available: false,
            unavailable_reason: Some(DiffUnavailableReason::NoRepo),
        });
        assert_eq!(
            serde_json::to_value(unavailable).unwrap(),
            json!({
                "diff": "",
                "available": false,
                "unavailable_reason": "no_repo"
            })
        );
    }

    #[test]
    fn diff_summary_response_preserves_available_serde_contract() {
        let available = session_vcs_diff_summary_response(SessionVcsDiffSummary {
            base_commit_sha: "base".to_string(),
            head_commit_sha: "head".to_string(),
            file_count: 1,
            line_additions: 2,
            line_deletions: 3,
            available: true,
            unavailable_reason: None,
        });
        assert_eq!(
            serde_json::to_value(available).unwrap(),
            json!({
                "base_commit_sha": "base",
                "head_commit_sha": "head",
                "file_count": 1,
                "line_additions": 2,
                "line_deletions": 3
            })
        );

        let unavailable = session_vcs_diff_summary_response(SessionVcsDiffSummary {
            base_commit_sha: "base".to_string(),
            head_commit_sha: "head".to_string(),
            file_count: 0,
            line_additions: 0,
            line_deletions: 0,
            available: false,
            unavailable_reason: Some(DiffUnavailableReason::NoTargetBranch),
        });
        assert_eq!(
            serde_json::to_value(unavailable).unwrap(),
            json!({
                "base_commit_sha": "base",
                "head_commit_sha": "head",
                "file_count": 0,
                "line_additions": 0,
                "line_deletions": 0,
                "available": false,
                "unavailable_reason": "no_target_branch"
            })
        );
    }

    #[test]
    fn git_status_response_preserves_entry_serde_contract() {
        let empty_entries = session_vcs_git_status_response(SessionVcsGitStatus {
            raw: "raw".to_string(),
            summary_line: "summary".to_string(),
            branch: None,
            upstream: None,
            ahead: 0,
            behind: 0,
            detached: false,
            staged: 0,
            unstaged: 0,
            untracked: 0,
            entries: Vec::new(),
            entries_truncated: false,
            entries_total_count: 0,
        });
        let value = serde_json::to_value(empty_entries).unwrap();
        assert!(value.get("entries").is_none());

        let with_entry = session_vcs_git_status_response(SessionVcsGitStatus {
            raw: "raw".to_string(),
            summary_line: "summary".to_string(),
            branch: Some("main".to_string()),
            upstream: Some("origin/main".to_string()),
            ahead: 1,
            behind: 2,
            detached: false,
            staged: 3,
            unstaged: 4,
            untracked: 5,
            entries: vec![SessionVcsGitStatusEntry {
                path: "renamed.rs".to_string(),
                orig_path: None,
                index_status: "R".to_string(),
                worktree_status: "M".to_string(),
            }],
            entries_truncated: true,
            entries_total_count: 1,
        });
        assert_eq!(
            serde_json::to_value(with_entry).unwrap(),
            json!({
                "raw": "raw",
                "summary_line": "summary",
                "branch": "main",
                "upstream": "origin/main",
                "ahead": 1,
                "behind": 2,
                "detached": false,
                "staged": 3,
                "unstaged": 4,
                "untracked": 5,
                "entries": [{
                    "path": "renamed.rs",
                    "index_status": "R",
                    "worktree_status": "M"
                }],
                "entries_truncated": true,
                "entries_total_count": 1
            })
        );
    }

    #[test]
    fn invalid_session_id_uses_existing_route_message() {
        let error =
            parse_session_vcs_route_id(SessionRouteParams::new("not-a-session")).unwrap_err();
        assert_eq!(error.kind(), SessionVcsRouteErrorKind::BadRequest);
        assert_eq!(error.message(), "invalid session id");
    }

    #[test]
    fn apply_action_preserves_trim_case_and_precedence() {
        assert_eq!(
            parse_session_vcs_apply_action(&apply_request(" ACCEPT ", "patch")).unwrap(),
            SessionVcsApplyAction::Accept
        );
        assert_eq!(
            parse_session_vcs_apply_action(&apply_request("reject", "patch")).unwrap(),
            SessionVcsApplyAction::Reject
        );

        let empty_patch =
            parse_session_vcs_apply_action(&apply_request("bogus", "   ")).unwrap_err();
        assert_eq!(empty_patch.kind(), SessionVcsRouteErrorKind::BadRequest);
        assert_eq!(empty_patch.message(), "patch is empty");

        let invalid_action =
            parse_session_vcs_apply_action(&apply_request("bogus", "patch")).unwrap_err();
        assert_eq!(invalid_action.kind(), SessionVcsRouteErrorKind::BadRequest);
        assert_eq!(invalid_action.message(), "action must be accept or reject");
    }

    #[test]
    fn low_level_errors_map_to_route_errors_with_redaction() {
        let not_found = session_vcs_route_error(SessionVcsError::NotFound);
        assert_eq!(not_found.kind(), SessionVcsRouteErrorKind::NotFound);
        assert_eq!(not_found.message(), "workspace not found");

        let invalid_target =
            session_vcs_route_error(SessionVcsError::InvalidExplicitTarget("bad target".into()));
        assert_eq!(invalid_target.kind(), SessionVcsRouteErrorKind::BadRequest);
        assert_eq!(invalid_target.message(), "bad target");

        let raw_message = "CTX_MCP_TOKEN=secret-token-123";
        let bad_patch = session_vcs_route_error(SessionVcsError::BadPatch(anyhow!(raw_message)));
        assert_eq!(bad_patch.kind(), SessionVcsRouteErrorKind::BadRequest);
        assert_eq!(bad_patch.message(), logs::redact_sensitive(raw_message));
        assert!(!bad_patch.message().contains("secret-token-123"));

        let internal = session_vcs_route_error(SessionVcsError::Internal(anyhow!(raw_message)));
        assert_eq!(internal.kind(), SessionVcsRouteErrorKind::Internal);
        assert_eq!(internal.message(), logs::redact_sensitive(raw_message));
        assert!(!internal.message().contains("secret-token-123"));
    }
}
