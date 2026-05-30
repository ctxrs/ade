use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use chrono::Utc;
use ctx_observability::perf_telemetry::PerfTelemetry;
use ctx_provider_runtime::ProviderRuntime;
use ctx_resource_utilization::memleak_debug::{
    append_memleak_debug_log, read_memleak_debug_process_stats, MemleakDebugConfig,
};
use ctx_store::StoreManager;
use ctx_transport_runtime::terminals::TerminalManager;
use ctx_transport_runtime::web_sessions::WebSessionManager;
use ctx_workspace_active_snapshot::WorkspaceActiveSnapshotHub;
use ctx_workspace_runtime::HarnessRuntimeManager;
use tokio::sync::broadcast;
use tokio::time::MissedTickBehavior;

use crate::daemon::state::SessionRuntime;
use crate::daemon::workspaces::WorkspaceCacheDebugStatsHost;
use ctx_observability::logs;

mod cache_stats;
mod snapshot;

use self::cache_stats::collect_memleak_debug_cache_stats;
use self::snapshot::MemleakDebugSnapshot;

#[derive(Clone)]
pub(in crate::daemon) struct MemleakDebugHost {
    data_root: PathBuf,
    shutdown_tx: broadcast::Sender<()>,
    sessions: Arc<SessionRuntime>,
    workspaces: WorkspaceCacheDebugStatsHost,
    providers: Arc<ProviderRuntime>,
    active_snapshot: Arc<WorkspaceActiveSnapshotHub>,
    terminals: Arc<TerminalManager>,
    perf_telemetry: PerfTelemetry,
    web_sessions: Arc<WebSessionManager>,
    harness_runtime: Arc<HarnessRuntimeManager>,
    stores: StoreManager,
}

pub(in crate::daemon) struct MemleakDebugHostParts {
    pub(in crate::daemon) data_root: PathBuf,
    pub(in crate::daemon) shutdown_tx: broadcast::Sender<()>,
    pub(in crate::daemon) sessions: Arc<SessionRuntime>,
    pub(in crate::daemon) workspaces: WorkspaceCacheDebugStatsHost,
    pub(in crate::daemon) providers: Arc<ProviderRuntime>,
    pub(in crate::daemon) active_snapshot: Arc<WorkspaceActiveSnapshotHub>,
    pub(in crate::daemon) terminals: Arc<TerminalManager>,
    pub(in crate::daemon) perf_telemetry: PerfTelemetry,
    pub(in crate::daemon) web_sessions: Arc<WebSessionManager>,
    pub(in crate::daemon) harness_runtime: Arc<HarnessRuntimeManager>,
    pub(in crate::daemon) stores: StoreManager,
}

impl MemleakDebugHost {
    pub(in crate::daemon) fn new(parts: MemleakDebugHostParts) -> Self {
        Self {
            data_root: parts.data_root,
            shutdown_tx: parts.shutdown_tx,
            sessions: parts.sessions,
            workspaces: parts.workspaces,
            providers: parts.providers,
            active_snapshot: parts.active_snapshot,
            terminals: parts.terminals,
            perf_telemetry: parts.perf_telemetry,
            web_sessions: parts.web_sessions,
            harness_runtime: parts.harness_runtime,
            stores: parts.stores,
        }
    }
}

pub(in crate::daemon) fn spawn_memleak_debug(host: MemleakDebugHost) {
    let config = MemleakDebugConfig::from_env();
    if !config.enabled {
        return;
    }
    let mut shutdown_rx = host.shutdown_tx.subscribe();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(config.interval);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => break,
                _ = ticker.tick() => {
                    if let Err(err) = sample_once(&host).await {
                        tracing::warn!("memleak debug sample failed: {err:#}");
                    }
                }
            }
        }
    });
}

async fn sample_once(host: &MemleakDebugHost) -> Result<()> {
    let cache_stats = collect_memleak_debug_cache_stats(host).await;
    let active_snapshot = host.active_snapshot.stats().await;
    let terminals = host.terminals.stats().await;
    let perf_telemetry = host.perf_telemetry.stats();
    let web_sessions = host.web_sessions.stats().await;
    let harness_runtime = host.harness_runtime.stats().await;
    let stores = host.stores.stats().await;
    let process = read_memleak_debug_process_stats();

    let snapshot = MemleakDebugSnapshot {
        occurred_at: Utc::now(),
        rss_bytes: process.rss_bytes,
        thread_count: process.thread_count,
        sessions: cache_stats.sessions,
        workspaces: cache_stats.workspaces,
        providers: cache_stats.providers,
        active_snapshot,
        terminals,
        perf_telemetry,
        web_sessions,
        harness_runtime,
        stores,
        glibc: process.glibc,
        jemalloc: process.jemalloc,
    };

    let logs_dir = logs::logs_dir(&host.data_root);
    append_memleak_debug_log(&logs_dir, &snapshot).await?;
    Ok(())
}
