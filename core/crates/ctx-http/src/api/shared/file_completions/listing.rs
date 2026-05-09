use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use axum::http::StatusCode;
use ctx_core::ids::WorkspaceId;
use ctx_core::models::{ExecutionEnvironment, Worktree};
use ctx_fs::git::{list_tracked_files, list_untracked_files};
use ctx_observability::perf_telemetry::{PerfMetric, PerfMetricKind};
use ctx_worktree_data_plane::resolve_worktree_data_plane_with_host as resolve_worktree_data_plane;

use crate::daemon::AppState;

use super::container::list_container_worktree_files;

pub(crate) async fn load_and_cache_worktree_files(
    state: &Arc<AppState>,
    worktree: &Worktree,
    execution_environment: ExecutionEnvironment,
    now: Instant,
) -> Result<Arc<Vec<String>>, StatusCode> {
    let started_at = Instant::now();
    let data_plane = resolve_worktree_data_plane(state.as_ref(), worktree)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let root = data_plane.live_worktree_root.clone();
    let files = if matches!(
        data_plane.execution_mode,
        ctx_settings_model::ExecutionMode::Sandbox
    ) {
        Arc::new(list_container_worktree_files(state, worktree, execution_environment).await?)
    } else {
        Arc::new(list_host_git_files(&root).await?)
    };

    let mut cache = state.workspaces.file_completions_cache.lock().await;
    cache.insert(
        worktree.id,
        crate::daemon::TimedEntry::new(crate::daemon::CachedFileCompletions {
            cached_at: now,
            files: files.clone(),
        }),
    );
    record_list_files_metric(state, "list_files_worktree", started_at).await;
    Ok(files)
}

pub(crate) async fn load_and_cache_workspace_files(
    state: &Arc<AppState>,
    ws_id: WorkspaceId,
    root: &PathBuf,
    now: Instant,
) -> Result<Arc<Vec<String>>, StatusCode> {
    let started_at = Instant::now();
    let files = Arc::new(list_host_git_files(root).await?);

    let mut cache = state
        .workspaces
        .workspace_file_completions_cache
        .lock()
        .await;
    cache.insert(
        ws_id,
        crate::daemon::TimedEntry::new(crate::daemon::CachedFileCompletions {
            cached_at: now,
            files: files.clone(),
        }),
    );
    record_list_files_metric(state, "list_files_workspace", started_at).await;
    Ok(files)
}

pub(super) async fn list_host_git_files(root: &PathBuf) -> Result<Vec<String>, StatusCode> {
    let tracked = list_tracked_files(root)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let untracked = list_untracked_files(root)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(merge_and_sort_git_paths(tracked, untracked))
}

pub(super) fn merge_and_sort_git_paths(
    mut tracked: Vec<String>,
    untracked: Vec<String>,
) -> Vec<String> {
    if !untracked.is_empty() {
        let mut seen: std::collections::HashSet<String> = tracked.iter().cloned().collect();
        for p in untracked {
            if seen.insert(p.clone()) {
                tracked.push(p);
            }
        }
    }
    tracked.sort();
    tracked
}

async fn record_list_files_metric(state: &Arc<AppState>, event: &'static str, started_at: Instant) {
    let mut labels = HashMap::new();
    labels.insert("event".to_string(), event.to_string());
    labels.insert("source".to_string(), "daemon".to_string());
    let metric = PerfMetric {
        name: "fs.list_files_ms".to_string(),
        kind: PerfMetricKind::Histogram,
        unit: "ms".to_string(),
        value: started_at.elapsed().as_millis() as f64,
        labels,
    };
    state
        .telemetry
        .perf_telemetry
        .record_metric(metric, None, None, None)
        .await;
}
