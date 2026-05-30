use std::path::PathBuf;
use std::sync::Arc;

use ctx_execution_runtime::ExecutionSetupCoordinator;
use ctx_mcp_auth::McpAuthRegistry;
use ctx_observability::ops_events::OpsEvents;
use ctx_provider_runtime::ProviderRuntime;
use ctx_storage_admission::StorageGuardRuntime;
use ctx_store::Store;

pub(super) struct CoreRouteDepsParts {
    pub(super) data_root: PathBuf,
    pub(super) daemon_url: String,
    pub(super) public_base_url: Option<String>,
    pub(super) auth_token: Option<String>,
    pub(super) mcp_auth: Arc<McpAuthRegistry>,
    pub(super) storage_guard: Arc<StorageGuardRuntime>,
    pub(super) global_store: Store,
    pub(super) ops_events: OpsEvents,
    pub(super) execution_setup: Arc<ExecutionSetupCoordinator>,
    pub(super) providers: Arc<ProviderRuntime>,
}

#[derive(Clone)]
pub(super) struct CoreRouteDeps {
    pub(super) data_root: PathBuf,
    pub(super) daemon_url: String,
    pub(super) public_base_url: Option<String>,
    pub(super) auth_token: Option<String>,
    pub(super) mcp_auth: Arc<McpAuthRegistry>,
    pub(super) storage_guard: Arc<StorageGuardRuntime>,
    pub(super) global_store: Store,
    pub(super) ops_events: OpsEvents,
    pub(super) execution_setup: Arc<ExecutionSetupCoordinator>,
    pub(super) providers: Arc<ProviderRuntime>,
}

impl CoreRouteDeps {
    pub(super) fn new(parts: CoreRouteDepsParts) -> Self {
        Self {
            data_root: parts.data_root,
            daemon_url: parts.daemon_url,
            public_base_url: parts.public_base_url,
            auth_token: parts.auth_token,
            mcp_auth: parts.mcp_auth,
            storage_guard: parts.storage_guard,
            global_store: parts.global_store,
            ops_events: parts.ops_events,
            execution_setup: parts.execution_setup,
            providers: parts.providers,
        }
    }
}
