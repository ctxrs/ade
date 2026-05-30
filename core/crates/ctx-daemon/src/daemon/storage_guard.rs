use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;

use ctx_observability::ops_events::OpsEvents;
use ctx_resource_utilization::ResourceSampler;
use ctx_session_runtime::runtime::SessionRuntime;
#[cfg(test)]
use ctx_storage_admission::storage_emergency_message;
use ctx_storage_admission::{
    StorageGuardRuntime, StorageGuardStatus, STORAGE_GUARD_MONITOR_INTERVAL,
};
use tokio::sync::{broadcast, Mutex};

use crate::daemon::scheduler::SchedulerCommand;
use crate::daemon::SessionStoreLookup;

#[cfg(test)]
const GIB: u64 = ctx_storage_admission::STORAGE_BYTES_GIB;
#[cfg(test)]
const MIB: u64 = ctx_storage_admission::STORAGE_BYTES_MIB;

mod observations;
mod publication;

use observations::{collect_observed_paths, sample_storage_disks};
#[cfg(test)]
use publication::dispatch_storage_emergency_interrupt;
use publication::{emit_reserve_warnings, publish_storage_guard_snapshot};

#[derive(Clone)]
pub(in crate::daemon) struct StorageGuardHost {
    data_root: PathBuf,
    storage_guard: Arc<StorageGuardRuntime>,
    resource_sampler: Arc<Mutex<ResourceSampler>>,
    sessions: Arc<SessionRuntime<SchedulerCommand>>,
    session_stores: SessionStoreLookup,
    ops_events: OpsEvents,
    shutdown_tx: broadcast::Sender<()>,
}

pub(in crate::daemon) struct StorageGuardHostParts {
    pub(in crate::daemon) data_root: PathBuf,
    pub(in crate::daemon) storage_guard: Arc<StorageGuardRuntime>,
    pub(in crate::daemon) resource_sampler: Arc<Mutex<ResourceSampler>>,
    pub(in crate::daemon) sessions: Arc<SessionRuntime<SchedulerCommand>>,
    pub(in crate::daemon) session_stores: SessionStoreLookup,
    pub(in crate::daemon) ops_events: OpsEvents,
    pub(in crate::daemon) shutdown_tx: broadcast::Sender<()>,
}

impl StorageGuardHost {
    pub(in crate::daemon) fn new(parts: StorageGuardHostParts) -> Self {
        Self {
            data_root: parts.data_root,
            storage_guard: parts.storage_guard,
            resource_sampler: parts.resource_sampler,
            sessions: parts.sessions,
            session_stores: parts.session_stores,
            ops_events: parts.ops_events,
            shutdown_tx: parts.shutdown_tx,
        }
    }

    pub(in crate::daemon) fn data_root(&self) -> &Path {
        &self.data_root
    }

    #[cfg(test)]
    pub(in crate::daemon) fn storage_guard_snapshot(&self) -> StorageGuardStatus {
        self.storage_guard.snapshot()
    }
}

pub(in crate::daemon) fn spawn_storage_guard(host: StorageGuardHost) {
    let mut shutdown_rx = host.shutdown_tx.subscribe();
    tokio::spawn(async move {
        if let Err(err) = evaluate_storage_guard(&host, &[]).await {
            tracing::warn!("storage guard initial evaluation failed: {err:#}");
        }

        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => break,
                _ = tokio::time::sleep(STORAGE_GUARD_MONITOR_INTERVAL) => {
                    if let Err(err) = evaluate_storage_guard(&host, &[]).await {
                        tracing::warn!("storage guard tick failed: {err:#}");
                    }
                }
            }
        }
    });
}

#[cfg(test)]
async fn preflight_turn_start(host: &StorageGuardHost, workdir: &Path) -> Result<()> {
    let current = host.storage_guard_snapshot();
    if current.is_emergency() {
        anyhow::bail!(storage_emergency_message(current.active.as_ref()));
    }
    let snapshot = refresh_preflight_storage_guard(&host, &[workdir.to_path_buf()]).await;
    if snapshot.is_emergency() {
        anyhow::bail!(storage_emergency_message(snapshot.active.as_ref()));
    }
    Ok(())
}

async fn evaluate_storage_guard(
    host: &StorageGuardHost,
    extra_paths: &[PathBuf],
) -> Result<StorageGuardStatus> {
    let observed_paths = collect_observed_paths(host, extra_paths).await;
    let disks = sample_storage_disks(host).await;
    let (previous, snapshot, warnings) = host
        .storage_guard
        .evaluate(host.data_root(), &observed_paths, &disks)
        .await;
    emit_reserve_warnings(warnings);

    publish_storage_guard_snapshot(host, &previous, &snapshot).await;
    Ok(snapshot)
}

#[cfg(test)]
async fn refresh_preflight_storage_guard(
    host: &StorageGuardHost,
    extra_paths: &[PathBuf],
) -> StorageGuardStatus {
    let observed_paths = collect_observed_paths(host, extra_paths).await;
    let disks = sample_storage_disks(host).await;
    let (previous, snapshot) =
        host.storage_guard
            .sample_preflight(host.data_root(), &observed_paths, &disks);
    publish_storage_guard_snapshot(host, &previous, &snapshot).await;
    snapshot
}

#[cfg(test)]
mod tests;
