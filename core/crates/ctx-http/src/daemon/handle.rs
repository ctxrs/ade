use std::path::Path;
use std::sync::Arc;

use axum::extract::FromRef;
use ctx_observability::perf_telemetry::PerfTelemetry;
use ctx_observability::telemetry::Telemetry;
use ctx_storage_admission::StorageGuardStatus;
use ctx_store::Store;

use super::state::DaemonState;

#[derive(Clone)]
pub struct DaemonHandle {
    state: Arc<DaemonState>,
}

impl DaemonHandle {
    pub fn new(state: Arc<DaemonState>) -> Self {
        Self { state }
    }

    pub fn core(&self) -> CoreHandle {
        CoreHandle::new(Arc::clone(&self.state))
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

    pub fn telemetry(&self) -> TelemetryHandle {
        TelemetryHandle::new(Arc::clone(&self.state))
    }

    pub fn transport(&self) -> TransportHandle {
        TransportHandle::new(Arc::clone(&self.state))
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

impl CoreHandle {
    pub(in crate::daemon) fn global_store(&self) -> &Store {
        self.state.global_store()
    }

    pub(crate) async fn insert_blob(
        &self,
        id: &str,
        sha256: &str,
        bytes: i64,
        mime_type: &str,
        name: Option<&str>,
        created_at: chrono::DateTime<chrono::Utc>,
    ) -> anyhow::Result<()> {
        self.state
            .global_store()
            .insert_blob(id, sha256, bytes, mime_type, name, created_at)
            .await
    }

    pub(crate) async fn get_blob(
        &self,
        id: &str,
    ) -> anyhow::Result<
        Option<(
            String,
            String,
            i64,
            Option<String>,
            chrono::DateTime<chrono::Utc>,
        )>,
    > {
        self.state.global_store().get_blob(id).await
    }

    pub(crate) fn data_root(&self) -> &Path {
        &self.state.core.data_root
    }

    pub(crate) fn daemon_url(&self) -> &str {
        &self.state.core.daemon_url
    }

    pub(crate) fn public_base_url(&self) -> Option<&str> {
        self.state.core.public_base_url.as_deref()
    }

    pub(crate) fn auth_token(&self) -> Option<&str> {
        self.state.core.auth_token.as_deref()
    }

    pub(crate) fn has_auth_token(&self) -> bool {
        self.auth_token().is_some()
    }

    pub(crate) fn local_shutdown_token(&self) -> Option<&str> {
        self.state.core.local_shutdown_token.as_deref()
    }

    pub(crate) fn storage_guard_snapshot(&self) -> StorageGuardStatus {
        self.state.storage_guard_snapshot()
    }

    pub(crate) async fn verify_mcp_auth_token(
        &self,
        token: &str,
    ) -> Option<ctx_mcp_auth::McpAuthContext> {
        crate::daemon::verify_mcp_auth_token(&self.state, token).await
    }

    pub(crate) fn emit_mcp_token_denied(
        &self,
        mcp_auth: ctx_mcp_auth::McpAuthContext,
        method: &str,
        path: &str,
        reason: &str,
    ) {
        crate::daemon::emit_mcp_token_denied(&self.state, mcp_auth, method, path, reason);
    }
}

impl TelemetryHandle {
    pub(crate) fn perf_telemetry(&self) -> &PerfTelemetry {
        &self.state.telemetry.perf_telemetry
    }

    pub(crate) fn telemetry(&self) -> &Telemetry {
        &self.state.telemetry.telemetry
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

        impl FromRef<DaemonHandle> for $name {
            fn from_ref(handle: &DaemonHandle) -> Self {
                handle.$accessor()
            }
        }
    };
}

domain_handle_with_accessor!(CoreHandle, core);
domain_handle_with_accessor!(SessionsHandle, sessions);
domain_handle_with_accessor!(TasksHandle, tasks);
domain_handle_with_accessor!(WorkspacesHandle, workspaces);
domain_handle_with_accessor!(WorkspaceStreamHandle, workspace_stream);
domain_handle_with_accessor!(ProvidersHandle, providers);
domain_handle_with_accessor!(TelemetryHandle, telemetry);
domain_handle_with_accessor!(TransportHandle, transport);
domain_handle_with_accessor!(ExecutionHandle, execution);
