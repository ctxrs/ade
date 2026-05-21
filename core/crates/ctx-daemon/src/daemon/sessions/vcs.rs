use anyhow::Error;
use async_trait::async_trait;
use ctx_core::ids::{SessionId, WorktreeId};
use ctx_core::models::{Session, SessionGitStatusSummary, Worktree};
pub use ctx_session_vcs_service::vcs::{
    SessionVcsApplyAction, SessionVcsDiff, SessionVcsDiffQuery, SessionVcsDiffSummary,
    SessionVcsError, SessionVcsGitStatus, SessionVcsGitStatusEntry,
};
use ctx_session_vcs_service::vcs::{
    SessionVcsDataPlane, SessionVcsDiffBaseQuery, SessionVcsDiffBaseResolution,
    SessionVcsDiffSummaryCounts, SessionVcsDiffSummaryMismatch, SessionVcsGitStatusSnapshot,
    SessionVcsService,
};
use ctx_worktree_vcs_service::{
    apply_worktree_vcs_session_patch, is_no_vcs_repo_error as workspace_is_no_vcs_repo_error,
    resolve_worktree_diff_base_from_source, GitStatusEntry, GitStatusSnapshot,
    WorktreeDiffBaseResolution, WorktreeVcsCommitLookupSource, WorktreeVcsDiffBaseQuery,
    WorktreeVcsDiffSummaryCounts, WorktreeVcsDiffSummaryMismatch,
};

use crate::daemon::git_status::{
    load_git_status_snapshot, worktree_has_vcs_repo, HttpWorktreeVcsSource,
};
use crate::daemon::workspaces::{diff_worktree_for_session, diff_worktree_summary_for_session};
use crate::daemon::SessionsHandle;

impl SessionsHandle {
    pub async fn get_session_vcs_diff_for_request(
        &self,
        session_id: SessionId,
        query: SessionVcsDiffQuery,
    ) -> Result<SessionVcsDiff, SessionVcsError> {
        SessionVcsService::new(self)
            .get_session_vcs_diff(session_id, query)
            .await
    }

    pub async fn get_session_vcs_diff_summary_for_request(
        &self,
        session_id: SessionId,
        query: SessionVcsDiffQuery,
    ) -> Result<SessionVcsDiffSummary, SessionVcsError> {
        SessionVcsService::new(self)
            .get_session_vcs_diff_summary(session_id, query)
            .await
    }

    pub async fn apply_session_vcs_diff_patch_for_request(
        &self,
        session_id: SessionId,
        action: SessionVcsApplyAction,
        patch: &str,
    ) -> Result<SessionVcsDiff, SessionVcsError> {
        SessionVcsService::new(self)
            .apply_session_vcs_diff_patch(session_id, action, patch)
            .await
    }

    pub async fn get_session_vcs_git_status_for_request(
        &self,
        session_id: SessionId,
    ) -> Result<SessionVcsGitStatus, SessionVcsError> {
        SessionVcsService::new(self)
            .get_session_vcs_git_status(session_id)
            .await
    }
}

#[async_trait]
impl SessionVcsDataPlane for SessionsHandle {
    async fn load_session_vcs_parts(
        &self,
        session_id: SessionId,
    ) -> anyhow::Result<Option<(Session, Worktree)>> {
        let Some(store) = self.session_store_or_none(session_id).await? else {
            return Ok(None);
        };
        let Some(session) = store.get_session(session_id).await? else {
            return Ok(None);
        };
        let Some(worktree) = store.get_worktree(session.worktree_id).await? else {
            return Ok(None);
        };
        Ok(Some((session, worktree)))
    }

    async fn persist_session_git_status_summary(
        &self,
        session_id: SessionId,
        worktree_id: WorktreeId,
        summary: &SessionGitStatusSummary,
    ) -> anyhow::Result<()> {
        let store = self.store_for_session(session_id).await?;
        store
            .upsert_session_git_status_summary(session_id, worktree_id, summary)
            .await
    }

    async fn worktree_has_vcs_repo(&self, worktree: &Worktree) -> anyhow::Result<bool> {
        worktree_has_vcs_repo(&self.state, worktree).await
    }

    async fn load_git_status_snapshot(
        &self,
        worktree: &Worktree,
        include_untracked_files: bool,
        include_entries: bool,
    ) -> anyhow::Result<SessionVcsGitStatusSnapshot> {
        load_git_status_snapshot(
            &self.state,
            worktree,
            include_untracked_files,
            include_entries,
        )
        .await
        .map(session_vcs_git_status_snapshot)
    }

    async fn resolve_worktree_commit(
        &self,
        worktree: &Worktree,
        revision: &str,
    ) -> anyhow::Result<String> {
        let source = HttpWorktreeVcsSource::new(&self.state, worktree);
        source.resolve_commit(revision).await
    }

    async fn diff_worktree_for_session(
        &self,
        worktree: &Worktree,
        base_commit_sha: &str,
    ) -> anyhow::Result<String> {
        diff_worktree_for_session(&self.state, worktree, base_commit_sha).await
    }

    async fn diff_worktree_summary_for_session(
        &self,
        worktree: &Worktree,
        base_commit_sha: &str,
    ) -> anyhow::Result<SessionVcsDiffSummaryCounts> {
        diff_worktree_summary_for_session(&self.state, worktree, base_commit_sha)
            .await
            .map(session_vcs_diff_summary_counts)
    }

    async fn resolve_worktree_diff_base(
        &self,
        worktree: &Worktree,
        query: SessionVcsDiffBaseQuery,
    ) -> SessionVcsDiffBaseResolution {
        let source = HttpWorktreeVcsSource::new(&self.state, worktree);
        let resolution = resolve_worktree_diff_base_from_source(
            &source,
            worktree,
            WorktreeVcsDiffBaseQuery {
                base_commit_sha: query.base_commit_sha,
                target_branch: query.target_branch,
            },
        )
        .await;
        session_vcs_diff_base_resolution(resolution)
    }

    async fn apply_worktree_vcs_session_patch(
        &self,
        worktree: &Worktree,
        patch: &str,
        reverse_patch: bool,
    ) -> anyhow::Result<()> {
        apply_worktree_vcs_session_patch(
            std::path::Path::new(&worktree.root_path),
            patch,
            reverse_patch,
        )
        .await
    }

    async fn session_vcs_diff_summary_mismatch(
        &self,
        worktree: &Worktree,
        base_commit_sha: &str,
        counts: SessionVcsDiffSummaryCounts,
    ) -> Option<SessionVcsDiffSummaryMismatch> {
        let snapshot = self.state.get_worktree_vcs_snapshot(worktree.id).await?;
        ctx_worktree_vcs_service::worktree_vcs_diff_summary_mismatch(
            &snapshot,
            base_commit_sha,
            workspace_vcs_diff_summary_counts(counts),
        )
        .map(|mismatch| session_vcs_diff_summary_mismatch(snapshot.rev, mismatch))
    }

    async fn emit_compat_payload_reject_counter(&self, surface: &'static str, issue: &'static str) {
        self.emit_compat_payload_reject_counter(surface, issue, None)
            .await;
    }

    fn is_no_vcs_repo_error(&self, error: &Error) -> bool {
        workspace_is_no_vcs_repo_error(error)
    }
}

fn session_vcs_diff_base_resolution(
    resolution: WorktreeDiffBaseResolution,
) -> SessionVcsDiffBaseResolution {
    SessionVcsDiffBaseResolution {
        base_commit_sha: resolution.base_commit_sha,
        unavailable_reason: resolution.unavailable_reason,
        explicit_target: resolution.explicit_target,
        error: resolution.error,
    }
}

fn session_vcs_diff_summary_counts(
    counts: WorktreeVcsDiffSummaryCounts,
) -> SessionVcsDiffSummaryCounts {
    SessionVcsDiffSummaryCounts {
        file_count: counts.file_count,
        line_additions: counts.line_additions,
        line_deletions: counts.line_deletions,
    }
}

fn workspace_vcs_diff_summary_counts(
    counts: SessionVcsDiffSummaryCounts,
) -> WorktreeVcsDiffSummaryCounts {
    WorktreeVcsDiffSummaryCounts {
        file_count: counts.file_count,
        line_additions: counts.line_additions,
        line_deletions: counts.line_deletions,
    }
}

fn session_vcs_diff_summary_mismatch(
    snapshot_rev: i64,
    mismatch: WorktreeVcsDiffSummaryMismatch,
) -> SessionVcsDiffSummaryMismatch {
    SessionVcsDiffSummaryMismatch {
        snapshot_rev,
        snapshot_file_count: mismatch.snapshot_file_count,
        snapshot_line_additions: mismatch.snapshot_line_additions,
        snapshot_line_deletions: mismatch.snapshot_line_deletions,
        actual_file_count: mismatch.actual_file_count,
        actual_line_additions: mismatch.actual_line_additions,
        actual_line_deletions: mismatch.actual_line_deletions,
    }
}

fn session_vcs_git_status_snapshot(snapshot: GitStatusSnapshot) -> SessionVcsGitStatusSnapshot {
    SessionVcsGitStatusSnapshot {
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
            .map(session_vcs_git_status_entry)
            .collect(),
        entries_total_count: snapshot.entries_total_count,
        entries_truncated: snapshot.entries_truncated,
    }
}

fn session_vcs_git_status_entry(entry: GitStatusEntry) -> SessionVcsGitStatusEntry {
    SessionVcsGitStatusEntry {
        path: entry.path,
        orig_path: entry.orig_path,
        index_status: entry.index_status,
        worktree_status: entry.worktree_status,
    }
}
