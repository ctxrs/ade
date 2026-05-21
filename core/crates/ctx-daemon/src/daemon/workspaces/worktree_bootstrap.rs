use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use ctx_core::ids::WorktreeId;
use ctx_core::models::{Workspace, Worktree};
use ctx_worktree_bootstrap_service::{
    BootstrapCommandResult, BootstrapConfig, BootstrapReport, BootstrapStep,
};

use crate::daemon::DaemonState;

mod command;
mod config;
mod report;

pub async fn spawn_worktree_bootstrap(
    state: Arc<DaemonState>,
    workspace: Workspace,
    worktree: Worktree,
) -> Result<()> {
    ctx_worktree_bootstrap_service::spawn_worktree_bootstrap(state, workspace, worktree).await
}

#[async_trait]
impl ctx_worktree_bootstrap_service::WorktreeBootstrapHost for DaemonState {
    async fn load_bootstrap_config(
        &self,
        workspace: &Workspace,
    ) -> Result<Option<BootstrapConfig>> {
        config::load_bootstrap_config(self, workspace).await
    }

    async fn execute_bootstrap_step(
        &self,
        workspace: &Workspace,
        worktree: &Worktree,
        step: &BootstrapStep,
        timeout: Duration,
    ) -> Result<BootstrapCommandResult> {
        command::run_bootstrap_step(self, step, workspace, worktree, timeout).await
    }

    async fn persist_bootstrap_report(
        &self,
        workspace_id: ctx_core::ids::WorkspaceId,
        worktree: &Worktree,
        report: BootstrapReport,
    ) {
        report::persist_bootstrap_report(self, workspace_id, worktree, report).await;
    }

    async fn register_bootstrap(&self, worktree_id: WorktreeId, wait_for_completion: bool) {
        self.register_worktree_bootstrap(worktree_id, wait_for_completion)
            .await;
    }

    async fn finish_bootstrap(&self, worktree_id: WorktreeId) {
        self.finish_worktree_bootstrap(worktree_id).await;
    }
}
