mod runtime_parts;
#[cfg(not(test))]
mod startup;

use super::*;
use ctx_storage_admission::StorageGuardRuntime;
use ctx_workspace_services::worktree_vcs::worktree_vcs_enabled_from_env;
use runtime_parts::{
    build_execution_runtime, build_provider_runtime, build_runtime_parts, build_telemetry_runtime,
    build_tool_output_spool, build_transport_runtime, build_workspace_runtime,
};

impl AppState {
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
        Self::new_with_runtime_flags(
            data_root,
            stores,
            providers,
            daemon_url,
            public_base_url,
            auth_token,
            AppRuntimeFlags {
                worktree_vcs_enabled: worktree_vcs_enabled_from_env(),
            },
        )
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
        let worktree_vcs_enabled = runtime_flags.worktree_vcs_enabled;
        let tool_output_spool = build_tool_output_spool(&data_root);
        let runtime_parts = build_runtime_parts(&data_root);
        let storage_guard = StorageGuardRuntime::new(&data_root);
        let local_shutdown_token = std::env::var("CTX_LOCAL_DAEMON_SHUTDOWN_TOKEN")
            .ok()
            .filter(|value| !value.trim().is_empty());
        #[cfg(not(test))]
        startup::spawn_startup_prewarm_loader(
            data_root.clone(),
            Arc::clone(&runtime_parts.execution_setup),
        );
        Self {
            core: CoreState {
                data_root,
                storage_guard,
                tool_output_spool_enabled: tool_output_spool.enabled,
                tool_output_spool_dir: tool_output_spool.dir,
                stores,
                daemon_url,
                public_base_url,
                auth_token,
                local_shutdown_token,
                mcp_auth: ctx_mcp_auth::McpAuthRegistry::new(),
                ask_user_question: runtime_parts.ask_user_question,
                shutdown_tx: runtime_parts.shutdown_tx,
                update_drain: Arc::new(ctx_update_service::UpdateDrainCoordinator::new()),
            },
            sessions: SessionRuntime::new_from_env(),
            workspaces: build_workspace_runtime(
                worktree_vcs_enabled,
                runtime_parts.workspace_active_snapshot,
            ),
            providers: build_provider_runtime(providers),
            telemetry: build_telemetry_runtime(
                runtime_parts.telemetry,
                runtime_parts.ops_events,
                runtime_parts.perf_telemetry,
                runtime_parts.provider_unknown_events,
            ),
            transport: build_transport_runtime(runtime_parts.terminals, runtime_parts.web_sessions),
            execution: build_execution_runtime(
                runtime_parts.harness_runtime,
                runtime_parts.execution_setup,
            ),
        }
    }
}
