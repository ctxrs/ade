use super::*;
use crate::api::shared::store_for_existing_workspace_status;
use ctx_resource_utilization as resource_utilization;

#[derive(Debug, Deserialize)]
pub(in crate::api) struct ResourceUtilizationQuery {
    workspace_id: String,
}

pub(in crate::api) async fn resource_utilization(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ResourceUtilizationQuery>,
) -> Result<Json<resource_utilization::ResourceUtilizationSnapshot>, StatusCode> {
    if resource_utilization_disabled() {
        return Err(StatusCode::NOT_FOUND);
    }
    let workspace_id = WorkspaceId(
        uuid::Uuid::parse_str(&query.workspace_id).map_err(|_| StatusCode::BAD_REQUEST)?,
    );
    let workspace = state
        .global_store()
        .get_workspace(workspace_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let store = store_for_existing_workspace_status(&state, workspace_id).await?;
    let worktrees = store
        .list_worktrees(workspace_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let provider_processes = state.providers.list_provider_processes().await;

    let (system, disks, cache_age_ms, processes, disk_cache) = {
        let mut sampler = state.telemetry.resource_sampler.lock().await;
        let (system, disks, cache_age_ms) = sampler.system_snapshot();
        let processes = sampler.processes_snapshot_light(std::process::id(), &provider_processes);
        let disk_cache = sampler.disk_cache_entry(workspace_id);
        (system, disks, cache_age_ms, processes, disk_cache)
    };

    let disk = resource_utilization::disk_for_path(StdPath::new(&workspace.root_path), &disks);

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
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
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

    Ok(Json(resource_utilization::ResourceUtilizationSnapshot {
        collected_at: chrono::Utc::now().to_rfc3339(),
        cache_age_ms,
        system,
        processes,
        workspace: workspace_snapshot,
    }))
}

pub(in crate::api) fn resource_utilization_disabled() -> bool {
    env_bool("CTX_RESOURCE_UTILIZATION_DISABLED").unwrap_or(true)
}

pub(in crate::api) fn env_bool(key: &str) -> Option<bool> {
    std::env::var(key)
        .ok()
        .as_deref()
        .and_then(ctx_core::boolish::parse_boolish)
}
