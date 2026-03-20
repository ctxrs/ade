use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use async_trait::async_trait;
use axum::Router;
use chrono::Utc;
use directories::BaseDirs;
use serde_json::json;

use ctx_core::models::SessionTurnStatus;
use ctx_lsp::LspManagerConfig;
use ctx_providers::adapters::{
    ProviderAdapter, ProviderHealth, ProviderStatus, RunHandle, TurnInput,
};
use ctx_providers::crp::Tier1CrpAdapter;
use ctx_providers::fake::FakeProviderAdapter;
use ctx_store::{Store, StoreManager, StoreManagerConfig};

use crate::api;
use crate::installer;
use crate::installs::InstallTarget;
use crate::memleak_debug;
use crate::provider_child_reclassifier;
use crate::provider_guard;
use crate::provider_restart;
use crate::provider_usage;
use crate::resource_governance;
use crate::resource_telemetry;
use crate::scheduler::reconcile_turn_terminal_state;
use crate::settings;
use crate::storage_guard;
use crate::telemetry::TelemetryConfig;
use crate::tool_cgroup;

mod auth;
mod edit_plans;
mod sessions;
mod state;
mod workspaces;

pub(crate) use state::AttachmentMaterializationTask;
pub use state::{
    AppState, CacheSweepConfig, CacheSweepStats, CachedFileCompletions, CachedProviderOptions,
    CachedProviderVerify, GitStatusSnapshotCacheEntry, SessionHeadCacheKey, TimedEntry,
    WorkspaceActiveHeadCacheEntry, WorkspaceActiveSnapshotCacheEntry,
    WorktreeVcsSnapshotCacheEntry,
};

struct StaticStatusAdapter {
    status: ProviderStatus,
}

#[async_trait]
impl ProviderAdapter for StaticStatusAdapter {
    async fn inspect(&self) -> Result<ProviderStatus> {
        Ok(self.status.clone())
    }

    async fn run(
        &self,
        _input: TurnInput,
        _workdir: PathBuf,
        _env: HashMap<String, String>,
        _event_sink: tokio::sync::mpsc::Sender<ctx_providers::events::NormalizedEvent>,
    ) -> Result<RunHandle> {
        let msg = self
            .status
            .diagnostics
            .first()
            .cloned()
            .unwrap_or_else(|| "provider is unavailable".to_string());
        anyhow::bail!("{msg}");
    }

    async fn cancel(&self, _handle: RunHandle) -> Result<()> {
        Ok(())
    }
}

fn static_status_adapter(
    provider_id: &str,
    installed: bool,
    health: ProviderHealth,
    error_code: &str,
    message: String,
) -> Arc<dyn ProviderAdapter> {
    let mut details = HashMap::new();
    details.insert("error_code".to_string(), error_code.to_string());
    Arc::new(StaticStatusAdapter {
        status: ProviderStatus {
            provider_id: provider_id.to_string(),
            installed,
            detected_path: None,
            version: None,
            capabilities: None,
            health,
            diagnostics: vec![message],
            details,
            usability: ctx_providers::adapters::ProviderUsability::default(),
        },
    })
}

fn runtime_command_as_agent_command_for_target(
    cfg: &installer::AgentServerConfigFile,
    provider_id: &str,
    requested_target: Option<InstallTarget>,
) -> Result<Option<installer::AgentServerCommand>> {
    let Some(resolved) =
        installer::resolve_runtime_provider_command_for_target(cfg, provider_id, requested_target)?
    else {
        return Ok(None);
    };
    Ok(Some(installer::AgentServerCommand {
        command: resolved.command_abs_path,
        args: resolved.args,
        dependencies: resolved.dependencies,
        managed: None,
    }))
}

fn runtime_command_as_agent_command(
    cfg: &installer::AgentServerConfigFile,
    provider_id: &str,
) -> Result<Option<installer::AgentServerCommand>> {
    runtime_command_as_agent_command_for_target(cfg, provider_id, None)
}

fn acp_status_adapter_bridge_missing(provider_id: &str, msg: String) -> Arc<dyn ProviderAdapter> {
    static_status_adapter(
        provider_id,
        false,
        ProviderHealth::Error,
        "acp_bridge_missing",
        msg,
    )
}

fn acp_status_adapter_bridge_invalid(provider_id: &str, msg: String) -> Arc<dyn ProviderAdapter> {
    static_status_adapter(
        provider_id,
        false,
        ProviderHealth::Error,
        "acp_bridge_invalid",
        msg,
    )
}

fn acp_status_adapter_acp_command_invalid(
    provider_id: &str,
    msg: String,
) -> Arc<dyn ProviderAdapter> {
    static_status_adapter(
        provider_id,
        true,
        ProviderHealth::Error,
        "acp_command_invalid",
        msg,
    )
}

fn runtime_command_missing_adapter(provider_id: &str) -> Arc<dyn ProviderAdapter> {
    static_status_adapter(
        provider_id,
        false,
        ProviderHealth::Missing,
        "runtime_command_missing",
        format!("runtime command is not configured for provider '{provider_id}'"),
    )
}

fn runtime_command_invalid_adapter(provider_id: &str, err: String) -> Arc<dyn ProviderAdapter> {
    static_status_adapter(
        provider_id,
        false,
        ProviderHealth::Error,
        "runtime_command_invalid",
        format!("invalid runtime command for provider '{provider_id}': {err}"),
    )
}

pub(crate) fn is_acp_provider_id(provider_id: &str) -> bool {
    crate::provider_launch::resolver::is_acp_provider_id(provider_id)
}

pub(crate) fn acp_bridge_command(
    bridge_cmd: &installer::AgentServerCommand,
    acp_cmd: installer::AgentServerCommand,
) -> installer::AgentServerCommand {
    crate::provider_launch::resolver::acp_bridge_command(bridge_cmd, acp_cmd)
}

pub(crate) fn acp_bridge_adapter(
    id: &str,
    bridge_cmd: &installer::AgentServerCommand,
    acp_cmd: installer::AgentServerCommand,
) -> Arc<dyn ProviderAdapter> {
    crate::provider_launch::resolver::acp_bridge_adapter(id, bridge_cmd, acp_cmd)
}

#[cfg(test)]
pub(crate) fn runtime_probe_command_as_agent_command_for_target(
    data_root: &Path,
    cfg: &installer::AgentServerConfigFile,
    provider_id: &str,
    requested_target: Option<InstallTarget>,
) -> Result<Option<installer::AgentServerCommand>> {
    crate::provider_launch::resolver::runtime_probe_command_as_agent_command_for_target(
        data_root,
        cfg,
        provider_id,
        requested_target,
    )
}

#[cfg(test)]
pub(crate) fn runtime_probe_command_as_agent_command(
    data_root: &Path,
    cfg: &installer::AgentServerConfigFile,
    provider_id: &str,
) -> Result<Option<installer::AgentServerCommand>> {
    runtime_probe_command_as_agent_command_for_target(data_root, cfg, provider_id, None)
}

fn target_adapter_cache_key(provider_id: &str, target: InstallTarget) -> Option<String> {
    (!matches!(target, InstallTarget::Host)).then(|| format!("{provider_id}@{}", target.as_str()))
}

fn build_provider_adapter_for_target(
    data_root: &Path,
    cfg: &installer::AgentServerConfigFile,
    provider_id: &str,
    target: InstallTarget,
) -> Arc<dyn ProviderAdapter> {
    if matches!(provider_id, "codex" | "claude-crp") {
        return match runtime_command_as_agent_command_for_target(cfg, provider_id, Some(target)) {
            Ok(Some(cmd)) => Arc::new(Tier1CrpAdapter::from_raw(
                provider_id,
                cmd.command.clone(),
                cmd.args.clone(),
            )),
            Ok(None) => runtime_command_missing_adapter(provider_id),
            Err(err) => runtime_command_invalid_adapter(provider_id, err.to_string()),
        };
    }

    if is_acp_provider_id(provider_id) {
        let bridge_cmd = match runtime_command_as_agent_command_for_target(
            cfg,
            "acp-crp-bridge",
            Some(target),
        ) {
            Ok(cmd) => cmd,
            Err(err) => {
                return acp_status_adapter_bridge_invalid(
                    provider_id,
                    format!("invalid runtime command for acp-crp-bridge: {err}"),
                );
            }
        };
        let bridge_missing_message = "ACP bridge runtime is not configured".to_string();
        return match bridge_cmd.as_ref() {
            None => acp_status_adapter_bridge_missing(provider_id, bridge_missing_message),
            Some(bridge) => {
                match runtime_command_as_agent_command_for_target(cfg, provider_id, Some(target)) {
                    Ok(Some(cmd)) => {
                        match normalize_acp_provider_command(data_root, provider_id, cmd) {
                            Ok(cmd) => acp_bridge_adapter(provider_id, bridge, cmd),
                            Err(err) => acp_status_adapter_acp_command_invalid(
                                provider_id,
                                format!("invalid ACP command for provider '{provider_id}': {err}"),
                            ),
                        }
                    }
                    Ok(None) => acp_status_adapter_acp_command_invalid(
                        provider_id,
                        format!("ACP command is not configured for provider '{provider_id}'"),
                    ),
                    Err(err) => acp_status_adapter_acp_command_invalid(
                        provider_id,
                        format!("invalid ACP command for provider '{provider_id}': {err}"),
                    ),
                }
            }
        };
    }

    if provider_id == "fake" {
        return Arc::new(FakeProviderAdapter::new());
    }

    runtime_command_missing_adapter(provider_id)
}

pub(crate) async fn ensure_provider_adapter_for_target_with_cfg(
    state: &AppState,
    cfg: &installer::AgentServerConfigFile,
    provider_id: &str,
    target: InstallTarget,
) -> Arc<dyn ProviderAdapter> {
    if let Some(cache_key) = target_adapter_cache_key(provider_id, target) {
        if let Some(adapter) = state
            .providers
            .target_adapters
            .lock()
            .await
            .get(&cache_key)
            .cloned()
        {
            return adapter;
        }
        let adapter =
            build_provider_adapter_for_target(&state.core.data_root, cfg, provider_id, target);
        state
            .providers
            .target_adapters
            .lock()
            .await
            .insert(cache_key, adapter.clone());
        return adapter;
    }

    if let Some(adapter) = state
        .providers
        .adapters
        .lock()
        .await
        .get(provider_id)
        .cloned()
    {
        return adapter;
    }
    let adapter =
        build_provider_adapter_for_target(&state.core.data_root, cfg, provider_id, target);
    state
        .providers
        .adapters
        .lock()
        .await
        .insert(provider_id.to_string(), adapter.clone());
    adapter
}

pub(crate) async fn ensure_provider_adapter_for_target(
    state: &AppState,
    provider_id: &str,
    target: InstallTarget,
) -> Arc<dyn ProviderAdapter> {
    let cfg = installer::load_agent_server_config(&state.core.data_root)
        .await
        .unwrap_or_default();
    ensure_provider_adapter_for_target_with_cfg(state, &cfg, provider_id, target).await
}

pub(crate) fn normalize_acp_provider_command(
    data_root: &Path,
    provider_id: &str,
    cmd: installer::AgentServerCommand,
) -> Result<installer::AgentServerCommand> {
    crate::provider_launch::resolver::normalize_acp_provider_command(data_root, provider_id, cmd)
}

async fn reconcile_running_turns(state: &Arc<AppState>) -> Result<()> {
    let workspaces = state.global_store().list_workspaces().await?;
    let mut running_turns = Vec::new();
    for workspace in workspaces {
        let store = state.core.stores.workspace_uncached(workspace.id).await?;
        let mut turns = store
            .list_session_turns_by_statuses(&[SessionTurnStatus::Running])
            .await?;
        store.close().await;
        running_turns.append(&mut turns);
    }

    for turn in running_turns {
        if let Err(err) = reconcile_turn_terminal_state(
            state,
            turn.session_id,
            turn.run_id,
            turn.turn_id,
            "daemon_restart",
        )
        .await
        {
            tracing::warn!(
                session_id = %turn.session_id.0,
                turn_id = %turn.turn_id.0,
                err = %err,
                "failed to reconcile running turn after daemon restart"
            );
        }
    }

    Ok(())
}

async fn prune_archived_session_data_for_all_workspaces(
    stores: &StoreManager,
    retention_days: u64,
) -> Result<()> {
    let workspaces = stores.global().list_workspaces().await?;
    for workspace in workspaces {
        match stores.workspace_uncached(workspace.id).await {
            Ok(store) => {
                let prune_result = store
                    .prune_session_data_older_than_days(retention_days)
                    .await;
                store.close().await;
                match prune_result {
                    Ok(stats) => {
                        tracing::info!(
                            workspace_id = %workspace.id.0,
                            tool_summaries_deleted = stats.tool_summaries_deleted,
                            turn_thoughts_cleared = stats.turn_thoughts_cleared,
                            retention_days,
                            "pruned archived session data",
                        );
                    }
                    Err(err) => {
                        tracing::warn!(
                            workspace_id = %workspace.id.0,
                            retention_days,
                            "failed to prune old session data: {err:#}",
                        );
                    }
                }
            }
            Err(err) => {
                tracing::warn!(
                    workspace_id = %workspace.id.0,
                    "failed to open workspace store for pruning: {err:#}",
                );
            }
        }
    }
    Ok(())
}

fn spawn_cache_sweeper(state: Arc<AppState>) {
    let config = CacheSweepConfig::from_env();
    tokio::spawn(async move {
        let mut shutdown_rx = state.core.shutdown_tx.subscribe();
        loop {
            tokio::select! {
                _ = tokio::time::sleep(config.interval) => {
                    let stats = state.sweep_idle_caches(Instant::now(), config).await;
                    if stats.total_evicted() > 0 {
                        tracing::info!(
                            session_head_evicted = stats.session_head_evicted,
                            session_meta_evicted = stats.session_meta_evicted,
                            schedulers_evicted = stats.schedulers_evicted,
                            broadcasters_evicted = stats.broadcasters_evicted,
                            session_event_heads_evicted = stats.session_event_heads_evicted,
                            file_completions_evicted = stats.file_completions_evicted,
                            workspace_file_completions_evicted =
                                stats.workspace_file_completions_evicted,
                            git_status_evicted = stats.git_status_evicted,
                            workspace_snapshot_evicted = stats.workspace_snapshot_evicted,
                            workspace_heads_evicted = stats.workspace_heads_evicted,
                            worktree_bootstrap_evicted = stats.worktree_bootstrap_evicted,
                            workspace_stores_evicted = stats.workspace_stores_evicted,
                            "cache sweep completed"
                        );
                    }
                }
                _ = shutdown_rx.recv() => {
                    break;
                }
            }
        }
    });
}

const DEFAULT_ENDPOINT_MODEL_SWEEP_INTERVAL: Duration = Duration::from_secs(60 * 60 * 6);

fn endpoint_model_sweep_interval() -> Duration {
    std::env::var("CTX_ENDPOINT_MODEL_SWEEP_SECS")
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|secs| *secs > 0)
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_ENDPOINT_MODEL_SWEEP_INTERVAL)
}

async fn refresh_stale_selected_endpoint_model_catalogs(
    state: &Arc<AppState>,
) -> (usize, usize, HashSet<String>) {
    let provider_ids = {
        let statuses = state.providers.statuses.lock().await;
        statuses.keys().cloned().collect::<Vec<_>>()
    };
    let now = Utc::now();
    let mut refreshed = 0usize;
    let mut failed = 0usize;
    let mut refreshed_provider_ids = HashSet::new();

    for provider_id in provider_ids {
        let Ok(config) =
            crate::harness_sources::get_provider_source_config(&state.core.data_root, &provider_id)
                .await
        else {
            continue;
        };
        if config.selected_source_kind != crate::harness_sources::HarnessSourceKind::Endpoint {
            continue;
        }
        let Some(selected_endpoint_id) = config.selected_endpoint_id.as_deref() else {
            continue;
        };
        let Some(endpoint) = config
            .endpoints
            .iter()
            .find(|candidate| candidate.id == selected_endpoint_id)
        else {
            continue;
        };
        if !crate::harness_sources::endpoint_model_catalog_is_stale(endpoint, now) {
            continue;
        }

        match crate::harness_sources::refresh_provider_endpoint_model_catalog(
            &state.core.data_root,
            &provider_id,
            selected_endpoint_id,
        )
        .await
        {
            Ok(_) => {
                refreshed += 1;
                refreshed_provider_ids.insert(provider_id);
            }
            Err(err) => {
                failed += 1;
                tracing::warn!(
                    provider_id = provider_id,
                    endpoint_id = selected_endpoint_id,
                    err = %err,
                    "endpoint model catalog refresh failed"
                );
            }
        }
    }

    (refreshed, failed, refreshed_provider_ids)
}

fn spawn_endpoint_model_catalog_sweeper(state: Arc<AppState>) {
    let interval = endpoint_model_sweep_interval();
    tokio::spawn(async move {
        let mut shutdown_rx = state.core.shutdown_tx.subscribe();
        loop {
            tokio::select! {
                _ = tokio::time::sleep(interval) => {
                    let (refreshed, failed, refreshed_provider_ids) =
                        refresh_stale_selected_endpoint_model_catalogs(&state).await;

                    if !refreshed_provider_ids.is_empty() {
                        state
                            .providers
                            .options_cache
                            .lock()
                            .await
                            .retain(|cache_key, _| {
                                !refreshed_provider_ids
                                    .iter()
                                    .any(|provider_id| cache_key_matches_provider(cache_key, provider_id))
                            });
                    }

                    if refreshed > 0 || failed > 0 {
                        tracing::info!(
                            refreshed_endpoint_catalogs = refreshed,
                            failed_endpoint_catalog_refreshes = failed,
                            "endpoint model catalog sweep completed"
                        );
                    }
                }
                _ = shutdown_rx.recv() => {
                    break;
                }
            }
        }
    });
}

fn cache_key_matches_provider(cache_key: &str, provider_id: &str) -> bool {
    cache_key
        .rsplit_once('/')
        .is_some_and(|(_, key_provider)| key_provider == provider_id)
}

pub async fn serve(bind: String, data_dir: Option<String>) -> Result<()> {
    let data_root = match data_dir {
        Some(p) => PathBuf::from(p),
        None => {
            let base = BaseDirs::new().context("resolving home dir")?;
            base.home_dir().join(".ctx")
        }
    };
    tokio::fs::create_dir_all(&data_root).await?;
    // Canonicalize so Podman machine mount sources resolve under shared roots on macOS
    // (e.g. /tmp -> /private/tmp). This also reduces accidental duplicate state roots.
    let data_root = tokio::fs::canonicalize(&data_root)
        .await
        .unwrap_or(data_root);
    tokio::fs::create_dir_all(data_root.join("logs")).await.ok();

    let _daemon_lock = auth::acquire_daemon_lock(&data_root)?;

    let global_db_path = data_root.join("db").join("db.sqlite");
    let bootstrap_store = Store::open_sqlite(&global_db_path, None).await?;
    let settings_data = settings::load_settings(&bootstrap_store).await?;
    bootstrap_store.close().await;
    let store_config = settings_data
        .storage
        .as_ref()
        .map(|storage| StoreManagerConfig {
            max_connections: storage.max_connections,
        })
        .unwrap_or_default();
    let stores = StoreManager::open_with_config(&data_root, store_config).await?;

    // Retention policy (configurable via env):
    // - Keep tool summaries and final thoughts for archived tasks for N days.
    // - Do not retain thought chunk events (handled at ingestion time).
    const DEFAULT_TOOL_SUMMARY_RETENTION_DAYS: u64 = 30;
    fn tool_summary_retention_days() -> u64 {
        std::env::var("CTX_TOOL_SUMMARY_RETENTION_DAYS")
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .unwrap_or(DEFAULT_TOOL_SUMMARY_RETENTION_DAYS)
    }
    {
        let stores = stores.clone();
        tokio::spawn(async move {
            let mut last_cleanup = None::<String>;
            loop {
                let today = Utc::now().format("%Y-%m-%d").to_string();
                if last_cleanup.as_deref() != Some(&today) {
                    let retention_days = tool_summary_retention_days();
                    if let Err(err) =
                        prune_archived_session_data_for_all_workspaces(&stores, retention_days)
                            .await
                    {
                        tracing::warn!(
                            retention_days,
                            "failed to list workspaces for pruning: {err:#}",
                        );
                    }
                    last_cleanup = Some(today);
                }
                tokio::time::sleep(Duration::from_secs(60 * 60)).await;
            }
        });
    }

    let agent_cfg = installer::load_agent_server_config(&data_root)
        .await
        .unwrap_or_default();

    let mut bridge_runtime_error: Option<String> = None;
    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    let bridge_cmd = match runtime_command_as_agent_command(&agent_cfg, "acp-crp-bridge") {
        Ok(cmd) => cmd,
        Err(err) => {
            let message = format!("invalid runtime command for acp-crp-bridge: {err}");
            tracing::warn!("{message}");
            bridge_runtime_error = Some(message);
            None
        }
    };

    for provider_id in ["codex", "claude-crp"] {
        let adapter: Arc<dyn ProviderAdapter> =
            match runtime_command_as_agent_command(&agent_cfg, provider_id) {
                Ok(Some(cmd)) => Arc::new(Tier1CrpAdapter::from_raw(
                    provider_id,
                    cmd.command.clone(),
                    cmd.args.clone(),
                )),
                Ok(None) => runtime_command_missing_adapter(provider_id),
                Err(err) => runtime_command_invalid_adapter(provider_id, err.to_string()),
            };
        providers.insert(provider_id.to_string(), adapter);
    }

    for provider_id in [
        "gemini",
        "qwen",
        "cursor",
        "pi",
        "opencode",
        "mistral",
        "goose",
        "kimi",
        "auggie",
        "amp",
        "droid",
        "copilot",
        "cline",
        "openhands",
    ] {
        let bridge_missing_message = bridge_runtime_error
            .clone()
            .unwrap_or_else(|| "ACP bridge runtime is not configured".to_string());
        let adapter = match bridge_cmd.as_ref() {
            None => {
                if let Some(message) = bridge_runtime_error.clone() {
                    acp_status_adapter_bridge_invalid(provider_id, message)
                } else {
                    acp_status_adapter_bridge_missing(provider_id, bridge_missing_message)
                }
            }
            Some(bridge) => match runtime_command_as_agent_command(&agent_cfg, provider_id) {
                Ok(Some(cmd)) => {
                    match normalize_acp_provider_command(&data_root, provider_id, cmd) {
                        Ok(cmd) => acp_bridge_adapter(provider_id, bridge, cmd),
                        Err(err) => acp_status_adapter_acp_command_invalid(
                            provider_id,
                            format!("invalid ACP command for provider '{provider_id}': {err}"),
                        ),
                    }
                }
                Ok(None) => acp_status_adapter_acp_command_invalid(
                    provider_id,
                    format!("ACP command is not configured for provider '{provider_id}'"),
                ),
                Err(err) => acp_status_adapter_acp_command_invalid(
                    provider_id,
                    format!("invalid ACP command for provider '{provider_id}': {err}"),
                ),
            },
        };
        providers.insert(provider_id.to_string(), adapter);
    }

    // codex/claude CRP adapters are always registered now (legacy ACP bridge removed).

    if std::env::var("CTX_SHOW_FAKE_PROVIDER")
        .ok()
        .as_deref()
        .and_then(ctx_core::boolish::parse_boolish)
        .unwrap_or(false)
    {
        providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));
    }

    let listener = tokio::net::TcpListener::bind(&bind).await?;
    let local_addr = listener.local_addr()?;
    let host = match local_addr.ip() {
        std::net::IpAddr::V4(ip) if ip.octets() == [0, 0, 0, 0] => "127.0.0.1".to_string(),
        std::net::IpAddr::V6(ip) if ip.is_unspecified() => "::1".to_string(),
        ip => ip.to_string(),
    };
    let daemon_url = format!("http://{}:{}", host, local_addr.port());

    let mut auth = auth::load_or_init_daemon_auth(&data_root)?;
    let auth_token = Some(auth.token.clone());

    let mut lsp_cfg = LspManagerConfig::default();
    let _ = installer::apply_managed_lsp_server_config(&data_root, &mut lsp_cfg).await;
    let _ = installer::apply_user_lsp_server_config(&data_root, &mut lsp_cfg).await;
    auth.daemon_url = Some(daemon_url.clone());
    auth::write_daemon_auth_file(&auth::daemon_auth_path(&data_root), &auth)?;

    let state = Arc::new(AppState::new_with_lsp_config(
        data_root,
        stores,
        providers,
        daemon_url.clone(),
        auth_token,
        lsp_cfg,
    ));
    state.transport.web_sessions.clone().start_reaper().await;
    state.transport.terminals.clone().start_reaper().await;
    spawn_cache_sweeper(state.clone());
    spawn_endpoint_model_catalog_sweeper(state.clone());
    if let Err(err) = reconcile_running_turns(&state).await {
        tracing::warn!(err = %err, "failed to reconcile running turns on startup");
    }
    let settings = settings::load_settings(state.global_store()).await?;
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
    crate::merge_queue::spawn_merge_queue_runner(state.clone());
    provider_usage::spawn_provider_usage_poller(state.clone());

    // Reconnect managed mobile access tunnel on daemon start when enabled.
    {
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            if state.core.auth_token.is_none() {
                return;
            }
            let cfg = match state.global_store().get_mobile_access_config().await {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!("failed to read saved mobile access config: {e:#}");
                    return;
                }
            };
            let Some(cfg) = cfg else {
                return;
            };
            if !cfg.enabled {
                return;
            }

            let start_cfg = crate::mobile_tunnel::StartMobileTunnelConfig {
                relay_base_url: cfg.relay_base_url,
                tunnel_id: cfg.tunnel_id,
                tunnel_secret: cfg.tunnel_secret,
                public_base_url: cfg.public_base_url.trim_end_matches('/').to_string(),
                local_daemon_url: state.core.daemon_url.trim_end_matches('/').to_string(),
            };
            if let Err(e) = state.transport.mobile_tunnel.start(start_cfg).await {
                tracing::warn!("failed to start saved mobile tunnel: {e:#}");
            }
        });
    }

    installer::refresh_provider_statuses(&state).await?;
    let mut shutdown_rx = state.core.shutdown_tx.subscribe();
    let app: Router = api::router(state);

    tracing::info!("ctx daemon listening on {daemon_url}");
    println!("{}", json!({"event":"listening","url": daemon_url}));
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = shutdown_rx.recv().await;
        })
        .await?;
    Ok(())
}

pub async fn init_workspace(root: Option<String>) -> Result<()> {
    let root_path = root
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir().context("getting current dir")?);
    let vcs = ctx_fs::vcs::driver_for_path(&root_path).await?;
    vcs.assert_repo(&root_path).await?;

    let context_dir = root_path.join(".ctx");
    let pack_dir = context_dir.join("ctx-pack");
    let tmp_dir = pack_dir.join("tmp");

    tokio::fs::create_dir_all(pack_dir.join("specs")).await?;
    tokio::fs::create_dir_all(pack_dir.join("prompts")).await?;
    tokio::fs::create_dir_all(pack_dir.join("docs")).await?;
    tokio::fs::create_dir_all(pack_dir.join("skills")).await?;
    tokio::fs::create_dir_all(&tmp_dir).await?;

    tokio::fs::create_dir_all(context_dir.join("exec-plans")).await?;

    let gitignore_path = root_path.join(".gitignore");
    let ignore_line = ".ctx/ctx-pack/tmp/";
    let mut gitignore = if gitignore_path.exists() {
        tokio::fs::read_to_string(&gitignore_path).await?
    } else {
        String::new()
    };
    if !gitignore.lines().any(|l| l.trim() == ignore_line) {
        if !gitignore.ends_with('\n') && !gitignore.is_empty() {
            gitignore.push('\n');
        }
        gitignore.push_str(ignore_line);
        gitignore.push('\n');
        tokio::fs::write(&gitignore_path, gitignore).await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests;
