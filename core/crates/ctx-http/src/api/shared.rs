use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use axum::http::StatusCode;
use ctx_core::ids::WorkspaceId;
use ctx_core::models::{Session, Worktree};
use ctx_fs::git::{list_tracked_files, list_untracked_files};
use serde::{Deserialize, Serialize};

use crate::daemon::AppState;
use crate::perf_telemetry::{PerfMetric, PerfMetricKind};

#[derive(Debug, Serialize)]
pub(super) struct SessionWithEnv {
    #[serde(flatten)]
    pub(super) session: Session,
    pub(super) env_target: String, // "worktree" | "local"
}

pub(super) fn env_target_for_worktree(wt: Option<&Worktree>) -> String {
    match wt.and_then(|w| w.git_branch.as_ref()) {
        Some(_) => "worktree".to_string(),
        None => "local".to_string(),
    }
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct FileCompletionsQuery {
    pub(super) query: Option<String>,
    pub(super) limit: Option<u32>,
}

pub(super) async fn load_and_cache_worktree_files(
    state: &Arc<AppState>,
    worktree: &Worktree,
    now: Instant,
) -> Result<Arc<Vec<String>>, StatusCode> {
    let started_at = Instant::now();
    let root = PathBuf::from(&worktree.root_path);
    let mut files = list_tracked_files(&root)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let untracked = list_untracked_files(&root)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !untracked.is_empty() {
        let mut seen: std::collections::HashSet<String> = files.iter().cloned().collect();
        for p in untracked {
            if seen.insert(p.clone()) {
                files.push(p);
            }
        }
    }
    files.sort();
    let files = Arc::new(files);

    let mut cache = state.workspaces.file_completions_cache.lock().await;
    cache.insert(
        worktree.id,
        crate::daemon::TimedEntry::new(crate::daemon::CachedFileCompletions {
            cached_at: now,
            files: files.clone(),
        }),
    );
    let mut labels = HashMap::new();
    labels.insert("event".to_string(), "list_files_worktree".to_string());
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
    Ok(files)
}

pub(super) async fn load_and_cache_workspace_files(
    state: &Arc<AppState>,
    ws_id: WorkspaceId,
    root: &PathBuf,
    now: Instant,
) -> Result<Arc<Vec<String>>, StatusCode> {
    let started_at = Instant::now();
    let mut files = list_tracked_files(root)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let untracked = list_untracked_files(root)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !untracked.is_empty() {
        let mut seen: std::collections::HashSet<String> = files.iter().cloned().collect();
        for p in untracked {
            if seen.insert(p.clone()) {
                files.push(p);
            }
        }
    }
    files.sort();
    let files = Arc::new(files);

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
    let mut labels = HashMap::new();
    labels.insert("event".to_string(), "list_files_workspace".to_string());
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
    Ok(files)
}
