use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use ctx_core::ids::{TerminalId, WorkspaceId, WorktreeId};
use ctx_core::models::{
    Session, SessionHeadDelta, SessionHeadSnapshot, Workspace, Worktree, WorktreeVcsSnapshot,
};
use ctx_harness_runtime::HarnessExecutionPlan;
use ctx_provider_accounts as provider_accounts;
use ctx_provider_install::install_state::{InstallId, InstallTarget};
use ctx_provider_runtime::{provider_usage, CachedProviderOptions, CachedProviderVerify};
use ctx_providers::adapters::{ProviderAdapter, ProviderStatus};
use ctx_settings_model::ExecutionSettings;
use ctx_storage_admission::StorageGuardStatus;
use ctx_store::StoreManager;
use ctx_transport_runtime::terminals::TerminalSessionHandle;
use ctx_workspace_active_snapshot::WorkspaceActiveSnapshotHub;
use ctx_workspace_container::WorkspaceContainerStatus;

use crate::daemon::DaemonState;

impl DaemonState {
    pub fn test_data_root(&self) -> &Path {
        &self.core.data_root
    }

    pub fn test_daemon_url(&self) -> &str {
        &self.core.daemon_url
    }

    pub fn test_tool_output_spool_dir(&self) -> &Path {
        &self.core.tool_output_spool_dir
    }

    pub fn test_store_manager(&self) -> &StoreManager {
        &self.core.stores
    }

    pub fn test_set_local_shutdown_token(&mut self, token: Option<String>) {
        self.core.local_shutdown_token = token;
    }

    pub fn test_request_shutdown(&self) {
        let _ = self.core.shutdown_tx.send(());
    }

    pub fn test_publish_storage_guard(&self, status: StorageGuardStatus) {
        self.core.storage_guard.publish(status);
    }

    pub async fn test_replace_provider_statuses(&self, statuses: HashMap<String, ProviderStatus>) {
        self.providers.replace_provider_statuses(statuses).await;
    }

    pub async fn test_upsert_provider_status(&self, provider_id: String, status: ProviderStatus) {
        self.providers
            .upsert_provider_status(provider_id, status)
            .await;
    }

    pub async fn test_with_provider_options_cache<R>(
        &self,
        f: impl FnOnce(&mut HashMap<String, CachedProviderOptions>) -> R,
    ) -> R {
        self.providers.with_provider_options_cache(f).await
    }

    pub async fn test_with_provider_verify_cache<R>(
        &self,
        f: impl FnOnce(&mut HashMap<String, CachedProviderVerify>) -> R,
    ) -> R {
        self.providers.with_provider_verify_cache(f).await
    }

    pub async fn test_with_provider_usage_cache<R>(
        &self,
        f: impl FnOnce(&mut HashMap<String, provider_usage::ProviderUsageSnapshot>) -> R,
    ) -> R {
        self.providers.with_provider_usage_cache(f).await
    }

    pub async fn test_has_target_provider_adapter(&self, cache_key: &str) -> bool {
        self.providers.has_target_provider_adapter(cache_key).await
    }

    pub async fn test_target_provider_adapter_entries(
        &self,
    ) -> Vec<(String, Arc<dyn ProviderAdapter>)> {
        self.providers.target_provider_adapter_entries().await
    }

    pub async fn test_tracked_install_ids(
        &self,
        provider_id: &str,
        target: Option<InstallTarget>,
    ) -> Vec<InstallId> {
        self.providers
            .tracked_install_ids(provider_id, target)
            .await
    }

    pub async fn test_with_codex_login_sessions<R>(
        &self,
        f: impl FnOnce(&mut HashMap<String, provider_accounts::CodexLoginStatus>) -> R,
    ) -> R {
        self.providers.with_codex_login_sessions(f).await
    }

    pub async fn test_with_claude_login_sessions<R>(
        &self,
        f: impl FnOnce(&mut HashMap<String, provider_accounts::ClaudeLoginStatus>) -> R,
    ) -> R {
        self.providers.with_claude_login_sessions(f).await
    }

    pub async fn test_with_gemini_login_sessions<R>(
        &self,
        f: impl FnOnce(&mut HashMap<String, provider_accounts::GeminiLoginStatus>) -> R,
    ) -> R {
        self.providers.with_gemini_login_sessions(f).await
    }

    pub async fn test_with_qwen_login_sessions<R>(
        &self,
        f: impl FnOnce(&mut HashMap<String, provider_accounts::QwenLoginStatus>) -> R,
    ) -> R {
        self.providers.with_qwen_login_sessions(f).await
    }

    pub async fn test_with_kimi_login_sessions<R>(
        &self,
        f: impl FnOnce(&mut HashMap<String, provider_accounts::KimiLoginStatus>) -> R,
    ) -> R {
        self.providers.with_kimi_login_sessions(f).await
    }

    pub async fn test_with_cursor_login_sessions<R>(
        &self,
        f: impl FnOnce(&mut HashMap<String, provider_accounts::CursorLoginStatus>) -> R,
    ) -> R {
        self.providers.with_cursor_login_sessions(f).await
    }

    pub async fn test_with_amp_login_sessions<R>(
        &self,
        f: impl FnOnce(&mut HashMap<String, provider_accounts::AmpLoginStatus>) -> R,
    ) -> R {
        self.providers.with_amp_login_sessions(f).await
    }

    pub async fn test_with_mistral_login_sessions<R>(
        &self,
        f: impl FnOnce(&mut HashMap<String, provider_accounts::MistralLoginStatus>) -> R,
    ) -> R {
        self.providers.with_mistral_login_sessions(f).await
    }

    pub async fn test_stop_mobile_tunnel(&self) {
        self.transport.mobile_tunnel.stop().await;
    }

    pub async fn test_terminal_handle(
        &self,
        terminal_id: TerminalId,
    ) -> Option<Arc<TerminalSessionHandle>> {
        self.transport.terminals.get(terminal_id).await
    }

    pub async fn test_prepare_harness(
        &self,
        workspace: &Workspace,
        worktree: &Worktree,
        execution_settings: &ExecutionSettings,
    ) -> anyhow::Result<HarnessExecutionPlan> {
        self.execution
            .harness
            .prepare(
                workspace,
                worktree,
                execution_settings,
                &self.core.daemon_url,
            )
            .await
    }

    pub async fn test_harness_container_status(
        &self,
        workspace_id: WorkspaceId,
    ) -> anyhow::Result<Option<WorkspaceContainerStatus>> {
        self.execution.harness.container_status(workspace_id).await
    }

    pub async fn test_worktree_has_git_status_watcher(&self, worktree_id: WorktreeId) -> bool {
        self.workspaces
            .git_status_watchers
            .lock()
            .await
            .contains(&worktree_id)
    }

    pub fn test_workspace_active_snapshot_hub(&self) -> Arc<WorkspaceActiveSnapshotHub> {
        Arc::clone(&self.workspaces.workspace_active_snapshot)
    }

    pub async fn test_publish_session_head_delta(
        &self,
        session: &Session,
        delta: SessionHeadDelta,
        bump_snapshot: bool,
    ) {
        self.test_publish_session_head_delta_for_workspace(
            session.workspace_id,
            session,
            delta,
            bump_snapshot,
        )
        .await;
    }

    pub async fn test_publish_session_head_delta_for_workspace(
        &self,
        workspace_id: WorkspaceId,
        session: &Session,
        delta: SessionHeadDelta,
        bump_snapshot: bool,
    ) {
        self.workspaces
            .workspace_active_snapshot
            .publish_session_head_delta(workspace_id, session, delta, bump_snapshot)
            .await;
    }

    pub async fn test_update_session_head(&self, head: SessionHeadSnapshot) {
        self.workspaces
            .workspace_active_snapshot
            .update_session_head(head)
            .await;
    }

    pub async fn test_update_worktree_vcs_activity(
        &self,
        previous: &std::collections::HashSet<WorktreeId>,
        next: &std::collections::HashSet<WorktreeId>,
    ) {
        self.workspaces
            .update_worktree_vcs_activity(previous, next)
            .await;
    }

    pub async fn test_cache_worktree_vcs_snapshot(&self, snapshot: WorktreeVcsSnapshot) {
        self.workspaces.cache_worktree_vcs_snapshot(snapshot).await;
    }

    pub async fn test_set_provider_inactivity_timeout(&self, timeout: Duration) {
        self.sessions.set_provider_inactivity_timeout(timeout).await;
    }
}
