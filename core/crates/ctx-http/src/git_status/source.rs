use std::sync::Arc;

use anyhow::Result;
use ctx_core::models::Worktree;
use ctx_workspace_config as workspace_config;
use ctx_workspace_services::worktree_vcs::{
    LocalWorktreeVcsSource, SandboxWorktreeVcsSource, WorktreeVcsCommitLookupSource,
    WorktreeVcsDiffBaseSource, WorktreeVcsDiffPathSource, WorktreeVcsStatusSource,
    WorktreeVcsStructuredStatus,
};

use crate::daemon::AppState;
use crate::settings::ExecutionMode;
use ctx_worktree_data_plane::resolve_worktree_data_plane_with_host as resolve_worktree_data_plane;

use super::sandbox::HttpSandboxWorktreeVcsExecutor;

pub(crate) struct HttpWorktreeVcsSource<'a> {
    state: &'a Arc<AppState>,
    worktree: &'a Worktree,
}

impl<'a> HttpWorktreeVcsSource<'a> {
    pub(crate) fn new(state: &'a Arc<AppState>, worktree: &'a Worktree) -> Self {
        Self { state, worktree }
    }
}

#[async_trait::async_trait]
impl WorktreeVcsStatusSource for HttpWorktreeVcsSource<'_> {
    async fn has_vcs_repo(&self) -> Result<bool> {
        let data_plane = resolve_worktree_data_plane(self.state.as_ref(), self.worktree).await?;
        if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
            let executor = HttpSandboxWorktreeVcsExecutor::new(self.state, self.worktree);
            return SandboxWorktreeVcsSource::new(&executor)
                .has_vcs_repo()
                .await;
        }

        LocalWorktreeVcsSource::new(self.worktree, data_plane.live_worktree_root.as_path())
            .has_vcs_repo()
            .await
    }

    async fn load_structured_status(
        &self,
        include_untracked_files: bool,
        include_entries: bool,
    ) -> Result<WorktreeVcsStructuredStatus> {
        let data_plane = resolve_worktree_data_plane(self.state.as_ref(), self.worktree).await?;
        let root = data_plane.live_worktree_root.as_path();
        let structured = if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
            let executor = HttpSandboxWorktreeVcsExecutor::new(self.state, self.worktree);
            SandboxWorktreeVcsSource::new(&executor)
                .load_structured_status(include_untracked_files, include_entries)
                .await?
        } else {
            LocalWorktreeVcsSource::new(self.worktree, root)
                .load_structured_status(include_untracked_files, include_entries)
                .await?
        };
        Ok(structured)
    }
}

#[async_trait::async_trait]
impl WorktreeVcsCommitLookupSource for HttpWorktreeVcsSource<'_> {
    async fn resolve_commit(&self, reference: &str) -> Result<String> {
        let data_plane = resolve_worktree_data_plane(self.state.as_ref(), self.worktree).await?;
        let root = data_plane.live_worktree_root.as_path();
        if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
            let executor = HttpSandboxWorktreeVcsExecutor::new(self.state, self.worktree);
            return SandboxWorktreeVcsSource::new(&executor)
                .resolve_commit(reference)
                .await;
        }
        LocalWorktreeVcsSource::new(self.worktree, root)
            .resolve_commit(reference)
            .await
    }
}

#[async_trait::async_trait]
impl WorktreeVcsDiffBaseSource for HttpWorktreeVcsSource<'_> {
    async fn load_primary_branch(&self) -> Result<Option<String>> {
        let store = self.state.store_for_worktree(self.worktree.id).await?;
        workspace_config::load_primary_branch(&store).await
    }

    async fn rev_parse_head(&self) -> Result<String> {
        self.resolve_commit("HEAD").await
    }

    async fn rev_parse_refs(&self, references: &[&str]) -> Result<Vec<String>> {
        if references.is_empty() {
            return Ok(Vec::new());
        }
        let data_plane = resolve_worktree_data_plane(self.state.as_ref(), self.worktree).await?;
        let root = data_plane.live_worktree_root.as_path();
        if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
            let executor = HttpSandboxWorktreeVcsExecutor::new(self.state, self.worktree);
            return SandboxWorktreeVcsSource::new(&executor)
                .rev_parse_refs(references)
                .await;
        }

        LocalWorktreeVcsSource::new(self.worktree, root)
            .rev_parse_refs(references)
            .await
    }

    async fn merge_base(&self, target_branch: &str) -> Result<String> {
        let data_plane = resolve_worktree_data_plane(self.state.as_ref(), self.worktree).await?;
        let root = data_plane.live_worktree_root.as_path();
        if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
            let executor = HttpSandboxWorktreeVcsExecutor::new(self.state, self.worktree);
            return SandboxWorktreeVcsSource::new(&executor)
                .merge_base(target_branch)
                .await;
        }
        LocalWorktreeVcsSource::new(self.worktree, root)
            .merge_base(target_branch)
            .await
    }

    fn redact_error(&self, err: &anyhow::Error) -> String {
        ctx_observability::logs::redact_sensitive(&err.to_string())
    }
}

#[async_trait::async_trait]
impl WorktreeVcsDiffPathSource for HttpWorktreeVcsSource<'_> {
    async fn diff_name_status(
        &self,
        base_commit_sha: &str,
        summary_count: bool,
    ) -> Result<Vec<(String, String, Option<String>)>> {
        let data_plane = resolve_worktree_data_plane(self.state.as_ref(), self.worktree).await?;
        let root = data_plane.live_worktree_root.as_path();
        if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
            let executor = HttpSandboxWorktreeVcsExecutor::new(self.state, self.worktree);
            return SandboxWorktreeVcsSource::new(&executor)
                .diff_name_status(base_commit_sha, summary_count)
                .await;
        }
        LocalWorktreeVcsSource::new(self.worktree, root)
            .diff_name_status(base_commit_sha, summary_count)
            .await
    }

    async fn list_untracked(&self) -> Result<Vec<String>> {
        let data_plane = resolve_worktree_data_plane(self.state.as_ref(), self.worktree).await?;
        let root = data_plane.live_worktree_root.as_path();
        if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
            let executor = HttpSandboxWorktreeVcsExecutor::new(self.state, self.worktree);
            return SandboxWorktreeVcsSource::new(&executor)
                .list_untracked()
                .await;
        }
        LocalWorktreeVcsSource::new(self.worktree, root)
            .list_untracked()
            .await
    }
}
