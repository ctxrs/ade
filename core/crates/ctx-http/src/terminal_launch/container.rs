use std::collections::HashMap;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use axum::Json;
use ctx_sandbox_container_runtime::{
    command_output_message, command_output_with_timeout, sandbox_cli_invocation,
    sandbox_container_command, SandboxCommandMode,
};

use crate::api::errors::ApiErrorResp;
use crate::daemon::AppState;
use crate::settings::{ContainerRuntimeKind, ExecutionMode};
use crate::terminals::{NativeContainerTerminalSpec, SharedVmContainerTerminalSpec};
use ctx_core::ids::WorkspaceId;
use ctx_core::models::{Workspace, Worktree};

use super::{bad_request, internal_error};

const TERMINAL_CONTAINER_CWD_TIMEOUT: Duration = Duration::from_secs(10);

pub(super) async fn prepare_terminal_container_launch(
    state: &Arc<AppState>,
    workspace: &Workspace,
    worktree: Option<&Worktree>,
    effective: &crate::settings::ExecutionSettings,
    workspace_id: WorkspaceId,
    cwd: &FsPath,
    container_cwd_authority_root: Option<&FsPath>,
) -> Result<
    (
        PathBuf,
        Option<NativeContainerTerminalSpec>,
        Option<SharedVmContainerTerminalSpec>,
    ),
    (StatusCode, Json<ApiErrorResp>),
> {
    if !matches!(effective.mode, ExecutionMode::Sandbox) {
        return Ok((cwd.to_path_buf(), None, None));
    }
    let container_cwd_authority_root = container_cwd_authority_root.ok_or_else(|| {
        internal_error("sandbox terminal requires a resolved container cwd authority root")
    })?;
    let container_name = ctx_workspace_container::workspace_container_name(workspace_id);

    match effective.container.runtime {
        ContainerRuntimeKind::NativeContainer => {
            state
                .execution
                .harness
                .ensure_workspace_container(workspace, effective, &state.core.daemon_url)
                .await
                .map_err(|e| internal_error(format!("failed to ensure harness container: {e}")))?;
            if worktree.is_none() {
                ensure_materialized_workspace_root(state, workspace).await?;
            }
            let canonical_cwd = canonicalize_container_terminal_cwd(
                &state.core.data_root,
                &SandboxCommandMode::NativeContainer,
                &container_name,
                cwd,
                container_cwd_authority_root,
            )
            .await?;
            let inv = sandbox_cli_invocation(&state.core.data_root)
                .map_err(|e| internal_error(format!("sandbox container CLI unavailable: {e}")))?;
            Ok((
                canonical_cwd.clone(),
                Some(NativeContainerTerminalSpec {
                    cli_bin: inv.bin,
                    cli_env: inv.env,
                    container_name,
                    workdir: canonical_cwd.to_string_lossy().to_string(),
                    user: Some(ctx_workspace_container::CONTAINER_TERMINAL_USER.to_string()),
                }),
                None,
            ))
        }
        ContainerRuntimeKind::SharedVmContainer => {
            if let Some(worktree) = worktree {
                state
                    .execution
                    .harness
                    .ensure_workspace_container_for_worktree(
                        workspace,
                        worktree,
                        effective,
                        &state.core.daemon_url,
                    )
                    .await
                    .map_err(|e| {
                        internal_error(format!("failed to ensure sandbox container: {e}"))
                    })?;
            } else {
                state
                    .execution
                    .harness
                    .ensure_workspace_container(workspace, effective, &state.core.daemon_url)
                    .await
                    .map_err(|e| {
                        internal_error(format!("failed to ensure sandbox container: {e}"))
                    })?;
            }
            if worktree.is_none() {
                ensure_materialized_workspace_root(state, workspace).await?;
            }
            let helper_path = ctx_avf_linux_runtime::helper_path()
                .map_err(|e| internal_error(format!("AVF helper unavailable: {e}")))?;
            let command_mode = SandboxCommandMode::SharedVm {
                helper_path: helper_path.clone(),
            };
            let canonical_cwd = canonicalize_container_terminal_cwd(
                &state.core.data_root,
                &command_mode,
                &container_name,
                cwd,
                container_cwd_authority_root,
            )
            .await?;
            Ok((
                canonical_cwd.clone(),
                None,
                Some(SharedVmContainerTerminalSpec {
                    helper_path,
                    data_root: state.core.data_root.clone(),
                    workspace_id,
                    workdir: canonical_cwd.to_string_lossy().to_string(),
                    user: Some(ctx_workspace_container::CONTAINER_TERMINAL_USER.to_string()),
                }),
            ))
        }
    }
}

async fn ensure_materialized_workspace_root(
    state: &Arc<AppState>,
    workspace: &Workspace,
) -> Result<(), (StatusCode, Json<ApiErrorResp>)> {
    let sandbox_mode = ctx_harness_runtime::selected_sandbox_command_mode(&state.core.data_root)
        .map_err(|err| internal_error(err.to_string()))?;
    ctx_sandbox_materialization::ensure_workspace_root_from_host_copy(
        &state.core.data_root,
        &sandbox_mode,
        workspace,
    )
    .await
    .map_err(|e| internal_error(format!("failed to materialize sandbox workspace root: {e}")))?;
    Ok(())
}

async fn canonicalize_container_terminal_cwd(
    data_root: &FsPath,
    mode: &SandboxCommandMode,
    container_name: &str,
    cwd: &FsPath,
    live_root: &FsPath,
) -> Result<PathBuf, (StatusCode, Json<ApiErrorResp>)> {
    let mut cmd = sandbox_container_command(data_root, mode)
        .map_err(|e| internal_error(format!("sandbox container CLI unavailable: {e}")))?;
    cmd.arg("exec")
        .arg("--user")
        .arg("0")
        .arg(container_name)
        .arg("realpath")
        .arg("-e")
        .arg("--")
        .arg(cwd);
    let output = command_output_with_timeout(cmd, TERMINAL_CONTAINER_CWD_TIMEOUT)
        .await
        .map_err(|e| internal_error(format!("failed to validate sandbox terminal cwd: {e}")))?;
    if !output.status.success() {
        let detail = command_output_message(&output);
        if detail.is_empty() {
            return Err(bad_request("cwd does not exist"));
        }
        return Err(bad_request(format!("cwd does not exist: {detail}")));
    }
    let stdout = String::from_utf8(output.stdout)
        .map_err(|_| internal_error("sandbox terminal cwd validation returned invalid UTF-8"))?;
    let canonical = stdout
        .lines()
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| internal_error("sandbox terminal cwd validation returned no path"))?;
    validate_canonical_container_terminal_cwd(live_root, &canonical)
}

fn validate_canonical_container_terminal_cwd(
    live_root: &FsPath,
    canonical: &FsPath,
) -> Result<PathBuf, (StatusCode, Json<ApiErrorResp>)> {
    if !canonical.is_absolute() {
        return Err(internal_error(
            "sandbox terminal cwd validation returned a relative path",
        ));
    }
    if !canonical.starts_with(live_root) {
        return Err(bad_request(
            "cwd must be within the container worktree/workspace root",
        ));
    }
    Ok(canonical.to_path_buf())
}

pub(super) fn container_terminal_env() -> HashMap<String, String> {
    HashMap::from([
        (
            "HOME".to_string(),
            ctx_workspace_container::CONTAINER_TERMINAL_HOME.to_string(),
        ),
        (
            "USER".to_string(),
            ctx_workspace_container::CONTAINER_TERMINAL_USER.to_string(),
        ),
        (
            "LOGNAME".to_string(),
            ctx_workspace_container::CONTAINER_TERMINAL_USER.to_string(),
        ),
    ])
}
