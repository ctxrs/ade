use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::http::StatusCode;
use axum::Json;
use ctx_core::ids::WorkspaceId;
use ctx_core::models::{ExecutionEnvironment, Worktree};
use ctx_fs::git::{list_tracked_files, list_untracked_files};
use serde::Deserialize;

use super::errors::ApiErrorResp;
use crate::daemon::AppState;
use crate::execution_effective;
use ctx_observability::logs;
use ctx_observability::perf_telemetry::{PerfMetric, PerfMetricKind};
use ctx_settings_model::ContainerRuntimeKind;
use ctx_worktree_data_plane::apply_data_plane_to_execution_settings;
use ctx_worktree_data_plane::resolve_worktree_data_plane_with_host as resolve_worktree_data_plane;

pub(super) fn status_code_for_internal_error(err: &anyhow::Error) -> StatusCode {
    if ctx_settings_service::is_execution_policy_denial(err) {
        StatusCode::FORBIDDEN
    } else if err
        .chain()
        .any(|cause| crate::storage_guard::is_storage_exhaustion_error(&cause.to_string()))
    {
        StatusCode::INSUFFICIENT_STORAGE
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

pub(crate) fn status_code_for_request_or_policy_error(err: &anyhow::Error) -> StatusCode {
    if ctx_settings_service::is_execution_policy_denial(err) {
        StatusCode::FORBIDDEN
    } else {
        StatusCode::BAD_REQUEST
    }
}

fn internal_api_error_message(err: &anyhow::Error) -> String {
    if let Some(storage_message) = err
        .chain()
        .map(ToString::to_string)
        .find(|message| crate::storage_guard::is_storage_exhaustion_error(message))
    {
        storage_message
    } else {
        format!("{err:#}")
    }
}

pub(crate) fn map_internal_api_error(err: &anyhow::Error) -> (StatusCode, Json<ApiErrorResp>) {
    (
        status_code_for_internal_error(err),
        Json(ApiErrorResp {
            error: logs::redact_sensitive(&internal_api_error_message(err)),
        }),
    )
}

pub(super) async fn store_for_existing_workspace_status(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
) -> Result<ctx_store::Store, StatusCode> {
    match state.lookup_workspace_store(workspace_id).await {
        crate::daemon::StoreLookup::Found(store) => Ok(store),
        crate::daemon::StoreLookup::Missing | crate::daemon::StoreLookup::Deleting => {
            Err(StatusCode::NOT_FOUND)
        }
        crate::daemon::StoreLookup::Unavailable(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_code_for_internal_error_maps_storage_failures_to_insufficient_storage() {
        let err =
            anyhow::anyhow!("Insufficient storage capacity for creating an isolated task worktree");
        assert_eq!(
            status_code_for_internal_error(&err),
            StatusCode::INSUFFICIENT_STORAGE
        );
    }

    #[test]
    fn status_code_for_internal_error_preserves_generic_internal_errors() {
        let err = anyhow::anyhow!("plain internal failure");
        assert_eq!(
            status_code_for_internal_error(&err),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn status_code_for_internal_error_maps_execution_policy_denials_to_forbidden() {
        let err = ctx_settings_service::HostExecutionPolicy::SandboxOnly
            .validate_execution_environment(ctx_core::models::ExecutionEnvironment::Host)
            .expect_err("host execution should be denied");
        assert_eq!(status_code_for_internal_error(&err), StatusCode::FORBIDDEN);
    }

    #[test]
    fn status_code_for_request_or_policy_error_maps_policy_denials_to_forbidden() {
        let err = ctx_settings_service::HostExecutionPolicy::SandboxOnly
            .validate_execution_environment(ctx_core::models::ExecutionEnvironment::Host)
            .expect_err("host execution should be denied");
        assert_eq!(
            status_code_for_request_or_policy_error(&err),
            StatusCode::FORBIDDEN
        );
    }

    #[test]
    fn status_code_for_request_or_policy_error_preserves_bad_request_for_validation_errors() {
        let err = anyhow::anyhow!("invalid request");
        assert_eq!(
            status_code_for_request_or_policy_error(&err),
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn map_internal_api_error_preserves_storage_guidance() {
        let err = anyhow::anyhow!("wrapper")
            .context("Insufficient storage capacity for creating an isolated task worktree");
        let (status, body) = map_internal_api_error(&err);
        assert_eq!(status, StatusCode::INSUFFICIENT_STORAGE);
        assert_eq!(
            body.0.error,
            "Insufficient storage capacity for creating an isolated task worktree"
        );
    }

    #[test]
    fn map_effective_execution_settings_error_maps_policy_denials_to_forbidden() {
        let err = ctx_settings_service::HostExecutionPolicy::SandboxOnly
            .validate_execution_environment(ctx_core::models::ExecutionEnvironment::Host)
            .expect_err("host execution should be denied");
        let (status, _) = map_effective_execution_settings_error(
            execution_effective::EffectiveExecutionSettingsError::InvalidWorkspaceOverride(err),
        );

        assert_eq!(status, StatusCode::FORBIDDEN);
    }
}

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
            (status_code_for_request_or_policy_error(&err), err)
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
    // This runs on user keystrokes (completions). Bound it so a wedged sandbox connection doesn't
    // hang request handling indefinitely.
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

pub(super) async fn path_resolves_within_root(path: &Path, root: &Path) -> bool {
    let Ok(canonical_path) = tokio::fs::canonicalize(path).await else {
        return false;
    };
    let Ok(canonical_root) = tokio::fs::canonicalize(root).await else {
        return false;
    };
    canonical_path.starts_with(&canonical_root)
}
