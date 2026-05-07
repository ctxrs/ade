use std::sync::Arc;

use anyhow::Result;
use ctx_core::models::Worktree;
use ctx_workspace_config as workspace_config;
use ctx_workspace_services::worktree_vcs::{
    is_no_vcs_repo_error, LocalWorktreeVcsSource, WorktreeVcsCommitLookupSource,
    WorktreeVcsDiffBaseSource, WorktreeVcsGitCommand, WorktreeVcsStatusSource,
    WorktreeVcsStructuredStatus,
};

use crate::daemon::AppState;
use crate::settings::ExecutionMode;
use crate::worktree_data_plane::resolve_worktree_data_plane;

use super::sandbox::{container_git_status_structured, container_git_stdout};

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
        let data_plane = resolve_worktree_data_plane(self.state, self.worktree).await?;
        if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
            return match container_git_stdout(
                self.state,
                self.worktree,
                WorktreeVcsGitCommand::IsInsideWorkTree,
            )
            .await
            {
                Ok(_) => Ok(true),
                Err(err) if is_no_vcs_repo_error(&err) => Ok(false),
                Err(err) => Err(err),
            };
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
        let data_plane = resolve_worktree_data_plane(self.state, self.worktree).await?;
        let root = data_plane.live_worktree_root.as_path();
        let structured = if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
            container_git_status_structured(
                self.state,
                self.worktree,
                include_untracked_files,
                include_entries,
            )
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
        let data_plane = resolve_worktree_data_plane(self.state, self.worktree).await?;
        let root = data_plane.live_worktree_root.as_path();
        if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
            let bytes = container_git_stdout(
                self.state,
                self.worktree,
                WorktreeVcsGitCommand::RevParse {
                    reference: reference.to_string(),
                },
            )
            .await?;
            return Ok(ctx_workspace_services::worktree_vcs::parse_git_single_ref(
                &bytes,
            ));
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
        let data_plane = resolve_worktree_data_plane(self.state, self.worktree).await?;
        let root = data_plane.live_worktree_root.as_path();
        if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
            let bytes = container_git_stdout(
                self.state,
                self.worktree,
                WorktreeVcsGitCommand::RevParseRefs {
                    references: references
                        .iter()
                        .map(|reference| (*reference).to_string())
                        .collect(),
                },
            )
            .await?;
            return ctx_workspace_services::worktree_vcs::parse_git_refs(&bytes, references.len());
        }

        LocalWorktreeVcsSource::new(self.worktree, root)
            .rev_parse_refs(references)
            .await
    }

    async fn merge_base(&self, target_branch: &str) -> Result<String> {
        let data_plane = resolve_worktree_data_plane(self.state, self.worktree).await?;
        let root = data_plane.live_worktree_root.as_path();
        if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
            let bytes = container_git_stdout(
                self.state,
                self.worktree,
                WorktreeVcsGitCommand::MergeBase {
                    target_branch: target_branch.to_string(),
                },
            )
            .await?;
            return Ok(ctx_workspace_services::worktree_vcs::parse_git_single_ref(
                &bytes,
            ));
        }
        LocalWorktreeVcsSource::new(self.worktree, root)
            .merge_base(target_branch)
            .await
    }

    fn redact_error(&self, err: &anyhow::Error) -> String {
        crate::logs::redact_sensitive(&err.to_string())
    }
}
