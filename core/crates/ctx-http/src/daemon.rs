use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::Router;
use directories::BaseDirs;
use serde::Serialize;
use serde_json::json;

use ctx_core::ids::WorkspaceId;
use ctx_core::models::{ExecutionEnvironment, SessionTurn, SessionTurnStatus};
use ctx_providers::adapters::ProviderAdapter;
use ctx_providers::crp::Tier1CrpAdapter;
use ctx_providers::fake::FakeProviderAdapter;
use ctx_store::{Store, StoreManager, StoreManagerConfig};

use crate::api;
use crate::daemon::scheduler::reconcile_turn_terminal_state;
use ctx_observability::telemetry::TelemetryConfig;
use ctx_provider_install::install_state::InstallTarget;
use ctx_provider_runtime::provider_usage;

mod activity;
mod auth;
pub(crate) mod execution_effective;
pub mod git_status;
pub(crate) mod installer;
mod lifecycle;
mod listener;
mod managed_auto_update;
mod mcp_auth;
mod memleak_debug;
pub(crate) mod merge_queue;
mod mobile_startup;
mod provider_adapters;
mod provider_bootstrap;
mod provider_child_reclassifier;
pub mod provider_guard;
pub(crate) mod provider_launch;
mod provider_registry;
pub mod provider_restart;
mod provider_runtime;
pub(crate) mod resource_governance;
pub mod resource_telemetry;
mod retention;
pub mod scheduler;
pub(crate) mod sessions;
mod state;
pub(crate) mod storage_guard;
pub(crate) mod tool_cgroup;
mod workspace_init;
#[cfg(test)]
mod workspace_runtime;
pub(crate) mod workspaces;

use activity::reconcile_running_turns;
pub(crate) use activity::reconcile_running_turns_with_reason;
pub use activity::{
    daemon_sandbox_work_activity_summary, daemon_turn_activity_summary, ActiveTurnRecord,
    DaemonSandboxWorkActivitySummary, DaemonTurnActivitySummary,
};
pub use ctx_provider_runtime::{CachedProviderOptions, CachedProviderVerify};
pub use ctx_update_service::UpdateDrainState;
pub use ctx_workspace_services::file_completions::CachedFileCompletions;
pub(crate) use lifecycle::spawn_deferred_daemon_shutdown;
#[cfg(test)]
pub(crate) use lifecycle::{collect_provider_adapters_for_shutdown, shutdown_provider_adapters};
#[cfg(test)]
pub(crate) use listener::daemon_public_base_url_from_env;
pub use mcp_auth::issue_provider_session_mcp_token;
pub(crate) use mcp_auth::{
    emit_mcp_token_denied, issue_provider_session_mcp_token_with_capabilities,
    revoke_provider_session_mcp_token, verify_mcp_auth_token, McpAuthCapabilities, McpAuthContext,
};
#[cfg(test)]
pub(crate) use provider_adapters::runtime_probe_command_as_agent_command;
pub(crate) use provider_adapters::{acp_bridge_adapter, acp_bridge_command, is_acp_provider_id};
use provider_adapters::{
    acp_status_adapter_acp_command_invalid, acp_status_adapter_bridge_invalid,
    acp_status_adapter_bridge_missing, runtime_command_as_agent_command_for_target,
    runtime_command_invalid_adapter, runtime_command_missing_adapter, target_adapter_cache_key,
};
pub(crate) use provider_bootstrap::{
    ensure_provider_adapter_for_target, ensure_provider_adapter_for_target_with_cfg,
    load_managed_agent_server_config_or_err, normalize_acp_provider_command,
};
#[cfg(test)]
pub(crate) use retention::prune_archived_session_data_for_all_workspaces;
pub(crate) use state::AttachmentMaterializationTask;
pub(crate) use state::WorktreeVcsDirtyBits;
pub use state::{
    AppRuntimeFlags, AppState, CacheSweepConfig, CacheSweepStats, GitStatusSnapshotCacheEntry,
    SessionHeadCacheKey, StoreLookup, TimedEntry, WorkspaceActiveHeadCacheEntry,
    WorkspaceActiveSnapshotCacheEntry, WorktreeVcsSnapshotCacheEntry,
};
pub use workspace_init::init_workspace;

fn spawn_startup_provider_status_refresh(state: Arc<AppState>) {
    tokio::spawn(async move {
        if let Err(err) = installer::refresh_provider_statuses(state.as_ref()).await {
            tracing::warn!("startup provider status refresh failed: {err:#}");
        }
    });
}

pub async fn serve(bind: Vec<String>, data_dir: Option<String>) -> Result<()> {
    let data_root = match data_dir {
        Some(p) => PathBuf::from(p),
        None => {
            let base = BaseDirs::new().context("resolving home dir")?;
            base.home_dir().join(".ctx")
        }
    };
    ctx_fs::permissions::ensure_private_dir(&data_root).await?;
    // Canonicalize so legacy machine mount sources resolve under shared roots on macOS
    // (e.g. /tmp -> /private/tmp). This also reduces accidental duplicate state roots.
    let data_root = tokio::fs::canonicalize(&data_root)
        .await
        .unwrap_or(data_root);
    ctx_fs::permissions::ensure_private_dir(&data_root).await?;
    ctx_fs::permissions::ensure_private_dir(&data_root.join("logs"))
        .await
        .ok();

    let _daemon_lock = auth::acquire_daemon_lock(&data_root)?;

    let global_db_path = data_root.join("db").join("db.sqlite");
    let bootstrap_store = Store::open_sqlite(&global_db_path, None).await?;
    let settings_data = ctx_settings_service::load_settings(&bootstrap_store).await?;
    bootstrap_store.close().await;
    let store_config = settings_data
        .storage
        .as_ref()
        .map(|storage| StoreManagerConfig {
            max_connections: storage.max_connections,
            workspace_max_connections: storage.max_connections,
            ..StoreManagerConfig::default()
        })
        .unwrap_or_default();
    let stores = StoreManager::open_with_config(&data_root, store_config).await?;

    retention::spawn_archived_session_data_pruner(stores.clone());

    let agent_cfg = load_managed_agent_server_config_or_err(&data_root).await?;

    let providers = provider_registry::build_startup_provider_adapters(&data_root, &agent_cfg);

    let bound = listener::bind_daemon_listeners(bind).await?;
    let daemon_url = bound.daemon_url.clone();
    let requested_binds = bound.requested_binds.clone();
    let listeners = bound.listeners;
    let public_base_url = listener::daemon_public_base_url_from_env()?;

    let mut auth = auth::load_or_init_daemon_auth(&data_root)?;
    let auth_token = Some(auth.token.clone());

    auth.daemon_url = Some(daemon_url.clone());
    auth::write_daemon_auth_file(&auth::daemon_auth_path(&data_root), &auth)?;

    let state = Arc::new(AppState::new_with_public_base_url(
        data_root,
        stores,
        providers,
        daemon_url.clone(),
        public_base_url,
        auth_token,
    ));
    state.transport.web_sessions.clone().start_reaper().await;
    state.transport.terminals.clone().start_reaper().await;
    lifecycle::spawn_cache_sweeper(state.clone());
    lifecycle::spawn_provider_worker_sweeper(state.clone());
    lifecycle::spawn_endpoint_model_catalog_sweeper(state.clone());
    if let Err(err) = reconcile_running_turns(&state).await {
        tracing::warn!(err = %err, "failed to reconcile running turns on startup");
    }
    let settings = ctx_settings_service::load_settings(state.global_store()).await?;
    let mut telemetry_cfg = TelemetryConfig::default();
    if let Some(telemetry) = settings.telemetry.as_ref() {
        telemetry_cfg.enabled = telemetry.enabled;
        if !telemetry.endpoint.trim().is_empty() {
            telemetry_cfg.endpoint = telemetry.endpoint.clone();
        }
    }
    state.telemetry.telemetry.update_config(telemetry_cfg).await;
    let perf_enabled = settings
        .telemetry
        .as_ref()
        .map(|t| t.enabled)
        .unwrap_or(true);
    state
        .telemetry
        .perf_telemetry
        .update_remote_enabled(perf_enabled)
        .await;
    if let Err(err) = resource_governance::apply_settings(&state, &settings).await {
        tracing::warn!("failed to apply resource governance settings: {err:#}");
    }
    if let Err(err) = provider_guard::apply_settings(&state, &settings).await {
        tracing::warn!("failed to apply provider guard settings: {err:#}");
    }
    if let Err(err) = provider_restart::apply_settings(&state, &settings).await {
        tracing::warn!("failed to apply provider restart settings: {err:#}");
    }

    if let Err(err) = tool_cgroup::apply_settings(&state, &settings).await {
        tracing::warn!("failed to apply tool cgroup settings: {err:#}");
    }

    resource_telemetry::spawn_resource_telemetry(state.clone());
    memleak_debug::spawn_memleak_debug(state.clone());
    storage_guard::spawn_storage_guard(state.clone());
    provider_guard::spawn_provider_guard(state.clone());
    provider_restart::spawn_provider_restart(state.clone());
    provider_child_reclassifier::spawn_provider_child_reclassifier(state.clone());
    merge_queue::spawn_merge_queue_runner(state.clone());
    provider_usage::spawn_provider_usage_poller(state.clone());
    managed_auto_update::spawn_managed_daemon_auto_update(state.clone(), requested_binds.clone());
    lifecycle::spawn_process_shutdown_listener(state.clone());

    mobile_startup::spawn_saved_mobile_tunnel_reconnect(state.clone());

    spawn_startup_provider_status_refresh(state.clone());
    let app: Router = api::router(state.clone());

    let bound_addrs = listeners
        .iter()
        .filter_map(|listener| listener.local_addr().ok())
        .map(|addr| addr.to_string())
        .collect::<Vec<_>>();
    tracing::info!("ctx daemon listening on {daemon_url} (binds={bound_addrs:?})");
    println!("{}", json!({"event":"listening","url": daemon_url}));
    let mut servers = tokio::task::JoinSet::new();
    for listener in listeners {
        let app = app.clone();
        let mut shutdown_rx = state.core.shutdown_tx.subscribe();
        servers.spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.recv().await;
                })
                .await
        });
    }
    while let Some(result) = servers.join_next().await {
        result.context("daemon listener task panicked")??;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
