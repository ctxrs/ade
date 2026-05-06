use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};

use ctx_core::models::Worktree;
use ctx_fs::vcs::VcsStructuredStatus;
use ctx_workspace_container::workspace_container_name;
use ctx_workspace_services::worktree_vcs::{
    parse_git_diff_name_status, parse_git_list_untracked, WorktreeVcsGitCommand,
};

use crate::daemon::AppState;
use crate::execution_effective;
use crate::settings::ContainerRuntimeKind;
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
    args: &[String],
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
            let guest_args = args.to_vec();
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
    command: WorktreeVcsGitCommand,
) -> Result<Vec<u8>> {
    let args = command.args();
    let out = container_git_output(state, worktree, &args).await?;
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
    include_entries: bool,
) -> Result<VcsStructuredStatus> {
    let bytes = container_git_stdout(
        state,
        worktree,
        WorktreeVcsGitCommand::Status {
            include_untracked_files,
        },
    )
    .await?;
    Ok(ctx_fs::git::git_status_structured_from_bytes_with_entries(
        &bytes,
        include_entries,
    ))
}

pub(crate) async fn container_git_list_untracked(
    state: &Arc<AppState>,
    worktree: &Worktree,
) -> Result<Vec<String>> {
    let bytes = container_git_stdout(state, worktree, WorktreeVcsGitCommand::ListUntracked).await?;
    Ok(parse_git_list_untracked(&bytes))
}

pub(crate) async fn container_git_diff_name_status(
    state: &Arc<AppState>,
    worktree: &Worktree,
    base_commit_sha: &str,
) -> Result<Vec<(String, String, Option<String>)>> {
    container_git_diff_name_status_inner(state, worktree, base_commit_sha, false).await
}

pub(crate) async fn container_git_diff_name_status_no_renames(
    state: &Arc<AppState>,
    worktree: &Worktree,
    base_commit_sha: &str,
) -> Result<Vec<(String, String, Option<String>)>> {
    container_git_diff_name_status_inner(state, worktree, base_commit_sha, true).await
}

async fn container_git_diff_name_status_inner(
    state: &Arc<AppState>,
    worktree: &Worktree,
    base_commit_sha: &str,
    no_renames: bool,
) -> Result<Vec<(String, String, Option<String>)>> {
    let bytes = container_git_stdout(
        state,
        worktree,
        WorktreeVcsGitCommand::DiffNameStatus {
            base_commit_sha: base_commit_sha.to_string(),
            no_renames,
        },
    )
    .await?;
    Ok(parse_git_diff_name_status(&bytes))
}
