use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};

use ctx_core::models::Worktree;
use ctx_fs::vcs;
use ctx_fs::vcs::VcsStructuredStatus;
use ctx_workspace_container::workspace_container_name;

use crate::daemon::AppState;
use crate::execution_effective;
use crate::settings::{ContainerRuntimeKind, ExecutionMode};
use crate::worktree_data_plane::resolve_worktree_data_plane;
use ctx_harness_runtime::sandbox_container_command;
use ctx_worktree_data_plane::apply_data_plane_to_execution_settings;

enum SandboxGitTarget {
    NativeContainer { container_name: String },
    SharedVmContainer,
}

struct SandboxGitContext {
    live_worktree_root: PathBuf,
    target: SandboxGitTarget,
}

async fn ensure_container_for_worktree(
    state: &Arc<AppState>,
    worktree: &Worktree,
) -> Result<SandboxGitContext> {
    let data_plane = resolve_worktree_data_plane(state, worktree).await?;
    let effective =
        execution_effective::effective_execution_settings(state, data_plane.workspace.id).await?;
    let effective = apply_data_plane_to_execution_settings(&effective, &data_plane)?;
    state
        .execution
        .harness
        .ensure_workspace_container_for_worktree(
            &data_plane.workspace,
            worktree,
            &effective,
            &state.core.daemon_url,
        )
        .await?;
    if matches!(
        effective.container.runtime,
        ContainerRuntimeKind::SharedVmContainer
    ) {
        Ok(SandboxGitContext {
            live_worktree_root: data_plane.live_worktree_root,
            target: SandboxGitTarget::SharedVmContainer,
        })
    } else {
        Ok(SandboxGitContext {
            live_worktree_root: data_plane.live_worktree_root,
            target: SandboxGitTarget::NativeContainer {
                container_name: workspace_container_name(worktree.workspace_id),
            },
        })
    }
}

async fn container_git_output(
    state: &Arc<AppState>,
    worktree: &Worktree,
    args: &[&str],
) -> Result<std::process::Output> {
    const SANDBOX_GIT_TIMEOUT: Duration = Duration::from_secs(30);
    let context = ensure_container_for_worktree(state, worktree).await?;
    match context.target {
        SandboxGitTarget::NativeContainer { container_name } => {
            let mut cmd = sandbox_container_command(&state.core.data_root)?;
            cmd.arg("exec")
                .arg("--workdir")
                .arg(&context.live_worktree_root)
                .arg(&container_name)
                .arg("git")
                .args(args);
            ctx_sandbox_container_runtime::command_output_with_timeout(cmd, SANDBOX_GIT_TIMEOUT)
                .await
                .context("sandbox exec git timed out")
        }
        SandboxGitTarget::SharedVmContainer => {
            let guest_args = args
                .iter()
                .map(|arg| (*arg).to_string())
                .collect::<Vec<_>>();
            tokio::time::timeout(
                SANDBOX_GIT_TIMEOUT,
                ctx_avf_linux_runtime::run_guest_exec_capture(
                    &state.core.data_root,
                    worktree.workspace_id,
                    worktree.id,
                    &context.live_worktree_root,
                    "git",
                    &guest_args,
                    &HashMap::new(),
                    None,
                    false,
                ),
            )
            .await
            .context("shared VM container exec git timed out")?
        }
    }
}

pub(super) async fn container_git_stdout(
    state: &Arc<AppState>,
    worktree: &Worktree,
    args: &[&str],
) -> Result<Vec<u8>> {
    let out = container_git_output(state, worktree, args).await?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        anyhow::bail!(
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
}

pub(crate) async fn container_git_status_structured(
    state: &Arc<AppState>,
    worktree: &Worktree,
    include_untracked_files: bool,
) -> Result<VcsStructuredStatus> {
    let untracked_mode = if include_untracked_files {
        "--untracked-files=all"
    } else {
        "--untracked-files=normal"
    };
    let bytes = container_git_stdout(
        state,
        worktree,
        &["status", "--porcelain", "-z", "--branch", untracked_mode],
    )
    .await?;
    Ok(ctx_fs::git::git_status_structured_from_bytes(&bytes))
}

pub(crate) async fn container_git_list_untracked(
    state: &Arc<AppState>,
    worktree: &Worktree,
) -> Result<Vec<String>> {
    let bytes = container_git_stdout(
        state,
        worktree,
        &["ls-files", "--others", "--exclude-standard", "-z"],
    )
    .await?;
    let mut out = Vec::new();
    for part in bytes.split(|b| *b == 0) {
        if part.is_empty() {
            continue;
        }
        out.push(String::from_utf8_lossy(part).to_string());
    }
    Ok(out)
}

pub(crate) async fn container_git_count_untracked(
    state: &Arc<AppState>,
    worktree: &Worktree,
) -> Result<i64> {
    let bytes = container_git_stdout(
        state,
        worktree,
        &["ls-files", "--others", "--exclude-standard", "-z"],
    )
    .await?;
    Ok(bytes
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .count() as i64)
}

pub(crate) async fn container_git_diff_name_status(
    state: &Arc<AppState>,
    worktree: &Worktree,
    base_commit_sha: &str,
) -> Result<Vec<(String, String, Option<String>)>> {
    let bytes = container_git_stdout(
        state,
        worktree,
        &["diff", "--name-status", "-z", base_commit_sha],
    )
    .await?;
    let mut out = Vec::new();
    let mut parts = bytes
        .split(|b| *b == 0)
        .filter(|part| !part.is_empty())
        .map(|part| String::from_utf8_lossy(part).to_string())
        .peekable();
    while let Some(part) = parts.next() {
        let status = part.trim().to_string();
        if status.is_empty() {
            continue;
        }
        let Some(path) = parts.next() else {
            continue;
        };
        if status.is_empty() || path.trim().is_empty() {
            continue;
        }
        let status_char = status.chars().next().unwrap_or('M');
        if status_char == 'R' || status_char == 'C' {
            let Some(next_path) = parts.next() else {
                continue;
            };
            let new_path = next_path;
            if new_path.trim().is_empty() {
                continue;
            }
            out.push((status, new_path, Some(path)));
        } else {
            out.push((status, path, None));
        }
    }
    Ok(out)
}

pub(crate) async fn container_git_diff_name_status_count(
    state: &Arc<AppState>,
    worktree: &Worktree,
    base_commit_sha: &str,
) -> Result<i64> {
    let bytes = container_git_stdout(
        state,
        worktree,
        &["diff", "--name-status", "-z", base_commit_sha],
    )
    .await?;
    Ok(count_name_status_entries(&bytes))
}

fn count_name_status_entries(bytes: &[u8]) -> i64 {
    let mut total = 0;
    let mut parts = bytes
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty());
    while let Some(status_bytes) = parts.next() {
        let status = String::from_utf8_lossy(status_bytes);
        let status = status.trim();
        if status.is_empty() {
            continue;
        }
        let Some(path) = parts.next() else {
            continue;
        };
        if String::from_utf8_lossy(path).trim().is_empty() {
            continue;
        }
        let status_char = status.chars().next().unwrap_or('M');
        if status_char == 'R' || status_char == 'C' {
            let Some(next_path) = parts.next() else {
                continue;
            };
            if String::from_utf8_lossy(next_path).trim().is_empty() {
                continue;
            }
        }
        total += 1;
    }
    total
}

pub(crate) async fn container_git_rev_parse(
    state: &Arc<AppState>,
    worktree: &Worktree,
    reference: &str,
) -> Result<String> {
    let bytes = container_git_stdout(state, worktree, &["rev-parse", reference]).await?;
    Ok(String::from_utf8_lossy(&bytes).trim().to_string())
}

async fn container_git_merge_base(
    state: &Arc<AppState>,
    worktree: &Worktree,
    target_branch: &str,
) -> Result<String> {
    let bytes =
        container_git_stdout(state, worktree, &["merge-base", target_branch, "HEAD"]).await?;
    Ok(String::from_utf8_lossy(&bytes).trim().to_string())
}

pub(crate) async fn worktree_rev_parse_head(
    state: &Arc<AppState>,
    worktree: &Worktree,
) -> Result<String> {
    let data_plane = resolve_worktree_data_plane(state, worktree).await?;
    if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
        return container_git_rev_parse(state, worktree, "HEAD").await;
    }
    let root = data_plane.live_worktree_root.as_path();
    let driver = vcs::driver_for_path(root).await?;
    driver.rev_parse_head(root).await
}

pub(crate) async fn worktree_merge_base(
    state: &Arc<AppState>,
    worktree: &Worktree,
    target_branch: &str,
) -> Result<String> {
    let data_plane = resolve_worktree_data_plane(state, worktree).await?;
    if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
        return container_git_merge_base(state, worktree, target_branch).await;
    }
    let root = data_plane.live_worktree_root.as_path();
    let driver = vcs::driver_for_path(root).await?;
    driver.merge_base(root, target_branch, "HEAD").await
}
