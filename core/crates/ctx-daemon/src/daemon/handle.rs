use std::path::Path;
use std::sync::Arc;

use ctx_mcp_auth::McpAuthRegistry;
use ctx_observability::ops_events::{OpsEvent, OpsEvents};
use ctx_observability::perf_telemetry::PerfTelemetry;
use ctx_observability::telemetry::Telemetry;
use ctx_storage_admission::StorageGuardStatus;
use ctx_store::Store;

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

    pub fn core(&self) -> CoreHandle {
        CoreHandle::new(Arc::clone(&self.state))
    }

    pub fn auth(&self) -> AuthHandle {
        AuthHandle::new(
            self.state.core.auth_token.clone(),
            Arc::clone(&self.state.core.mcp_auth),
            self.state.global_store().clone(),
            self.state.telemetry.ops_events.clone(),
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
        TelemetryHandle::new(&self.state.telemetry)
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

    pub async fn insert_blob(
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

    pub async fn get_blob(
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

    pub fn data_root(&self) -> &Path {
        &self.state.core.data_root
    }

    pub fn daemon_url(&self) -> &str {
        &self.state.core.daemon_url
    }

    pub fn public_base_url(&self) -> Option<&str> {
        self.state.core.public_base_url.as_deref()
    }

    pub fn auth_token(&self) -> Option<&str> {
        self.state.core.auth_token.as_deref()
    }

    pub fn has_auth_token(&self) -> bool {
        self.auth_token().is_some()
    }

    pub fn subscribe_shutdown(&self) -> tokio::sync::broadcast::Receiver<()> {
        self.state.core.shutdown_tx.subscribe()
    }

    pub fn storage_guard_snapshot(&self) -> StorageGuardStatus {
        self.state.storage_guard_snapshot()
    }
}

impl TelemetryHandle {
    pub(in crate::daemon) fn new(runtime: &TelemetryRuntime) -> Self {
        Self {
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
}

#[derive(Clone)]
pub struct TelemetryHandle {
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
    data_root: std::path::PathBuf,
}

impl LogsHandle {
    pub(in crate::daemon) fn new(data_root: std::path::PathBuf) -> Self {
        Self { data_root }
    }

    pub(in crate::daemon) fn data_root(&self) -> &Path {
        &self.data_root
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

domain_handle_with_accessor!(CoreHandle, core);
domain_handle_with_accessor!(SessionsHandle, sessions);
domain_handle_with_accessor!(TasksHandle, tasks);
domain_handle_with_accessor!(WorkspacesHandle, workspaces);
domain_handle_with_accessor!(WorkspaceStreamHandle, workspace_stream);
domain_handle_with_accessor!(ProvidersHandle, providers);
domain_handle_with_accessor!(TransportHandle, transport);
domain_handle_with_accessor!(ExecutionHandle, execution);
