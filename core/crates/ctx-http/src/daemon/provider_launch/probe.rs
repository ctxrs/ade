use std::path::Path;

use anyhow::Result;
use async_trait::async_trait;
use ctx_core::models::{Workspace, Worktree};

use crate::daemon::AppState;
use ctx_observability::logs;

mod runtime;

#[allow(unused_imports)]
pub(crate) use ctx_provider_runtime::provider_launch::probe::{
    provider_auth_context_for_workspace_runtime, provider_auth_context_for_worktree_runtime,
    provider_has_active_auth_for_workspace_runtime, provider_probe_context_for_workspace_runtime,
    provider_probe_env, provider_probe_env_for_workspace_runtime, PreparedWorkspaceProbeRuntime,
    WorkspaceRuntimeProbeContext,
};

#[async_trait]
impl ctx_provider_runtime::provider_launch::probe::ProviderProbeHost for AppState {
    fn data_root(&self) -> &Path {
        &self.core.data_root
    }

    fn daemon_url(&self) -> &str {
        &self.core.daemon_url
    }

    fn auth_token(&self) -> Option<&String> {
        self.core.auth_token.as_ref()
    }

    fn redact_sensitive(&self, input: &str) -> String {
        logs::redact_sensitive(input)
    }

    async fn load_workspace(
        &self,
        workspace_id: ctx_core::ids::WorkspaceId,
    ) -> Result<Option<Workspace>, String> {
        self.global_store()
            .get_workspace(workspace_id)
            .await
            .map_err(|err| logs::redact_sensitive(&format!("loading workspace failed: {err:#}")))
    }

    async fn prepare_workspace_probe_runtime(
        &self,
        workspace: &Workspace,
    ) -> Result<PreparedWorkspaceProbeRuntime, String> {
        runtime::prepare_workspace_probe_runtime(self, workspace).await
    }

    async fn prepare_worktree_probe_runtime(
        &self,
        workspace: &Workspace,
        worktree: &Worktree,
    ) -> Result<PreparedWorkspaceProbeRuntime, String> {
        runtime::prepare_worktree_probe_runtime(self, workspace, worktree).await
    }
}
