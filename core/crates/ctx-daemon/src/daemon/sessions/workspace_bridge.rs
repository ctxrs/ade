use anyhow::Result;
use ctx_core::ids::{SessionId, TaskId, WorkspaceId, WorktreeId};
use ctx_core::models::{
    Session, SessionGitStatusSummary, Workspace, Worktree, WorktreeVcsSnapshot,
};
use ctx_store::Store;
use ctx_workspace_services::worktree_vcs::{
    resolve_worktree_diff_base_from_source, GitStatusSnapshot, WorktreeDiffBaseResolution,
    WorktreeVcsCommitLookupSource, WorktreeVcsDiffBaseQuery, WorktreeVcsDiffSummaryCounts,
};

use crate::daemon::git_status::{
    load_git_status_snapshot, worktree_has_vcs_repo, HttpWorktreeVcsSource,
};
use crate::daemon::handle::SessionsHandle;
use crate::daemon::workspaces::{
    complete_files_for_session, diff_worktree_for_session, diff_worktree_summary_for_session,
    resolve_existing_worktree_execution, update_workspace_provider_preferred_model_id,
    FileCompletionsError, ResolvedExistingWorktreeExecution,
};

impl SessionsHandle {
    pub async fn load_session_vcs_parts(
        &self,
        session_id: SessionId,
    ) -> Result<Option<(Session, Worktree)>> {
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

    pub async fn persist_session_git_status_summary(
        &self,
        session_id: SessionId,
        worktree_id: WorktreeId,
        summary: &SessionGitStatusSummary,
    ) -> Result<()> {
        let store = self.store_for_session(session_id).await?;
        store
            .upsert_session_git_status_summary(session_id, worktree_id, summary)
            .await
    }

    pub async fn complete_files_for_session(
        &self,
        session_id: SessionId,
        query: Option<String>,
        limit: Option<u32>,
    ) -> Result<Vec<String>, FileCompletionsError> {
        complete_files_for_session(&self.state, session_id, query, limit).await
    }

    pub async fn resolve_existing_worktree_execution(
        &self,
        store: &Store,
        workspace: &Workspace,
        worktree_id: WorktreeId,
    ) -> anyhow::Result<ResolvedExistingWorktreeExecution> {
        resolve_existing_worktree_execution(&self.state, store, workspace, worktree_id).await
    }

    pub async fn update_workspace_provider_preferred_model_id(
        &self,
        workspace_id: WorkspaceId,
        provider_id: &str,
        preferred_model_id: Option<String>,
    ) -> anyhow::Result<()> {
        update_workspace_provider_preferred_model_id(
            &self.state,
            workspace_id,
            provider_id,
            preferred_model_id,
        )
        .await
    }

    pub async fn emit_workspace_task_upsert(&self, task_id: TaskId) -> anyhow::Result<()> {
        self.state.emit_workspace_task_upsert(task_id).await
    }

    pub async fn get_worktree_vcs_snapshot(
        &self,
        worktree_id: WorktreeId,
    ) -> Option<WorktreeVcsSnapshot> {
        self.state.get_worktree_vcs_snapshot(worktree_id).await
    }

    pub async fn worktree_has_vcs_repo(&self, worktree: &Worktree) -> anyhow::Result<bool> {
        worktree_has_vcs_repo(&self.state, worktree).await
    }

    pub async fn load_git_status_snapshot(
        &self,
        worktree: &Worktree,
        include_untracked_files: bool,
        include_entries: bool,
    ) -> anyhow::Result<GitStatusSnapshot> {
        load_git_status_snapshot(
            &self.state,
            worktree,
            include_untracked_files,
            include_entries,
        )
        .await
    }

    pub async fn resolve_worktree_commit(
        &self,
        worktree: &Worktree,
        revision: &str,
    ) -> anyhow::Result<String> {
        let source = HttpWorktreeVcsSource::new(&self.state, worktree);
        source.resolve_commit(revision).await
    }

    pub async fn diff_worktree_for_session(
        &self,
        worktree: &Worktree,
        base_commit_sha: &str,
    ) -> anyhow::Result<String> {
        diff_worktree_for_session(&self.state, worktree, base_commit_sha).await
    }

    pub async fn diff_worktree_summary_for_session(
        &self,
        worktree: &Worktree,
        base_commit_sha: &str,
    ) -> anyhow::Result<WorktreeVcsDiffSummaryCounts> {
        diff_worktree_summary_for_session(&self.state, worktree, base_commit_sha).await
    }

    pub async fn resolve_worktree_diff_base(
        &self,
        worktree: &Worktree,
        query: WorktreeVcsDiffBaseQuery,
    ) -> WorktreeDiffBaseResolution {
        let source = HttpWorktreeVcsSource::new(&self.state, worktree);
        resolve_worktree_diff_base_from_source(&source, worktree, query).await
    }
}
