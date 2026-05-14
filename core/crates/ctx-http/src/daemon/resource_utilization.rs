use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use ctx_core::ids::WorkspaceId;
use ctx_resource_utilization as resource_utilization;

use crate::daemon::{DaemonState, StoreLookup, WorkspacesHandle};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResourceUtilizationSnapshotError {
    Disabled,
    WorkspaceNotFound,
    Internal,
}

pub(crate) async fn workspace_resource_utilization_snapshot(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
) -> Result<resource_utilization::ResourceUtilizationSnapshot, ResourceUtilizationSnapshotError> {
    if resource_utilization::resource_utilization_disabled_from_env() {
        return Err(ResourceUtilizationSnapshotError::Disabled);
    }

    let workspace = state
        .global_store()
        .get_workspace(workspace_id)
        .await
        .map_err(|_| ResourceUtilizationSnapshotError::Internal)?
        .ok_or(ResourceUtilizationSnapshotError::WorkspaceNotFound)?;

    let store = match state.lookup_workspace_store(workspace_id).await {
        StoreLookup::Found(store) => store,
        StoreLookup::Missing | StoreLookup::Deleting => {
            return Err(ResourceUtilizationSnapshotError::WorkspaceNotFound);
        }
        StoreLookup::Unavailable(_) => return Err(ResourceUtilizationSnapshotError::Internal),
    };
    let worktrees = store
        .list_worktrees(workspace_id)
        .await
        .map_err(|_| ResourceUtilizationSnapshotError::Internal)?;

    let provider_processes = state.providers.list_provider_processes().await;

    let (system, disks, cache_age_ms, processes, disk_cache) = {
        let mut sampler = state.telemetry.resource_sampler.lock().await;
        let (system, disks, cache_age_ms) = sampler.system_snapshot();
        let processes = sampler.processes_snapshot_light(std::process::id(), &provider_processes);
        let disk_cache = sampler.disk_cache_entry(workspace_id);
        (system, disks, cache_age_ms, processes, disk_cache)
    };

    let disk = resource_utilization::disk_for_path(Path::new(&workspace.root_path), &disks);

    let now = Instant::now();
    let refresh_disk = resource_utilization::should_refresh_disk_cache(now, disk_cache.as_ref());
    let (mut workspace_snapshot, size_cache_age_ms) = if refresh_disk {
        let workspace_clone = workspace.clone();
        let worktrees_clone = worktrees.clone();
        let disk_clone = disk.clone();
        let snapshot = tokio::task::spawn_blocking(move || {
            resource_utilization::compute_workspace_disk_snapshot(
                workspace_clone,
                worktrees_clone,
                disk_clone,
                0,
            )
        })
        .await
        .map_err(|_| ResourceUtilizationSnapshotError::Internal)?;
        let mut sampler = state.telemetry.resource_sampler.lock().await;
        sampler.update_disk_cache(workspace_id, now, snapshot.clone());
        (snapshot, 0)
    } else {
        let age_ms = resource_utilization::disk_cache_age_ms(now, disk_cache.as_ref());
        let snapshot = disk_cache
            .as_ref()
            .map(|c| c.snapshot.clone())
            .unwrap_or_else(|| {
                resource_utilization::compute_workspace_disk_snapshot(
                    workspace.clone(),
                    worktrees.clone(),
                    disk.clone(),
                    age_ms,
                )
            });
        (snapshot, age_ms)
    };

    workspace_snapshot.disk = disk;
    workspace_snapshot.size_cache_age_ms = size_cache_age_ms;

    Ok(resource_utilization::ResourceUtilizationSnapshot {
        collected_at: chrono::Utc::now().to_rfc3339(),
        cache_age_ms,
        system,
        processes,
        workspace: workspace_snapshot,
    })
}

impl WorkspacesHandle {
    pub(crate) async fn workspace_resource_utilization_snapshot(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<resource_utilization::ResourceUtilizationSnapshot, ResourceUtilizationSnapshotError>
    {
        workspace_resource_utilization_snapshot(&self.state, workspace_id).await
    }
}
