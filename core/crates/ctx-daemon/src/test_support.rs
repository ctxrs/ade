use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use ctx_core::ids::{
    SessionId, TaskId, TerminalId, WorkspaceAttachmentId, WorkspaceId, WorktreeId,
};
use ctx_core::models::{
    Session, SessionHeadDelta, WorkspaceAttachmentStatus, Worktree, WorktreeVcsSnapshot,
};
use ctx_provider_install::install_state::{
    InstallId, InstallInfo, InstallProgressEvent, InstallTarget,
};
use ctx_provider_runtime::{provider_usage, CachedProviderOptions, CachedProviderVerify};
use ctx_providers::adapters::{ProviderAdapter, ProviderStatus};
use ctx_storage_admission::StorageGuardStatus;
use ctx_store::{Store, StoreManager};
use tokio::sync::Mutex as AsyncMutex;

use crate::daemon::{self, AppRuntimeFlags, DaemonHandle, DaemonState};

#[derive(Clone)]
pub struct TestDaemon {
    state: Arc<DaemonState>,
}

impl TestDaemon {
    pub fn new(
        data_root: PathBuf,
        stores: StoreManager,
        providers: HashMap<String, Arc<dyn ProviderAdapter>>,
        daemon_url: String,
        auth_token: Option<String>,
    ) -> Self {
        Self::new_with_public_base_url(data_root, stores, providers, daemon_url, None, auth_token)
    }

    pub fn new_with_public_base_url(
        data_root: PathBuf,
        stores: StoreManager,
        providers: HashMap<String, Arc<dyn ProviderAdapter>>,
        daemon_url: String,
        public_base_url: Option<String>,
        auth_token: Option<String>,
    ) -> Self {
        Self::from_state(Arc::new(DaemonState::new_with_public_base_url(
            data_root,
            stores,
            providers,
            daemon_url,
            public_base_url,
            auth_token,
        )))
    }

    pub fn new_with_runtime_flags(
        data_root: PathBuf,
        stores: StoreManager,
        providers: HashMap<String, Arc<dyn ProviderAdapter>>,
        daemon_url: String,
        public_base_url: Option<String>,
        auth_token: Option<String>,
        runtime_flags: AppRuntimeFlags,
    ) -> Self {
        Self::from_state(Arc::new(DaemonState::new_with_runtime_flags(
            data_root,
            stores,
            providers,
            daemon_url,
            public_base_url,
            auth_token,
            runtime_flags,
        )))
    }

    pub fn from_state(state: Arc<DaemonState>) -> Self {
        Self { state }
    }

    pub fn handle(&self) -> DaemonHandle {
        DaemonHandle::new(Arc::clone(&self.state))
    }

    pub fn data_root(&self) -> &Path {
        &self.state.core.data_root
    }

    pub fn tool_output_spool_dir(&self) -> &Path {
        self.state.test_tool_output_spool_dir()
    }

    pub fn daemon_url(&self) -> &str {
        &self.state.core.daemon_url
    }

    pub fn global_store(&self) -> &Store {
        self.state.global_store()
    }

    pub fn stores(&self) -> &StoreManager {
        &self.state.core.stores
    }

    pub fn request_shutdown(&self) {
        let _ = self.state.core.shutdown_tx.send(());
    }

    pub async fn set_session_running(&self, session_id: SessionId, running: bool) {
        self.state.set_running(session_id, running).await;
    }

    pub async fn is_session_running(&self, session_id: SessionId) -> bool {
        self.state.is_session_running(session_id).await
    }

    pub async fn store_for_session(&self, session_id: SessionId) -> anyhow::Result<Store> {
        self.state.store_for_session(session_id).await
    }

    pub async fn store_for_workspace(&self, workspace_id: WorkspaceId) -> anyhow::Result<Store> {
        self.state.store_for_workspace(workspace_id).await
    }

    pub async fn uncached_store_for_workspace(
        &self,
        workspace_id: WorkspaceId,
    ) -> anyhow::Result<Store> {
        self.state
            .core
            .stores
            .workspace_uncached(workspace_id)
            .await
    }

    pub async fn store_for_task(&self, task_id: TaskId) -> anyhow::Result<Store> {
        self.state.store_for_task(task_id).await
    }

    pub async fn task_session_creation_lock(&self, task_id: TaskId) -> Arc<tokio::sync::Mutex<()>> {
        self.state.task_session_creation_lock(task_id).await
    }

    pub fn spawn_merge_queue_runner(&self) {
        daemon::merge_queue::spawn_merge_queue_runner(Arc::clone(&self.state));
    }

    pub async fn ensure_workspace_active_snapshot_hydrated(
        &self,
        workspace_id: WorkspaceId,
    ) -> std::result::Result<(), daemon::workspaces::WorkspaceHydrationError> {
        self.state
            .ensure_workspace_active_snapshot_hydrated(workspace_id)
            .await
    }

    pub async fn mark_worktree_vcs_active_for_test(&self, worktree_id: WorktreeId) {
        let mut next_active = std::collections::HashSet::new();
        next_active.insert(worktree_id);
        self.state
            .test_update_worktree_vcs_activity(&std::collections::HashSet::new(), &next_active)
            .await;
    }

    pub async fn emit_worktree_vcs_snapshot_for_worktree(
        &self,
        worktree: &Worktree,
        include_commit_info: bool,
    ) -> anyhow::Result<()> {
        daemon::git_status::emit_worktree_vcs_snapshot_for_worktree(
            &self.state,
            worktree,
            include_commit_info,
        )
        .await
    }

    pub async fn load_worktree_for_test(
        &self,
        worktree_id: WorktreeId,
    ) -> anyhow::Result<Worktree> {
        self.state
            .store_for_worktree(worktree_id)
            .await?
            .get_worktree(worktree_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("worktree {worktree_id:?} not found"))
    }

    pub async fn set_workspace_attachment_status_for_test(
        &self,
        workspace_id: WorkspaceId,
        attachment_id: WorkspaceAttachmentId,
        status: WorkspaceAttachmentStatus,
    ) -> anyhow::Result<()> {
        self.state
            .store_for_workspace(workspace_id)
            .await?
            .update_workspace_attachment_status(
                attachment_id,
                status,
                None,
                None,
                chrono::Utc::now(),
            )
            .await
    }

    pub async fn worktree_vcs_snapshot(
        &self,
        worktree_id: WorktreeId,
    ) -> Option<WorktreeVcsSnapshot> {
        self.state.get_worktree_vcs_snapshot(worktree_id).await
    }

    pub async fn terminal_output_snapshot(&self, terminal_id: TerminalId) -> Option<Vec<u8>> {
        self.state
            .test_terminal_handle(terminal_id)
            .await
            .map(|handle| handle.output_snapshot())
    }

    pub async fn remember_session_meta(&self, session: &Session) {
        self.state.sessions.remember_session_meta(session).await;
    }

    pub async fn publish_session_head_delta(
        &self,
        session: &Session,
        delta: SessionHeadDelta,
        bump_snapshot: bool,
    ) {
        self.state
            .test_publish_session_head_delta(session, delta, bump_snapshot)
            .await;
    }

    pub async fn set_provider_inactivity_timeout(&self, timeout: Duration) {
        self.state
            .test_set_provider_inactivity_timeout(timeout)
            .await;
    }

    pub async fn replace_provider_statuses(&self, statuses: HashMap<String, ProviderStatus>) {
        self.state
            .providers
            .replace_provider_statuses(statuses)
            .await;
    }

    pub async fn refresh_provider_statuses(&self) -> anyhow::Result<()> {
        ctx_managed_installs::refresh_provider_statuses(self.state.as_ref()).await
    }

    pub async fn upsert_provider_status(&self, provider_id: String, status: ProviderStatus) {
        self.state
            .providers
            .upsert_provider_status(provider_id, status)
            .await;
    }

    pub fn publish_storage_guard(&self, status: StorageGuardStatus) {
        self.state.test_publish_storage_guard(status);
    }

    pub async fn stop_mobile_tunnel(&self) {
        self.state.test_stop_mobile_tunnel().await;
    }

    pub async fn issue_provider_session_mcp_token(
        &self,
        session_id: SessionId,
        workspace_id: WorkspaceId,
        worktree_id: WorktreeId,
    ) -> String {
        daemon::issue_provider_session_mcp_token(&self.state, session_id, workspace_id, worktree_id)
            .await
    }

    pub async fn issue_provider_session_mcp_token_with_capabilities(
        &self,
        session_id: SessionId,
        workspace_id: WorkspaceId,
        worktree_id: WorktreeId,
        capabilities: ctx_mcp_auth::McpAuthCapabilities,
    ) -> String {
        daemon::issue_provider_session_mcp_token_with_capabilities(
            &self.state,
            session_id,
            workspace_id,
            worktree_id,
            capabilities,
        )
        .await
    }

    pub async fn revoke_provider_session_mcp_token(&self, token: &str) -> bool {
        daemon::revoke_provider_session_mcp_token(&self.state, token).await
    }

    pub async fn test_with_provider_usage_cache<R>(
        &self,
        f: impl FnOnce(&mut HashMap<String, provider_usage::ProviderUsageSnapshot>) -> R,
    ) -> R {
        self.state.test_with_provider_usage_cache(f).await
    }

    pub async fn test_with_provider_options_cache<R>(
        &self,
        f: impl FnOnce(&mut HashMap<String, CachedProviderOptions>) -> R,
    ) -> R {
        self.state.test_with_provider_options_cache(f).await
    }

    pub async fn test_with_provider_verify_cache<R>(
        &self,
        f: impl FnOnce(&mut HashMap<String, CachedProviderVerify>) -> R,
    ) -> R {
        self.state.test_with_provider_verify_cache(f).await
    }

    pub async fn start_install(
        &self,
        provider_id: String,
        target: Option<InstallTarget>,
    ) -> (InstallId, bool) {
        self.state.start_install(provider_id, target).await
    }

    pub async fn find_running_install(
        &self,
        provider_id: &str,
        target: Option<InstallTarget>,
    ) -> Option<InstallId> {
        self.state.find_running_install(provider_id, target).await
    }

    pub async fn install_provider_with_progress(
        &self,
        install_id: InstallId,
        provider_id: String,
        target: InstallTarget,
    ) -> anyhow::Result<()> {
        let state: Arc<ctx_managed_installs::AppState> = self.state.clone();
        ctx_managed_installs::install_provider_with_progress(state, install_id, provider_id, target)
            .await
    }

    pub async fn install_title_generation_local_with_progress(
        &self,
        install_id: InstallId,
    ) -> anyhow::Result<()> {
        let state: Arc<ctx_managed_installs::AppState> = self.state.clone();
        ctx_managed_installs::install_title_generation_local_with_progress(state, install_id).await
    }

    pub async fn emit_install_event(&self, install_id: InstallId, event: InstallProgressEvent) {
        self.state.emit_install_event(install_id, event).await;
    }

    pub async fn get_install_info(&self, install_id: InstallId) -> Option<InstallInfo> {
        self.state.get_install_info(install_id).await
    }

    pub async fn tracked_install_ids(
        &self,
        provider_id: &str,
        target: Option<InstallTarget>,
    ) -> Vec<InstallId> {
        self.state
            .test_tracked_install_ids(provider_id, target)
            .await
    }

    pub async fn has_target_provider_adapter(&self, cache_key: &str) -> bool {
        self.state.test_has_target_provider_adapter(cache_key).await
    }

    pub async fn target_provider_adapter_cache_keys(&self) -> Vec<String> {
        self.state
            .test_target_provider_adapter_entries()
            .await
            .into_iter()
            .map(|(cache_key, _)| cache_key)
            .collect()
    }

    pub async fn get_install_polling_info(&self, install_id: InstallId) -> Option<InstallInfo> {
        self.state.get_install_polling_info(install_id).await
    }

    pub async fn get_install_events(
        &self,
        install_id: InstallId,
    ) -> Option<Vec<InstallProgressEvent>> {
        self.state.get_install_events(install_id).await
    }

    pub async fn provider_login_session_caches_empty(&self) -> bool {
        let gemini = self
            .state
            .test_with_gemini_login_sessions(|map| map.is_empty())
            .await;
        let qwen = self
            .state
            .test_with_qwen_login_sessions(|map| map.is_empty())
            .await;
        let amp = self
            .state
            .test_with_amp_login_sessions(|map| map.is_empty())
            .await;
        let mistral = self
            .state
            .test_with_mistral_login_sessions(|map| map.is_empty())
            .await;
        let kimi = self
            .state
            .test_with_kimi_login_sessions(|map| map.is_empty())
            .await;
        let claude = self
            .state
            .test_with_claude_login_sessions(|map| map.is_empty())
            .await;
        let codex = self
            .state
            .test_with_codex_login_sessions(|map| map.is_empty())
            .await;
        let cursor = self
            .state
            .test_with_cursor_login_sessions(|map| map.is_empty())
            .await;
        gemini && qwen && amp && mistral && kimi && claude && codex && cursor
    }
}

/// Workspace-runtime tests historically used a sandbox-specific name for the
/// shared sandbox-runtime lock. Keep that lock separate from the broader
/// process-env lock so long-lived runtime jobs are not queued behind unrelated
/// bundle/env tests.
pub fn sandbox_cli_env_test_lock() -> &'static AsyncMutex<()> {
    static LOCK: OnceLock<AsyncMutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| AsyncMutex::new(()))
}

#[cfg(unix)]
pub fn write_running_container_sandbox_cli_shim(
    dir: &Path,
    log_path: &Path,
    container_name: &str,
) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = dir.join("sandbox-cli-running-container-test.sh");
    std::fs::write(
        &path,
        format!(
            "#!/bin/sh\nLOG=\"{log}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"info\" ]; then\n  printf '{{}}\\n'\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"start\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"init\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"image\" ] && [ \"$2\" = \"inspect\" ]; then\n  echo 'transient image store failure' >&2\n  exit 125\nfi\nif [ \"$1\" = \"volume\" ] && [ \"$2\" = \"inspect\" ]; then\n  exit 1\nfi\nif [ \"$1\" = \"volume\" ] && [ \"$2\" = \"create\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"inspect\" ] && [ \"$2\" = \"{container}\" ]; then\n  suffix=${{2#ctx-harness-}}\n  printf '[{{\"Mounts\":[{{\"Type\":\"volume\",\"Name\":\"ctx-ws-%s\",\"Destination\":\"/ctx/ws\"}}]}}]\\n' \"$suffix\"\n  exit 0\nfi\nif [ \"$1\" = \"container\" ] && [ \"$2\" = \"inspect\" ] && [ \"$3\" = \"{container}\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"container\" ] && [ \"$2\" = \"inspect\" ] && [ \"$5\" = \"{container}\" ]; then\n  printf 'true\\n'\n  exit 0\nfi\nif [ \"$1\" = \"exec\" ]; then\n  shift\n  while [ \"$#\" -gt 0 ]; do\n    case \"$1\" in\n      --interactive)\n        shift\n        ;;\n      --user|--workdir|--env)\n        shift 2\n        ;;\n      *)\n        break\n        ;;\n    esac\n  done\n  container_name=\"$1\"\n  shift\n  command=\"$1\"\n  shift\n  if [ \"$container_name\" != \"{container}\" ]; then\n    echo \"unexpected container: $container_name\" >&2\n    exit 1\n  fi\n  if [ \"$command\" = \"tar\" ] && [ \"$1\" = \"-xf\" ] && [ \"$2\" = \"-\" ]; then\n    cat >/dev/null\n    exit 0\n  fi\n  if [ \"$command\" = \"git\" ] && [ \"$1\" = \"checkout\" ]; then\n    exit 0\n  fi\n  if [ \"$command\" = \"id\" ] && [ \"$1\" = \"-u\" ]; then\n    printf '1000\\n'\n    exit 0\n  fi\n  if [ \"$command\" = \"id\" ] && [ \"$1\" = \"-g\" ]; then\n    printf '1000\\n'\n    exit 0\n  fi\n  if [ \"$command\" = \"df\" ] && [ \"$1\" = \"-Pk\" ]; then\n    printf 'Filesystem 1024-blocks Used Available Capacity Mounted on\\n'\n    printf 'overlay 10485760 1024 7340032 1%% /ctx/ws\\n'\n    exit 0\n  fi\n  if [ \"$command\" = \"sh\" ] && [ \"$1\" = \"-lc\" ]; then\n    case \"$2\" in\n      *\"git rev-parse --is-inside-work-tree\"*)\n        printf 'true\\n'\n        exit 0\n        ;;\n      *)\n        exit 0\n        ;;\n    esac\n  fi\n  exit 0\nfi\necho \"unexpected sandbox CLI invocation: $*\" >&2\nexit 1\n",
            log = log_path.display(),
            container = container_name,
        ),
    )
    .expect("write running-container sandbox CLI shim");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod running-container sandbox CLI shim");
    path
}

pub fn avf_linux_runtime_manager_test_sandbox_cli_path(dir: &Path) -> PathBuf {
    dir.join("ctx-avf-linux-sandbox-cli-runtime-manager-test.sh")
}
