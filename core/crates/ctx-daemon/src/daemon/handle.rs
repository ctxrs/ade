use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use ctx_core::ids::{SessionId, WorkspaceId};
use ctx_execution_runtime::ExecutionSetupCoordinator;
use ctx_mcp_auth::McpAuthRegistry;
use ctx_merge_queue::MergeQueueRuntime;
use ctx_observability::ops_events::{OpsEvent, OpsEvents};
use ctx_observability::perf_telemetry::PerfTelemetry;
use ctx_observability::telemetry::Telemetry;
use ctx_provider_install::install_state::{
    InstallId, InstallInfo, InstallProgressEvent, InstallTarget,
};
use ctx_provider_runtime::ProviderRuntime;
use ctx_resource_utilization::resource_governance::ResourceGovernanceRuntime;
use ctx_resource_utilization::ResourceSampler;
use ctx_session_runtime::runtime::SessionRuntime;
use ctx_storage_admission::{StorageGuardRuntime, StorageGuardStatus};
use ctx_store::manager::WorkspaceStoreAccessOutcome;
use ctx_store::{Store, StoreManager};
use ctx_transport_runtime::mobile_tunnel::MobileTunnelManager;
use ctx_transport_runtime::terminals::TerminalManager;
use ctx_update_service::UpdateDrainCoordinator;
use ctx_workspace_runtime::HarnessRuntimeManager;
use tokio::sync::{broadcast, Mutex};

use super::{
    blobs::BlobHandle,
    state::{DaemonState, TelemetryRuntime},
};

#[derive(Clone)]
pub struct DaemonHandle {
    state: Arc<DaemonState>,
}

impl DaemonHandle {
    pub fn new(state: Arc<DaemonState>) -> Self {
        Self { state }
    }

    pub fn subscribe_shutdown(&self) -> broadcast::Receiver<()> {
        self.state.core.shutdown_tx.subscribe()
    }

    pub fn auth(&self) -> AuthHandle {
        AuthHandle::new(
            self.state.core.auth_token.clone(),
            Arc::clone(&self.state.core.mcp_auth),
            self.state.global_store().clone(),
            self.state.telemetry.ops_events.clone(),
        )
    }

    pub fn health(&self) -> HealthHandle {
        HealthHandle::new(
            self.state.core.data_root.clone(),
            self.state.core.daemon_url.clone(),
            self.state.core.auth_token.clone(),
            Arc::clone(&self.state.core.storage_guard),
        )
    }

    pub fn diagnostics(&self) -> DiagnosticsHandle {
        DiagnosticsHandle::new(
            self.health(),
            self.state.core.data_root.clone(),
            Arc::clone(&self.state.execution.setup),
            Arc::clone(&self.state.providers),
        )
    }

    pub fn blob(&self) -> BlobHandle {
        BlobHandle::new(
            self.state.core.data_root.clone(),
            self.state.global_store().clone(),
        )
    }

    pub fn request_base(&self) -> RequestBaseHandle {
        RequestBaseHandle::new(
            self.state.core.daemon_url.clone(),
            self.state.core.public_base_url.clone(),
        )
    }

    pub fn logs(&self) -> LogsHandle {
        LogsHandle::new(self.state.core.data_root.clone())
    }

    pub fn org_policy(&self) -> OrgPolicyHandle {
        OrgPolicyHandle::new(self.state.global_store().clone())
    }

    pub fn dictation(&self) -> DictationHandle {
        DictationHandle::new(self.state.global_store().clone())
    }

    pub fn update_release(&self) -> UpdateReleaseHandle {
        UpdateReleaseHandle::new(self.state.core.data_root.clone())
    }

    pub fn update_activity(&self) -> UpdateActivityHandle {
        UpdateActivityHandle::new(
            self.state.global_store().clone(),
            self.state.core.stores.clone(),
            Arc::clone(&self.state.core.update_drain),
            self.state.core.data_root.clone(),
        )
    }

    pub fn settings(&self) -> SettingsHandle {
        SettingsHandle::new(
            self.state.global_store().clone(),
            self.state.telemetry.telemetry.clone(),
            self.state.telemetry.perf_telemetry.clone(),
            Arc::clone(&self.state.telemetry.resource_sampler),
            Arc::clone(&self.state.telemetry.resource_governance),
            Arc::clone(&self.state.providers),
            Arc::clone(&self.state.transport.terminals),
        )
    }

    pub fn mobile_store(&self) -> MobileStoreHandle {
        MobileStoreHandle::new(self.state.global_store().clone())
    }

    pub fn mobile_runtime(&self) -> MobileRuntimeHandle {
        MobileRuntimeHandle::new(
            self.state.global_store().clone(),
            self.state.transport.mobile_tunnel.clone(),
            self.state.core.daemon_url.clone(),
            self.state.core.auth_token.is_some(),
        )
    }

    pub fn mobile_secure_proxy(&self) -> MobileSecureProxyHandle {
        MobileSecureProxyHandle::new(
            self.state.global_store().clone(),
            self.health(),
            self.state.telemetry.telemetry.clone(),
        )
    }

    pub fn sessions(&self) -> SessionsHandle {
        SessionsHandle::new(Arc::clone(&self.state))
    }

    pub fn tasks(&self) -> TasksHandle {
        TasksHandle::new(Arc::clone(&self.state))
    }

    pub fn workspaces(&self) -> WorkspacesHandle {
        WorkspacesHandle::new(Arc::clone(&self.state))
    }

    pub fn workspace_stream(&self) -> WorkspaceStreamHandle {
        WorkspaceStreamHandle::new(Arc::clone(&self.state))
    }

    pub fn providers(&self) -> ProvidersHandle {
        ProvidersHandle::new(Arc::clone(&self.state))
    }

    pub fn provider_accounts(&self) -> ProviderAccountsHandle {
        ProviderAccountsHandle::new(
            self.state.core.data_root.clone(),
            Arc::clone(&self.state.providers),
        )
    }

    pub fn provider_bootstrap(&self) -> ProviderBootstrapHandle {
        ProviderBootstrapHandle::new(
            self.state.core.data_root.clone(),
            ProtectedWorkspaceStoreLookup::new(
                self.state.core.stores.clone(),
                Arc::clone(&self.state.sessions),
                Arc::clone(&self.state.transport.merge_queue),
            ),
            Arc::clone(&self.state.providers),
            self.state.telemetry.ops_events.clone(),
        )
    }

    fn provider_workspace_launch_runtime(&self) -> Arc<ProviderWorkspaceLaunchRuntime> {
        Arc::new(ProviderWorkspaceLaunchRuntime::new(
            self.state.core.data_root.clone(),
            self.state.core.daemon_url.clone(),
            self.state.core.auth_token.clone(),
            ProtectedWorkspaceStoreLookup::new(
                self.state.core.stores.clone(),
                Arc::clone(&self.state.sessions),
                Arc::clone(&self.state.transport.merge_queue),
            ),
            Arc::clone(&self.state.providers),
            self.state.telemetry.ops_events.clone(),
            Arc::clone(&self.state.execution.harness),
        ))
    }

    pub fn provider_options(&self) -> ProviderOptionsHandle {
        ProviderOptionsHandle::new(self.provider_workspace_launch_runtime())
    }

    pub fn provider_workspace_auth(&self) -> ProviderWorkspaceAuthHandle {
        ProviderWorkspaceAuthHandle::new(self.provider_workspace_launch_runtime())
    }

    pub fn provider_status(&self) -> ProviderStatusHandle {
        ProviderStatusHandle::new(
            self.state.core.data_root.clone(),
            Arc::clone(&self.state.providers),
            self.state.telemetry.ops_events.clone(),
        )
    }

    pub fn provider_admin(&self) -> ProviderAdminHandle {
        ProviderAdminHandle::new(
            self.state.core.data_root.clone(),
            Arc::clone(&self.state.providers),
            self.state.telemetry.ops_events.clone(),
        )
    }

    pub fn provider_install(&self) -> ProviderInstallHandle {
        ProviderInstallHandle::new(
            self.state.core.data_root.clone(),
            Arc::clone(&self.state.providers),
            self.state.telemetry.ops_events.clone(),
        )
    }

    pub fn provider_usage(&self) -> ProviderUsageHandle {
        ProviderUsageHandle::new(
            self.state.core.data_root.clone(),
            Arc::clone(&self.state.providers),
            self.state.core.shutdown_tx.clone(),
        )
    }

    pub fn provider_harness_config(&self) -> ProviderHarnessConfigHandle {
        ProviderHarnessConfigHandle::new(
            self.state.core.data_root.clone(),
            Arc::clone(&self.state.providers),
        )
    }

    pub fn telemetry(&self) -> TelemetryHandle {
        TelemetryHandle::new(self.state.core.data_root.clone(), &self.state.telemetry)
    }

    pub fn transport(&self) -> TransportHandle {
        TransportHandle::new(Arc::clone(&self.state))
    }

    pub fn execution_launch(&self) -> ExecutionLaunchHandle {
        ExecutionLaunchHandle::new(
            self.state.global_store().clone(),
            self.state.core.stores.clone(),
            Arc::clone(&self.state.core.update_drain),
            Arc::clone(&self.state.execution.setup),
            self.state.core.daemon_url.clone(),
        )
    }

    pub fn linux_sandbox_runtime(&self) -> LinuxSandboxRuntimeHandle {
        LinuxSandboxRuntimeHandle::new(
            self.state.core.data_root.clone(),
            self.state.global_store().clone(),
            self.state.core.stores.clone(),
            Arc::clone(&self.state.core.update_drain),
            Arc::clone(&self.state.transport.terminals),
            Arc::clone(&self.state.execution.harness),
        )
    }

    pub fn update_drain(&self) -> UpdateDrainHandle {
        UpdateDrainHandle::new(
            self.state.global_store().clone(),
            self.state.core.stores.clone(),
            Arc::clone(&self.state.core.update_drain),
        )
    }

    pub fn execution(&self) -> ExecutionHandle {
        ExecutionHandle::new(Arc::clone(&self.state))
    }
}

impl From<Arc<DaemonState>> for DaemonHandle {
    fn from(state: Arc<DaemonState>) -> Self {
        Self::new(state)
    }
}

impl TelemetryHandle {
    pub(in crate::daemon) fn new(data_root: PathBuf, runtime: &TelemetryRuntime) -> Self {
        Self {
            data_root,
            perf_telemetry: runtime.perf_telemetry.clone(),
            telemetry: runtime.telemetry.clone(),
        }
    }

    pub fn perf_telemetry(&self) -> &PerfTelemetry {
        &self.perf_telemetry
    }

    pub fn telemetry(&self) -> &Telemetry {
        &self.telemetry
    }

    pub async fn read_perf_telemetry_export_for_date(
        &self,
        date: &str,
    ) -> Result<Vec<u8>, ctx_route_contracts::telemetry::TelemetryExportError> {
        let path = ctx_observability::perf_telemetry::perf_log_path_for_date(&self.data_root, date);
        tokio::fs::read(&path)
            .await
            .map_err(|_| ctx_route_contracts::telemetry::TelemetryExportError::not_found())
    }
}

#[derive(Clone)]
pub struct TelemetryHandle {
    data_root: PathBuf,
    perf_telemetry: PerfTelemetry,
    telemetry: Telemetry,
}

#[derive(Clone)]
pub struct AuthHandle {
    auth_token: Option<String>,
    mcp_auth: Arc<McpAuthRegistry>,
    store: Store,
    ops_events: OpsEvents,
}

impl AuthHandle {
    pub(in crate::daemon) fn new(
        auth_token: Option<String>,
        mcp_auth: Arc<McpAuthRegistry>,
        store: Store,
        ops_events: OpsEvents,
    ) -> Self {
        Self {
            auth_token,
            mcp_auth,
            store,
            ops_events,
        }
    }

    pub fn auth_token(&self) -> Option<&str> {
        self.auth_token.as_deref()
    }

    pub fn has_auth_token(&self) -> bool {
        self.auth_token.is_some()
    }

    pub async fn verify_mcp_auth_token(&self, token: &str) -> Option<ctx_mcp_auth::McpAuthContext> {
        self.mcp_auth.verify_token(token).await
    }

    pub fn emit_mcp_token_denied(
        &self,
        mcp_auth: ctx_mcp_auth::McpAuthContext,
        method: &str,
        path: &str,
        reason: &str,
    ) {
        let mut event = OpsEvent::new("warn", "mcp_token_denied");
        event.session_id = Some(mcp_auth.session_id.0.to_string());
        event.worktree_id = Some(mcp_auth.worktree_id.0.to_string());
        event.meta = Some(serde_json::json!({
            "workspace_id": mcp_auth.workspace_id.0.to_string(),
            "capabilities": mcp_auth.capabilities.names(),
            "detail": {
                "method": method,
                "path": path,
                "reason": reason,
            },
        }));
        self.ops_events.emit(event);
    }

    pub async fn verify_mobile_api_token_hash(
        &self,
        hash: &str,
    ) -> Result<
        Option<ctx_mobile_access_service::MobileAuthContext>,
        ctx_mobile_access_service::MobileAuthContextError,
    > {
        ctx_mobile_access_service::verify_mobile_api_token_hash(&self.store, hash).await
    }
}

#[derive(Clone)]
pub struct HealthHandle {
    data_root: PathBuf,
    daemon_url: String,
    auth_token: Option<String>,
    storage_guard: Arc<StorageGuardRuntime>,
}

impl HealthHandle {
    pub(in crate::daemon) fn new(
        data_root: PathBuf,
        daemon_url: String,
        auth_token: Option<String>,
        storage_guard: Arc<StorageGuardRuntime>,
    ) -> Self {
        Self {
            data_root,
            daemon_url,
            auth_token,
            storage_guard,
        }
    }

    pub(in crate::daemon) fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub(in crate::daemon) fn daemon_url(&self) -> &str {
        &self.daemon_url
    }

    pub fn auth_token(&self) -> Option<&str> {
        self.auth_token.as_deref()
    }

    pub(in crate::daemon) fn auth_required(&self) -> bool {
        self.auth_token.is_some()
    }

    pub(in crate::daemon) fn storage_guard_snapshot(&self) -> StorageGuardStatus {
        self.storage_guard.snapshot()
    }
}

#[derive(Clone)]
pub struct DiagnosticsHandle {
    health: HealthHandle,
    data_root: PathBuf,
    execution_setup: Arc<ExecutionSetupCoordinator>,
    providers: Arc<ProviderRuntime>,
}

impl DiagnosticsHandle {
    pub(in crate::daemon) fn new(
        health: HealthHandle,
        data_root: PathBuf,
        execution_setup: Arc<ExecutionSetupCoordinator>,
        providers: Arc<ProviderRuntime>,
    ) -> Self {
        Self {
            health,
            data_root,
            execution_setup,
            providers,
        }
    }

    pub(in crate::daemon) fn health(&self) -> &HealthHandle {
        &self.health
    }

    pub(in crate::daemon) fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub(in crate::daemon) fn execution_setup(&self) -> &ExecutionSetupCoordinator {
        &self.execution_setup
    }

    pub(in crate::daemon) fn providers(&self) -> &ProviderRuntime {
        &self.providers
    }
}

#[derive(Clone)]
pub struct RequestBaseHandle {
    daemon_url: String,
    public_base_url: Option<String>,
}

impl RequestBaseHandle {
    pub(in crate::daemon) fn new(daemon_url: String, public_base_url: Option<String>) -> Self {
        Self {
            daemon_url,
            public_base_url,
        }
    }

    pub fn daemon_url(&self) -> &str {
        &self.daemon_url
    }

    pub fn public_base_url(&self) -> Option<&str> {
        self.public_base_url.as_deref()
    }
}

#[derive(Clone)]
pub struct LogsHandle {
    data_root: PathBuf,
}

impl LogsHandle {
    pub(in crate::daemon) fn new(data_root: PathBuf) -> Self {
        Self { data_root }
    }

    pub(in crate::daemon) fn data_root(&self) -> &Path {
        &self.data_root
    }
}

#[derive(Clone)]
pub struct OrgPolicyHandle {
    store: Store,
}

impl OrgPolicyHandle {
    pub(in crate::daemon) fn new(store: Store) -> Self {
        Self { store }
    }

    pub(in crate::daemon) fn store(&self) -> &Store {
        &self.store
    }
}

#[derive(Clone)]
pub struct DictationHandle {
    store: Store,
}

impl DictationHandle {
    pub(in crate::daemon) fn new(store: Store) -> Self {
        Self { store }
    }

    pub(in crate::daemon) fn store(&self) -> &Store {
        &self.store
    }
}

#[derive(Clone)]
pub struct UpdateReleaseHandle {
    data_root: PathBuf,
}

impl UpdateReleaseHandle {
    pub(in crate::daemon) fn new(data_root: PathBuf) -> Self {
        Self { data_root }
    }

    pub(in crate::daemon) fn data_root(&self) -> &Path {
        &self.data_root
    }
}

#[derive(Clone)]
pub struct UpdateActivityHandle {
    global_store: Store,
    stores: StoreManager,
    update_drain: Arc<UpdateDrainCoordinator>,
    data_root: PathBuf,
}

impl UpdateActivityHandle {
    pub(in crate::daemon) fn new(
        global_store: Store,
        stores: StoreManager,
        update_drain: Arc<UpdateDrainCoordinator>,
        data_root: PathBuf,
    ) -> Self {
        Self {
            global_store,
            stores,
            update_drain,
            data_root,
        }
    }

    pub(in crate::daemon) fn global_store(&self) -> &Store {
        &self.global_store
    }

    pub(in crate::daemon) fn stores(&self) -> &StoreManager {
        &self.stores
    }

    pub(in crate::daemon) fn update_drain(&self) -> &UpdateDrainCoordinator {
        self.update_drain.as_ref()
    }

    pub(in crate::daemon) fn data_root(&self) -> &Path {
        &self.data_root
    }
}

#[derive(Clone)]
pub struct SettingsHandle {
    store: Store,
    telemetry: Telemetry,
    perf_telemetry: PerfTelemetry,
    resource_sampler: Arc<Mutex<ResourceSampler>>,
    resource_governance: Arc<Mutex<ResourceGovernanceRuntime>>,
    providers: Arc<ProviderRuntime>,
    terminals: Arc<TerminalManager>,
}

impl SettingsHandle {
    pub(in crate::daemon) fn new(
        store: Store,
        telemetry: Telemetry,
        perf_telemetry: PerfTelemetry,
        resource_sampler: Arc<Mutex<ResourceSampler>>,
        resource_governance: Arc<Mutex<ResourceGovernanceRuntime>>,
        providers: Arc<ProviderRuntime>,
        terminals: Arc<TerminalManager>,
    ) -> Self {
        Self {
            store,
            telemetry,
            perf_telemetry,
            resource_sampler,
            resource_governance,
            providers,
            terminals,
        }
    }

    pub(in crate::daemon) fn store(&self) -> &Store {
        &self.store
    }

    pub(in crate::daemon) fn telemetry(&self) -> &Telemetry {
        &self.telemetry
    }

    pub(in crate::daemon) fn perf_telemetry(&self) -> &PerfTelemetry {
        &self.perf_telemetry
    }

    pub(in crate::daemon) fn resource_sampler(&self) -> &Mutex<ResourceSampler> {
        self.resource_sampler.as_ref()
    }

    pub(in crate::daemon) fn resource_governance(&self) -> &Mutex<ResourceGovernanceRuntime> {
        self.resource_governance.as_ref()
    }

    pub(in crate::daemon) fn providers(&self) -> &ProviderRuntime {
        self.providers.as_ref()
    }

    pub(in crate::daemon) fn terminals(&self) -> &TerminalManager {
        self.terminals.as_ref()
    }
}

#[derive(Clone)]
pub struct MobileStoreHandle {
    store: Store,
}

impl MobileStoreHandle {
    pub(in crate::daemon) fn new(store: Store) -> Self {
        Self { store }
    }

    pub(in crate::daemon) fn store(&self) -> &Store {
        &self.store
    }
}

#[derive(Clone)]
pub struct MobileRuntimeHandle {
    store: Store,
    mobile_tunnel: MobileTunnelManager,
    daemon_url: String,
    auth_token_configured: bool,
}

impl MobileRuntimeHandle {
    pub(in crate::daemon) fn new(
        store: Store,
        mobile_tunnel: MobileTunnelManager,
        daemon_url: String,
        auth_token_configured: bool,
    ) -> Self {
        Self {
            store,
            mobile_tunnel,
            daemon_url,
            auth_token_configured,
        }
    }

    pub(in crate::daemon) fn store(&self) -> &Store {
        &self.store
    }

    pub(in crate::daemon) fn mobile_tunnel(&self) -> &MobileTunnelManager {
        &self.mobile_tunnel
    }

    pub(in crate::daemon) fn daemon_url(&self) -> &str {
        &self.daemon_url
    }

    pub(in crate::daemon) fn auth_token_configured(&self) -> bool {
        self.auth_token_configured
    }
}

#[derive(Clone)]
pub struct MobileSecureProxyHandle {
    store: Store,
    health: HealthHandle,
    telemetry: Telemetry,
}

impl MobileSecureProxyHandle {
    pub(in crate::daemon) fn new(store: Store, health: HealthHandle, telemetry: Telemetry) -> Self {
        Self {
            store,
            health,
            telemetry,
        }
    }

    pub(in crate::daemon) fn store(&self) -> &Store {
        &self.store
    }

    pub(in crate::daemon) fn health(&self) -> &HealthHandle {
        &self.health
    }

    pub(in crate::daemon) fn telemetry(&self) -> &Telemetry {
        &self.telemetry
    }
}

#[derive(Clone)]
pub struct ProviderAccountsHandle {
    data_root: PathBuf,
    providers: Arc<ProviderRuntime>,
}

impl ProviderAccountsHandle {
    pub(in crate::daemon) fn new(data_root: PathBuf, providers: Arc<ProviderRuntime>) -> Self {
        Self {
            data_root,
            providers,
        }
    }

    pub(in crate::daemon) fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub(in crate::daemon) fn providers(&self) -> &ProviderRuntime {
        self.providers.as_ref()
    }
}

#[derive(Clone)]
pub(in crate::daemon) struct ProtectedWorkspaceStoreLookup {
    stores: StoreManager,
    sessions: Arc<SessionRuntime<crate::daemon::scheduler::SchedulerCommand>>,
    merge_queue: Arc<MergeQueueRuntime>,
}

impl ProtectedWorkspaceStoreLookup {
    pub(in crate::daemon) fn new(
        stores: StoreManager,
        sessions: Arc<SessionRuntime<crate::daemon::scheduler::SchedulerCommand>>,
        merge_queue: Arc<MergeQueueRuntime>,
    ) -> Self {
        Self {
            stores,
            sessions,
            merge_queue,
        }
    }

    pub(in crate::daemon) fn global_store(&self) -> &Store {
        self.stores.global()
    }

    pub(in crate::daemon) async fn store_for_workspace(
        &self,
        workspace_id: WorkspaceId,
    ) -> anyhow::Result<Store> {
        match self.stores.workspace_access_outcome(workspace_id).await {
            Ok(WorkspaceStoreAccessOutcome::Access(access)) => {
                if access.kind.triggers_open_side_effects() {
                    let mut protected = self.protected_workspace_store_ids().await;
                    protected.insert(workspace_id);
                    self.stores.evict_workspaces_to_cap(&protected).await;
                }
                Ok(access.store)
            }
            Ok(WorkspaceStoreAccessOutcome::Missing | WorkspaceStoreAccessOutcome::Deleting) => {
                anyhow::bail!("workspace {} not found", workspace_id.0)
            }
            Err(err) => Err(err),
        }
    }

    async fn protected_workspace_store_ids(&self) -> HashSet<WorkspaceId> {
        let mut active_sessions: HashSet<SessionId> = HashSet::new();
        {
            let set = self.sessions.running_sessions.lock().await;
            active_sessions.extend(set.iter().copied());
        }
        {
            let map = self.sessions.schedulers.lock().await;
            active_sessions.extend(map.keys().copied());
        }
        {
            let map = self.sessions.broadcasters.lock().await;
            active_sessions.extend(map.keys().copied());
        }
        {
            let map = self.sessions.session_event_heads.lock().await;
            active_sessions.extend(map.keys().copied());
        }

        let mut active_workspaces: HashSet<WorkspaceId> = HashSet::new();
        let mut missing = Vec::new();
        {
            let cache = self.sessions.session_meta_cache.lock().await;
            for session_id in &active_sessions {
                if let Some(entry) = cache.get(session_id) {
                    active_workspaces.insert(entry.value.workspace_id);
                } else {
                    missing.push(*session_id);
                }
            }
        }
        for session_id in missing {
            if let Ok(Some(workspace_id)) = self
                .stores
                .global()
                .get_workspace_id_for_session(session_id)
                .await
            {
                active_workspaces.insert(workspace_id);
            }
        }
        active_workspaces.extend(self.merge_queue.running_workspaces().await);
        active_workspaces
    }
}

#[derive(Clone)]
pub struct ProviderBootstrapHandle {
    data_root: PathBuf,
    workspace_stores: ProtectedWorkspaceStoreLookup,
    providers: Arc<ProviderRuntime>,
    ops_events: OpsEvents,
}

impl ProviderBootstrapHandle {
    pub(in crate::daemon) fn new(
        data_root: PathBuf,
        workspace_stores: ProtectedWorkspaceStoreLookup,
        providers: Arc<ProviderRuntime>,
        ops_events: OpsEvents,
    ) -> Self {
        Self {
            data_root,
            workspace_stores,
            providers,
            ops_events,
        }
    }

    pub(in crate::daemon) fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub(in crate::daemon) fn providers(&self) -> &ProviderRuntime {
        self.providers.as_ref()
    }

    pub(in crate::daemon) fn ops_events(&self) -> &OpsEvents {
        &self.ops_events
    }

    pub(in crate::daemon) fn global_store(&self) -> &Store {
        self.workspace_stores.global_store()
    }

    pub(in crate::daemon) async fn store_for_workspace(
        &self,
        workspace_id: WorkspaceId,
    ) -> anyhow::Result<Store> {
        self.workspace_stores
            .store_for_workspace(workspace_id)
            .await
    }

    pub(in crate::daemon) async fn install_target_for_workspace(
        &self,
        workspace_id: WorkspaceId,
    ) -> anyhow::Result<InstallTarget> {
        let store = self.store_for_workspace(workspace_id).await?;
        let effective =
            ctx_settings_service::effective_execution_settings(self.global_store(), &store)
                .await
                .with_context(|| {
                    format!(
                        "loading execution settings for workspace {}",
                        workspace_id.0
                    )
                })?;
        Ok(ctx_settings_service::install_target_for_settings(
            &effective,
        ))
    }
}

#[derive(Clone)]
pub(in crate::daemon) struct ProviderWorkspaceLaunchRuntime {
    data_root: PathBuf,
    daemon_url: String,
    auth_token: Option<String>,
    workspace_stores: ProtectedWorkspaceStoreLookup,
    providers: Arc<ProviderRuntime>,
    ops_events: OpsEvents,
    harness: Arc<HarnessRuntimeManager>,
}

impl ProviderWorkspaceLaunchRuntime {
    pub(in crate::daemon) fn new(
        data_root: PathBuf,
        daemon_url: String,
        auth_token: Option<String>,
        workspace_stores: ProtectedWorkspaceStoreLookup,
        providers: Arc<ProviderRuntime>,
        ops_events: OpsEvents,
        harness: Arc<HarnessRuntimeManager>,
    ) -> Self {
        Self {
            data_root,
            daemon_url,
            auth_token,
            workspace_stores,
            providers,
            ops_events,
            harness,
        }
    }

    pub(in crate::daemon) fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub(in crate::daemon) fn daemon_url(&self) -> &str {
        &self.daemon_url
    }

    pub(in crate::daemon) fn auth_token(&self) -> Option<&String> {
        self.auth_token.as_ref()
    }

    pub(in crate::daemon) fn providers(&self) -> &ProviderRuntime {
        self.providers.as_ref()
    }

    pub(in crate::daemon) fn ops_events(&self) -> &OpsEvents {
        &self.ops_events
    }

    pub(in crate::daemon) fn harness(&self) -> &HarnessRuntimeManager {
        self.harness.as_ref()
    }

    pub(in crate::daemon) fn global_store(&self) -> &Store {
        self.workspace_stores.global_store()
    }

    pub(in crate::daemon) async fn load_workspace(
        &self,
        workspace_id: WorkspaceId,
    ) -> anyhow::Result<Option<ctx_core::models::Workspace>> {
        self.global_store().get_workspace(workspace_id).await
    }

    pub(in crate::daemon) async fn store_for_workspace(
        &self,
        workspace_id: WorkspaceId,
    ) -> anyhow::Result<Store> {
        self.workspace_stores
            .store_for_workspace(workspace_id)
            .await
    }

    pub(in crate::daemon) async fn install_target_for_workspace(
        &self,
        workspace_id: WorkspaceId,
    ) -> anyhow::Result<InstallTarget> {
        let store = self.store_for_workspace(workspace_id).await?;
        let effective =
            ctx_settings_service::effective_execution_settings(self.global_store(), &store)
                .await
                .with_context(|| {
                    format!(
                        "loading execution settings for workspace {}",
                        workspace_id.0
                    )
                })?;
        Ok(ctx_settings_service::install_target_for_settings(
            &effective,
        ))
    }
}

#[derive(Clone)]
pub struct ProviderOptionsHandle {
    launch: Arc<ProviderWorkspaceLaunchRuntime>,
}

impl ProviderOptionsHandle {
    pub(in crate::daemon) fn new(launch: Arc<ProviderWorkspaceLaunchRuntime>) -> Self {
        Self { launch }
    }

    pub(in crate::daemon) fn launch(&self) -> &ProviderWorkspaceLaunchRuntime {
        self.launch.as_ref()
    }
}

#[derive(Clone)]
pub struct ProviderWorkspaceAuthHandle {
    launch: Arc<ProviderWorkspaceLaunchRuntime>,
}

impl ProviderWorkspaceAuthHandle {
    pub(in crate::daemon) fn new(launch: Arc<ProviderWorkspaceLaunchRuntime>) -> Self {
        Self { launch }
    }

    pub(in crate::daemon) fn launch(&self) -> &ProviderWorkspaceLaunchRuntime {
        self.launch.as_ref()
    }
}

#[derive(Clone)]
pub struct ProviderStatusHandle {
    data_root: PathBuf,
    providers: Arc<ProviderRuntime>,
    ops_events: OpsEvents,
}

impl ProviderStatusHandle {
    pub(in crate::daemon) fn new(
        data_root: PathBuf,
        providers: Arc<ProviderRuntime>,
        ops_events: OpsEvents,
    ) -> Self {
        Self {
            data_root,
            providers,
            ops_events,
        }
    }

    pub(in crate::daemon) fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub(in crate::daemon) fn providers(&self) -> &ProviderRuntime {
        self.providers.as_ref()
    }

    pub(in crate::daemon) fn ops_events(&self) -> &OpsEvents {
        &self.ops_events
    }
}

#[derive(Clone)]
pub struct ProviderAdminHandle {
    data_root: PathBuf,
    providers: Arc<ProviderRuntime>,
    ops_events: OpsEvents,
}

impl ProviderAdminHandle {
    pub(in crate::daemon) fn new(
        data_root: PathBuf,
        providers: Arc<ProviderRuntime>,
        ops_events: OpsEvents,
    ) -> Self {
        Self {
            data_root,
            providers,
            ops_events,
        }
    }

    pub(in crate::daemon) fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub(in crate::daemon) fn providers(&self) -> &ProviderRuntime {
        self.providers.as_ref()
    }

    pub(in crate::daemon) fn ops_events(&self) -> &OpsEvents {
        &self.ops_events
    }
}

#[derive(Clone)]
pub struct ProviderInstallHandle {
    data_root: PathBuf,
    providers: Arc<ProviderRuntime>,
    ops_events: OpsEvents,
}

impl ProviderInstallHandle {
    pub(in crate::daemon) fn new(
        data_root: PathBuf,
        providers: Arc<ProviderRuntime>,
        ops_events: OpsEvents,
    ) -> Self {
        Self {
            data_root,
            providers,
            ops_events,
        }
    }

    pub(in crate::daemon) fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub(in crate::daemon) fn providers(&self) -> &ProviderRuntime {
        self.providers.as_ref()
    }

    pub(in crate::daemon) fn ops_events(&self) -> &OpsEvents {
        &self.ops_events
    }

    pub(in crate::daemon) async fn get_install_polling_info(
        &self,
        install_id: InstallId,
    ) -> Option<InstallInfo> {
        let outcome = self.providers.get_install_polling_info(install_id).await;
        crate::daemon::provider_capability_hosts::emit_provider_install_ops_events(
            &self.ops_events,
            outcome.ops_events,
        );
        outcome.info
    }

    pub(in crate::daemon) async fn cancel_install(
        &self,
        install_id: InstallId,
    ) -> Option<InstallInfo> {
        let outcome = self.providers.cancel_install(install_id).await?;
        crate::daemon::provider_capability_hosts::emit_provider_install_ops_events(
            &self.ops_events,
            outcome.ops_events,
        );
        Some(outcome.info)
    }

    pub(in crate::daemon) async fn list_install_events(
        &self,
        install_id: InstallId,
    ) -> Option<Vec<InstallProgressEvent>> {
        let outcome = self.providers.get_install_events(install_id).await;
        crate::daemon::provider_capability_hosts::emit_provider_install_ops_events(
            &self.ops_events,
            outcome.ops_events,
        );
        outcome.events
    }

    pub(in crate::daemon) async fn install_event_sender(
        &self,
        install_id: InstallId,
    ) -> Option<broadcast::Sender<InstallProgressEvent>> {
        self.providers.get_install_sender(install_id).await
    }
}

#[derive(Clone)]
pub struct ProviderUsageHandle {
    data_root: PathBuf,
    providers: Arc<ProviderRuntime>,
    shutdown_tx: broadcast::Sender<()>,
}

impl ProviderUsageHandle {
    pub(in crate::daemon) fn new(
        data_root: PathBuf,
        providers: Arc<ProviderRuntime>,
        shutdown_tx: broadcast::Sender<()>,
    ) -> Self {
        Self {
            data_root,
            providers,
            shutdown_tx,
        }
    }

    pub(in crate::daemon) fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub(in crate::daemon) fn providers(&self) -> &ProviderRuntime {
        self.providers.as_ref()
    }

    pub(in crate::daemon) fn shutdown_tx(&self) -> &broadcast::Sender<()> {
        &self.shutdown_tx
    }
}

#[derive(Clone)]
pub struct ProviderHarnessConfigHandle {
    data_root: PathBuf,
    providers: Arc<ProviderRuntime>,
}

impl ProviderHarnessConfigHandle {
    pub(in crate::daemon) fn new(data_root: PathBuf, providers: Arc<ProviderRuntime>) -> Self {
        Self {
            data_root,
            providers,
        }
    }

    pub(in crate::daemon) fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub(in crate::daemon) fn providers(&self) -> &ProviderRuntime {
        self.providers.as_ref()
    }
}

#[derive(Clone)]
pub struct ExecutionLaunchHandle {
    global_store: Store,
    stores: StoreManager,
    update_drain: Arc<UpdateDrainCoordinator>,
    execution_setup: Arc<ExecutionSetupCoordinator>,
    daemon_url: String,
}

impl ExecutionLaunchHandle {
    pub(in crate::daemon) fn new(
        global_store: Store,
        stores: StoreManager,
        update_drain: Arc<UpdateDrainCoordinator>,
        execution_setup: Arc<ExecutionSetupCoordinator>,
        daemon_url: String,
    ) -> Self {
        Self {
            global_store,
            stores,
            update_drain,
            execution_setup,
            daemon_url,
        }
    }

    pub(in crate::daemon) fn global_store(&self) -> &Store {
        &self.global_store
    }

    pub(in crate::daemon) fn stores(&self) -> &StoreManager {
        &self.stores
    }

    pub(in crate::daemon) fn update_drain(&self) -> &UpdateDrainCoordinator {
        self.update_drain.as_ref()
    }

    pub(in crate::daemon) fn execution_setup(&self) -> &Arc<ExecutionSetupCoordinator> {
        &self.execution_setup
    }

    pub(in crate::daemon) fn daemon_url(&self) -> &str {
        &self.daemon_url
    }
}

#[derive(Clone)]
pub struct LinuxSandboxRuntimeHandle {
    data_root: PathBuf,
    global_store: Store,
    stores: StoreManager,
    update_drain: Arc<UpdateDrainCoordinator>,
    terminals: Arc<TerminalManager>,
    harness: Arc<HarnessRuntimeManager>,
}

impl LinuxSandboxRuntimeHandle {
    pub(in crate::daemon) fn new(
        data_root: PathBuf,
        global_store: Store,
        stores: StoreManager,
        update_drain: Arc<UpdateDrainCoordinator>,
        terminals: Arc<TerminalManager>,
        harness: Arc<HarnessRuntimeManager>,
    ) -> Self {
        Self {
            data_root,
            global_store,
            stores,
            update_drain,
            terminals,
            harness,
        }
    }

    pub(in crate::daemon) fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub(in crate::daemon) fn global_store(&self) -> &Store {
        &self.global_store
    }

    pub(in crate::daemon) fn stores(&self) -> &StoreManager {
        &self.stores
    }

    pub(in crate::daemon) fn update_drain(&self) -> Arc<UpdateDrainCoordinator> {
        Arc::clone(&self.update_drain)
    }

    pub(in crate::daemon) fn terminals(&self) -> &TerminalManager {
        self.terminals.as_ref()
    }

    pub(in crate::daemon) fn harness(&self) -> &HarnessRuntimeManager {
        self.harness.as_ref()
    }
}

#[derive(Clone)]
pub struct UpdateDrainHandle {
    global_store: Store,
    stores: StoreManager,
    update_drain: Arc<UpdateDrainCoordinator>,
}

impl UpdateDrainHandle {
    pub(in crate::daemon) fn new(
        global_store: Store,
        stores: StoreManager,
        update_drain: Arc<UpdateDrainCoordinator>,
    ) -> Self {
        Self {
            global_store,
            stores,
            update_drain,
        }
    }

    pub(in crate::daemon) fn global_store(&self) -> &Store {
        &self.global_store
    }

    pub(in crate::daemon) fn stores(&self) -> &StoreManager {
        &self.stores
    }

    pub(in crate::daemon) fn update_drain(&self) -> Arc<UpdateDrainCoordinator> {
        Arc::clone(&self.update_drain)
    }
}

macro_rules! domain_handle_with_accessor {
    ($name:ident, $accessor:ident) => {
        #[allow(dead_code)]
        #[derive(Clone)]
        pub struct $name {
            pub(in crate::daemon) state: Arc<DaemonState>,
        }

        impl $name {
            fn new(state: Arc<DaemonState>) -> Self {
                Self { state }
            }
        }
    };
}

domain_handle_with_accessor!(SessionsHandle, sessions);
domain_handle_with_accessor!(TasksHandle, tasks);
domain_handle_with_accessor!(WorkspacesHandle, workspaces);
domain_handle_with_accessor!(WorkspaceStreamHandle, workspace_stream);
domain_handle_with_accessor!(ProvidersHandle, providers);
domain_handle_with_accessor!(TransportHandle, transport);
domain_handle_with_accessor!(ExecutionHandle, execution);
