use super::*;
use crate::settings::ExecutionMode;
use crate::worktree_data_plane::resolve_worktree_data_plane;
use ctx_workspace_services::worktree_vcs::{
    parse_worktree_vcs_diff_summary_counts, WorktreeVcsDiffSummaryCounts,
    WORKTREE_VCS_CONTAINER_DIFF_SCRIPT, WORKTREE_VCS_CONTAINER_DIFF_SUMMARY_SCRIPT,
};
use ctx_worktree_data_plane::apply_data_plane_to_execution_settings;

enum SandboxExecTarget {
    NativeContainer { container_name: String },
    SharedVmContainer,
}

async fn ensure_container_for_worktree(
    state: &Arc<AppState>,
    worktree: &Worktree,
) -> anyhow::Result<SandboxExecTarget> {
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
        crate::settings::ContainerRuntimeKind::SharedVmContainer
    ) {
        Ok(SandboxExecTarget::SharedVmContainer)
    } else {
        Ok(SandboxExecTarget::NativeContainer {
            container_name: workspace_container_name(data_plane.workspace.id),
        })
    }
}

async fn container_exec_stdout(
    state: &Arc<AppState>,
    worktree: &Worktree,
    program: &str,
    args: &[&str],
) -> anyhow::Result<Vec<u8>> {
    const SANDBOX_EXEC_TIMEOUT: Duration = Duration::from_secs(30);
    let target = ensure_container_for_worktree(state, worktree).await?;
    let data_plane = resolve_worktree_data_plane(state, worktree).await?;
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

async fn container_diff_worktree(
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

async fn container_diff_worktree_summary(
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

pub(crate) async fn diff_worktree_for_session(
    state: &Arc<AppState>,
    worktree: &Worktree,
    base_commit_sha: &str,
) -> anyhow::Result<String> {
    let data_plane = resolve_worktree_data_plane(state, worktree).await?;
    if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
        return container_diff_worktree(state, worktree, base_commit_sha).await;
    }
    ctx_fs::worktrees::diff_worktree(
        data_plane.live_worktree_root.to_string_lossy().as_ref(),
        base_commit_sha,
    )
    .await
}

pub(crate) async fn diff_worktree_summary_for_session(
    state: &Arc<AppState>,
    worktree: &Worktree,
    base_commit_sha: &str,
) -> anyhow::Result<WorktreeVcsDiffSummaryCounts> {
    let data_plane = resolve_worktree_data_plane(state, worktree).await?;
    if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
        return container_diff_worktree_summary(state, worktree, base_commit_sha).await;
    }
    let (file_count, line_additions, line_deletions) = ctx_fs::worktrees::diff_worktree_summary(
        data_plane.live_worktree_root.to_string_lossy().as_ref(),
        base_commit_sha,
    )
    .await?;
    Ok(WorktreeVcsDiffSummaryCounts {
        file_count,
        line_additions,
        line_deletions,
    })
}
