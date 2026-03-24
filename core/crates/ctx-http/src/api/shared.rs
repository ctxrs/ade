use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::http::StatusCode;
use axum::Json;
use ctx_core::ids::WorkspaceId;
use ctx_core::models::{ExecutionEnvironment, Worktree};
use ctx_fs::git::{list_tracked_files, list_untracked_files};
use serde::Deserialize;

use super::errors::ApiErrorResp;
use crate::container_fs::is_container_path;
use crate::daemon::AppState;
use crate::execution_effective;
use crate::harness_runtime;
use crate::logs;
use crate::perf_telemetry::{PerfMetric, PerfMetricKind};
use crate::settings::ContainerRuntimeKind;

pub(super) fn session_root_kind_for_worktree(wt: Option<&Worktree>) -> &'static str {
    match wt.and_then(|w| w.git_branch.as_ref()) {
        Some(_) => "worktree",
        None => "workspace_root",
    }
}

pub(super) fn map_effective_execution_settings_error(
    err: execution_effective::EffectiveExecutionSettingsError,
) -> (StatusCode, Json<ApiErrorResp>) {
    let (status, error) = match err {
        execution_effective::EffectiveExecutionSettingsError::InvalidWorkspaceOverride(err) => {
            (StatusCode::BAD_REQUEST, err)
        }
        execution_effective::EffectiveExecutionSettingsError::Internal(err) => {
            (StatusCode::INTERNAL_SERVER_ERROR, err)
        }
    };
    (
        status,
        Json(ApiErrorResp {
            error: logs::redact_sensitive(&error.to_string()),
        }),
    )
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct FileCompletionsQuery {
    pub(super) query: Option<String>,
    pub(super) limit: Option<u32>,
}

pub(super) async fn load_and_cache_worktree_files(
    state: &Arc<AppState>,
    worktree: &Worktree,
    execution_environment: ExecutionEnvironment,
    now: Instant,
) -> Result<Arc<Vec<String>>, StatusCode> {
    let started_at = Instant::now();
    let root = PathBuf::from(&worktree.root_path);
    let files = if is_container_path(&root) {
        Arc::new(list_container_worktree_files(state, worktree, execution_environment).await?)
    } else {
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
        Arc::new(files)
    };

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

async fn list_container_worktree_files(
    state: &Arc<AppState>,
    worktree: &Worktree,
    execution_environment: ExecutionEnvironment,
) -> Result<Vec<String>, StatusCode> {
    // Run git inside the harness container.
    let workspace_id = worktree.workspace_id;
    let workspace = state
        .global_store()
        .get_workspace(workspace_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let settings = execution_effective::effective_execution_settings_for_environment(
        state,
        workspace_id,
        execution_environment,
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    state
        .execution
        .harness
        .ensure_workspace_container_for_worktree(
            &workspace,
            worktree,
            &settings,
            &state.core.daemon_url,
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let workdir = worktree.root_path.trim();

    let tracked = container_git_ls_files(
        state,
        worktree,
        settings.container.runtime.clone(),
        workdir,
        &["ls-files", "-z"],
    )
    .await?;
    let untracked = container_git_ls_files(
        state,
        worktree,
        settings.container.runtime,
        workdir,
        &["ls-files", "--others", "--exclude-standard", "-z"],
    )
    .await?;

    let mut files = tracked;
    if !untracked.is_empty() {
        let mut seen: std::collections::HashSet<String> = files.iter().cloned().collect();
        for p in untracked {
            if seen.insert(p.clone()) {
                files.push(p);
            }
        }
    }
    files.sort();
    Ok(files)
}

async fn container_git_ls_files(
    state: &Arc<AppState>,
    worktree: &Worktree,
    runtime: ContainerRuntimeKind,
    workdir: &str,
    git_args: &[&str],
) -> Result<Vec<String>, StatusCode> {
    // This runs on user keystrokes (completions). Bound it so a wedged sandbox connection doesn't
    // hang request handling indefinitely.
    const SANDBOX_GIT_LS_FILES_TIMEOUT: Duration = Duration::from_secs(30);
    let out = match runtime {
        ContainerRuntimeKind::Podman => {
            let mut cmd = harness_runtime::podman_command(&state.core.data_root)
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            cmd.arg("exec")
                .arg("--workdir")
                .arg(workdir)
                .arg(harness_runtime::workspace_container_name(
                    worktree.workspace_id,
                ))
                .arg("git");
            for arg in git_args {
                cmd.arg(arg);
            }
            harness_runtime::command_output_with_timeout(cmd, SANDBOX_GIT_LS_FILES_TIMEOUT)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        }
        ContainerRuntimeKind::AvfLinuxVm => {
            let args = git_args
                .iter()
                .map(|arg| (*arg).to_string())
                .collect::<Vec<_>>();
            let guest_cwd = PathBuf::from(workdir);
            tokio::time::timeout(
                SANDBOX_GIT_LS_FILES_TIMEOUT,
                crate::workspace_runtime::run_avf_linux_guest_exec_capture(
                    &state.core.data_root,
                    worktree.workspace_id,
                    worktree.id,
                    &guest_cwd,
                    "git",
                    &args,
                    &HashMap::new(),
                    None,
                    false,
                ),
            )
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        }
    };
    if !out.status.success() {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    // `-z` terminated output.
    let mut files = Vec::new();
    for part in out.stdout.split(|b| *b == 0u8) {
        if part.is_empty() {
            continue;
        }
        let s = String::from_utf8_lossy(part).to_string();
        if !s.trim().is_empty() {
            files.push(s);
        }
    }
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
