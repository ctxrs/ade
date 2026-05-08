use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::http::StatusCode;
use ctx_core::ids::WorkspaceId;
use ctx_core::models::{ExecutionEnvironment, Worktree};
use ctx_fs::git::{list_tracked_files, list_untracked_files};
use ctx_observability::perf_telemetry::{PerfMetric, PerfMetricKind};
use ctx_settings_model::ContainerRuntimeKind;
use ctx_worktree_data_plane::apply_data_plane_to_execution_settings;
use ctx_worktree_data_plane::resolve_worktree_data_plane_with_host as resolve_worktree_data_plane;
use serde::Deserialize;

use crate::daemon::execution_effective;
use crate::daemon::AppState;

use super::errors::status_code_for_internal_error;

#[derive(Debug, Deserialize, Default)]
pub(crate) struct FileCompletionsQuery {
    pub(crate) query: Option<String>,
    pub(crate) limit: Option<u32>,
}

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
    record_list_files_metric(state, "list_files_worktree", started_at).await;
    Ok(files)
}

async fn list_container_worktree_files(
    state: &Arc<AppState>,
    worktree: &Worktree,
    execution_environment: ExecutionEnvironment,
) -> Result<Vec<String>, StatusCode> {
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
    .map_err(|err| status_code_for_internal_error(&err))?;
    let data_plane = resolve_worktree_data_plane(state.as_ref(), worktree)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let settings = apply_data_plane_to_execution_settings(&settings, &data_plane)
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

    let workdir = data_plane.live_worktree_root.to_string_lossy().to_string();

    let tracked = container_git_ls_files(
        state,
        worktree,
        settings.container.runtime.clone(),
        &workdir,
        &["ls-files", "-z"],
    )
    .await?;
    let untracked = container_git_ls_files(
        state,
        worktree,
        settings.container.runtime,
        &workdir,
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
    const SANDBOX_GIT_LS_FILES_TIMEOUT: Duration = Duration::from_secs(30);
    let out = match runtime {
        ContainerRuntimeKind::NativeContainer => {
            let mut cmd = ctx_harness_runtime::sandbox_container_command(&state.core.data_root)
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            cmd.arg("exec")
                .arg("--workdir")
                .arg(workdir)
                .arg(ctx_workspace_container::workspace_container_name(
                    worktree.workspace_id,
                ))
                .arg("git");
            for arg in git_args {
                cmd.arg(arg);
            }
            ctx_sandbox_container_runtime::command_output_with_timeout(
                cmd,
                SANDBOX_GIT_LS_FILES_TIMEOUT,
            )
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        }
        ContainerRuntimeKind::SharedVmContainer => {
            let args = git_args
                .iter()
                .map(|arg| (*arg).to_string())
                .collect::<Vec<_>>();
            let guest_cwd = PathBuf::from(workdir);
            tokio::time::timeout(
                SANDBOX_GIT_LS_FILES_TIMEOUT,
                ctx_avf_linux_runtime::run_guest_exec_capture(
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

pub(crate) async fn load_and_cache_workspace_files(
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
    record_list_files_metric(state, "list_files_workspace", started_at).await;
    Ok(files)
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
