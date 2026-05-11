use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use ctx_core::models::Worktree;
use ctx_harness_runtime::sandbox_container_command;
use ctx_sandbox_container_runtime::command_output_with_timeout;
use ctx_workspace_services::worktree_vcs::{
    parse_worktree_vcs_diff_summary_counts, WorktreeVcsDiffSummaryCounts,
    WORKTREE_VCS_CONTAINER_DIFF_SCRIPT, WORKTREE_VCS_CONTAINER_DIFF_SUMMARY_SCRIPT,
};
use ctx_worktree_data_plane::resolve_worktree_data_plane_with_host as resolve_worktree_data_plane;

use crate::daemon::AppState;
use target::{ensure_container_for_worktree, SandboxExecTarget};

#[path = "sandbox/target.rs"]
mod target;

async fn container_exec_stdout(
    state: &Arc<AppState>,
    worktree: &Worktree,
    program: &str,
    args: &[&str],
) -> anyhow::Result<Vec<u8>> {
    const SANDBOX_EXEC_TIMEOUT: Duration = Duration::from_secs(30);
    let target = ensure_container_for_worktree(state, worktree).await?;
    let data_plane = resolve_worktree_data_plane(state.as_ref(), worktree).await?;
    let out = match target {
        SandboxExecTarget::NativeContainer { container_name } => {
            let mut cmd = sandbox_container_command(&state.core.data_root)?;
            cmd.arg("exec")
                .arg("--workdir")
                .arg(&data_plane.live_worktree_root)
                .arg(&container_name)
                .arg(program)
                .args(args);
            command_output_with_timeout(cmd, SANDBOX_EXEC_TIMEOUT)
                .await
                .context("sandbox exec command timed out")?
        }
        SandboxExecTarget::SharedVmContainer => ctx_avf_linux_runtime::run_guest_exec_capture(
            &state.core.data_root,
            worktree.workspace_id,
            worktree.id,
            &data_plane.live_worktree_root,
            program,
            &args
                .iter()
                .map(|arg| (*arg).to_string())
                .collect::<Vec<_>>(),
            &std::collections::HashMap::new(),
            None,
            false,
        )
        .await
        .context("shared VM container exec command failed")?,
    };
    if out.status.success() {
        Ok(out.stdout)
    } else {
        anyhow::bail!("{} {:?} failed: {}", program, args, {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !stderr.is_empty() {
                stderr
            } else if !stdout.is_empty() {
                stdout
            } else {
                "unknown sandbox exec failure".to_string()
            }
        });
    }
}

pub(super) async fn container_diff_worktree(
    state: &Arc<AppState>,
    worktree: &Worktree,
    base_commit_sha: &str,
) -> anyhow::Result<String> {
    let bytes = container_exec_stdout(
        state,
        worktree,
        "bash",
        &[
            "-lc",
            WORKTREE_VCS_CONTAINER_DIFF_SCRIPT,
            "--",
            base_commit_sha,
        ],
    )
    .await?;
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

pub(super) async fn container_diff_worktree_summary(
    state: &Arc<AppState>,
    worktree: &Worktree,
    base_commit_sha: &str,
) -> anyhow::Result<WorktreeVcsDiffSummaryCounts> {
    let bytes = container_exec_stdout(
        state,
        worktree,
        "bash",
        &[
            "-lc",
            WORKTREE_VCS_CONTAINER_DIFF_SUMMARY_SCRIPT,
            "--",
            base_commit_sha,
        ],
    )
    .await?;
    parse_worktree_vcs_diff_summary_counts(&bytes)
}
