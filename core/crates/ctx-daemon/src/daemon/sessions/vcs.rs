use std::path::Path;

use anyhow::Error;
use ctx_core::ids::SessionId;
use ctx_core::models::{DiffUnavailableReason, Session, Worktree};
use ctx_workspace_services::worktree_vcs::{
    apply_worktree_vcs_session_patch, is_no_vcs_repo_error,
    session_git_status_summary_from_snapshot, worktree_vcs_diff_summary_mismatch,
    WorktreeDiffBaseResolution, WorktreeVcsDiffBaseQuery,
};

use crate::daemon::SessionsHandle;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionVcsDiffQuery {
    pub base_commit_sha: Option<String>,
    pub target_branch: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionVcsApplyAction {
    Accept,
    Reject,
}

impl SessionVcsApplyAction {
    fn reverse_patch(self) -> bool {
        matches!(self, Self::Reject)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionVcsDiff {
    pub diff: String,
    pub available: bool,
    pub unavailable_reason: Option<DiffUnavailableReason>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionVcsDiffSummary {
    pub base_commit_sha: String,
    pub head_commit_sha: String,
    pub file_count: i64,
    pub line_additions: i64,
    pub line_deletions: i64,
    pub available: bool,
    pub unavailable_reason: Option<DiffUnavailableReason>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionVcsGitStatus {
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
    pub entries: Vec<SessionVcsGitStatusEntry>,
    pub entries_truncated: bool,
    pub entries_total_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionVcsGitStatusEntry {
    pub path: String,
    pub orig_path: Option<String>,
    pub index_status: String,
    pub worktree_status: String,
}

#[derive(Debug)]
pub enum SessionVcsError {
    NotFound,
    InvalidExplicitTarget(String),
    BadPatch(Error),
    Internal(Error),
}

struct SessionVcsContext {
    session: Session,
    worktree: Worktree,
}

enum PreparedSessionDiffRequest {
    Available {
        ctx: SessionVcsContext,
        base_commit_sha: String,
    },
    NoRepo {
        ctx: SessionVcsContext,
    },
    Unavailable {
        ctx: SessionVcsContext,
        base_commit_sha: String,
        reason: DiffUnavailableReason,
    },
}

impl SessionsHandle {
    pub async fn get_session_vcs_diff_for_request(
        &self,
        session_id: SessionId,
        query: SessionVcsDiffQuery,
    ) -> Result<SessionVcsDiff, SessionVcsError> {
        let (ctx, base_commit_sha) = match self
            .prepare_session_diff_request(session_id, query, "sessions.diff")
            .await?
        {
            PreparedSessionDiffRequest::Available {
                ctx,
                base_commit_sha,
            } => (ctx, base_commit_sha),
            PreparedSessionDiffRequest::NoRepo { .. } => {
                return Ok(session_vcs_diff_unavailable(DiffUnavailableReason::NoRepo));
            }
            PreparedSessionDiffRequest::Unavailable { reason, .. } => {
                return Ok(session_vcs_diff_unavailable(reason));
            }
        };
        let diff = match self
            .diff_worktree_for_session(&ctx.worktree, &base_commit_sha)
            .await
        {
            Ok(diff) => diff,
            Err(err) if is_no_vcs_repo_error(&err) => {
                return Ok(session_vcs_diff_unavailable(DiffUnavailableReason::NoRepo));
            }
            Err(err) => return Err(SessionVcsError::Internal(err)),
        };
        Ok(SessionVcsDiff {
            diff,
            available: true,
            unavailable_reason: None,
        })
    }

    pub async fn get_session_vcs_diff_summary_for_request(
        &self,
        session_id: SessionId,
        query: SessionVcsDiffQuery,
    ) -> Result<SessionVcsDiffSummary, SessionVcsError> {
        match self
            .prepare_session_diff_request(session_id, query, "sessions.diff_summary")
            .await?
        {
            PreparedSessionDiffRequest::Available {
                ctx,
                base_commit_sha,
            } => {
                self.session_vcs_diff_summary_available(&ctx.worktree, base_commit_sha)
                    .await
            }
            PreparedSessionDiffRequest::NoRepo { ctx } => Ok(session_vcs_diff_summary_unavailable(
                ctx.worktree.base_commit_sha.clone(),
                ctx.worktree.base_commit_sha,
                DiffUnavailableReason::NoRepo,
            )),
            PreparedSessionDiffRequest::Unavailable {
                ctx,
                base_commit_sha,
                reason,
            } => {
                let head_commit_sha = self
                    .resolve_head_commit_sha_or_base(&ctx.worktree, &base_commit_sha)
                    .await;
                Ok(session_vcs_diff_summary_unavailable(
                    base_commit_sha,
                    head_commit_sha,
                    reason,
                ))
            }
        }
    }

    pub async fn apply_session_vcs_diff_patch_for_request(
        &self,
        session_id: SessionId,
        action: SessionVcsApplyAction,
        patch: &str,
    ) -> Result<SessionVcsDiff, SessionVcsError> {
        let ctx = self.load_session_vcs_context(session_id).await?;
        apply_worktree_vcs_session_patch(
            Path::new(&ctx.worktree.root_path),
            patch,
            action.reverse_patch(),
        )
        .await
        .map_err(SessionVcsError::BadPatch)?;

        let resolution = self
            .resolve_session_diff_base(&ctx.worktree, SessionVcsDiffQuery::default())
            .await?;
        if let Some(unavailable_reason) = resolution.unavailable_reason.clone() {
            self.emit_compat_payload_reject_counter(
                "sessions.diff_apply",
                "no_target_branch",
                None,
            )
            .await;
            return Ok(session_vcs_diff_unavailable(unavailable_reason));
        }
        let diff = self
            .diff_worktree_for_session(&ctx.worktree, &resolution.base_commit_sha)
            .await
            .map_err(SessionVcsError::Internal)?;
        Ok(SessionVcsDiff {
            diff,
            available: true,
            unavailable_reason: None,
        })
    }

    pub async fn get_session_vcs_git_status_for_request(
        &self,
        session_id: SessionId,
    ) -> Result<SessionVcsGitStatus, SessionVcsError> {
        let ctx = self.load_session_vcs_context(session_id).await?;
        let snapshot = self
            .load_git_status_snapshot(&ctx.worktree, true, true)
            .await
            .map_err(SessionVcsError::Internal)?;
        let summary = session_git_status_summary_from_snapshot(&snapshot);
        let status = SessionVcsGitStatus {
            raw: snapshot.raw,
            summary_line: snapshot.summary_line,
            branch: snapshot.branch,
            upstream: snapshot.upstream,
            ahead: snapshot.ahead,
            behind: snapshot.behind,
            detached: snapshot.detached,
            staged: snapshot.staged,
            unstaged: snapshot.unstaged,
            untracked: snapshot.untracked,
            entries: snapshot
                .entries
                .into_iter()
                .map(|entry| SessionVcsGitStatusEntry {
                    path: entry.path,
                    orig_path: entry.orig_path,
                    index_status: entry.index_status,
                    worktree_status: entry.worktree_status,
                })
                .collect(),
            entries_truncated: snapshot.entries_truncated,
            entries_total_count: snapshot.entries_total_count,
        };
        if let Err(err) = self
            .persist_session_git_status_summary(ctx.session.id, ctx.worktree.id, &summary)
            .await
        {
            tracing::warn!(
                session_id = %ctx.session.id.0,
                "git status summary persist failed: {err:?}"
            );
        }
        Ok(status)
    }

    async fn prepare_session_diff_request(
        &self,
        session_id: SessionId,
        query: SessionVcsDiffQuery,
        compat_route: &'static str,
    ) -> Result<PreparedSessionDiffRequest, SessionVcsError> {
        let ctx = self.load_session_vcs_context(session_id).await?;
        if !self
            .worktree_has_vcs_repo(&ctx.worktree)
            .await
            .map_err(SessionVcsError::Internal)?
        {
            return Ok(PreparedSessionDiffRequest::NoRepo { ctx });
        }
        let resolution = self.resolve_session_diff_base(&ctx.worktree, query).await?;
        if let Some(reason) = resolution.unavailable_reason.clone() {
            self.emit_compat_payload_reject_counter(compat_route, "no_target_branch", None)
                .await;
            return Ok(PreparedSessionDiffRequest::Unavailable {
                ctx,
                base_commit_sha: resolution.base_commit_sha,
                reason,
            });
        }
        Ok(PreparedSessionDiffRequest::Available {
            ctx,
            base_commit_sha: resolution.base_commit_sha,
        })
    }

    async fn load_session_vcs_context(
        &self,
        session_id: SessionId,
    ) -> Result<SessionVcsContext, SessionVcsError> {
        let (session, worktree) = self
            .load_session_vcs_parts(session_id)
            .await
            .map_err(SessionVcsError::Internal)?
            .ok_or(SessionVcsError::NotFound)?;
        Ok(SessionVcsContext { session, worktree })
    }

    async fn resolve_session_diff_base(
        &self,
        worktree: &Worktree,
        query: SessionVcsDiffQuery,
    ) -> Result<WorktreeDiffBaseResolution, SessionVcsError> {
        let resolution = self
            .resolve_worktree_diff_base(
                worktree,
                WorktreeVcsDiffBaseQuery {
                    base_commit_sha: query.base_commit_sha,
                    target_branch: query.target_branch,
                },
            )
            .await;
        if resolution.explicit_target {
            if let Some(error) = resolution.error.clone() {
                return Err(SessionVcsError::InvalidExplicitTarget(error));
            }
        }
        Ok(resolution)
    }

    async fn session_vcs_diff_summary_available(
        &self,
        worktree: &Worktree,
        base_commit_sha: String,
    ) -> Result<SessionVcsDiffSummary, SessionVcsError> {
        let summary_counts = match self
            .diff_worktree_summary_for_session(worktree, &base_commit_sha)
            .await
        {
            Ok(counts) => Ok(counts),
            Err(err) if is_no_vcs_repo_error(&err) => Err(DiffUnavailableReason::NoRepo),
            Err(err) => return Err(SessionVcsError::Internal(err)),
        };
        let head_commit_sha = self
            .resolve_head_commit_sha_or_base(worktree, &base_commit_sha)
            .await;
        match summary_counts {
            Ok(counts) => {
                if let Some(snapshot) = self.get_worktree_vcs_snapshot(worktree.id).await {
                    if let Some(mismatch) =
                        worktree_vcs_diff_summary_mismatch(&snapshot, &base_commit_sha, counts)
                    {
                        tracing::warn!(
                            worktree_id = %worktree.id.0,
                            snapshot_rev = snapshot.rev,
                            base_commit_sha = %base_commit_sha,
                            snapshot_file_count = ?mismatch.snapshot_file_count,
                            snapshot_additions = ?mismatch.snapshot_line_additions,
                            snapshot_deletions = ?mismatch.snapshot_line_deletions,
                            summary_file_count = mismatch.actual_file_count,
                            summary_additions = mismatch.actual_line_additions,
                            summary_deletions = mismatch.actual_line_deletions,
                            "worktree vcs snapshot summary mismatch"
                        );
                    }
                }
                Ok(SessionVcsDiffSummary {
                    base_commit_sha,
                    head_commit_sha,
                    file_count: counts.file_count,
                    line_additions: counts.line_additions,
                    line_deletions: counts.line_deletions,
                    available: true,
                    unavailable_reason: None,
                })
            }
            Err(unavailable_reason) => Ok(session_vcs_diff_summary_unavailable(
                base_commit_sha,
                head_commit_sha,
                unavailable_reason,
            )),
        }
    }

    async fn resolve_head_commit_sha_or_base(
        &self,
        worktree: &Worktree,
        base_commit_sha: &str,
    ) -> String {
        self.resolve_worktree_commit(worktree, "HEAD")
            .await
            .unwrap_or_else(|_| base_commit_sha.to_string())
    }
}

fn session_vcs_diff_unavailable(reason: DiffUnavailableReason) -> SessionVcsDiff {
    SessionVcsDiff {
        diff: String::new(),
        available: false,
        unavailable_reason: Some(reason),
    }
}

fn session_vcs_diff_summary_unavailable(
    base_commit_sha: String,
    head_commit_sha: String,
    reason: DiffUnavailableReason,
) -> SessionVcsDiffSummary {
    SessionVcsDiffSummary {
        base_commit_sha,
        head_commit_sha,
        file_count: 0,
        line_additions: 0,
        line_deletions: 0,
        available: false,
        unavailable_reason: Some(reason),
    }
}
