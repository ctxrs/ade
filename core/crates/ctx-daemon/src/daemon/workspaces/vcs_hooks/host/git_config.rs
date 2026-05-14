use anyhow::{bail, Context, Result};
use ctx_core::models::{Workspace, Worktree};
use ctx_workspace_services::vcs_hooks::WorktreeHookExecution;

use super::super::sandbox::sandbox_command;
use crate::daemon::DaemonState;

pub(super) async fn sandbox_git_config_get(
    state: &DaemonState,
    workspace: &Workspace,
    worktree: &Worktree,
    execution: &WorktreeHookExecution,
    key: &str,
) -> Result<Option<String>> {
    let mut cmd = sandbox_command(
        state,
        workspace,
        worktree,
        execution,
        "git",
        &[
            "config".to_string(),
            "--worktree".to_string(),
            "--get".to_string(),
            key.to_string(),
        ],
    )?;
    let output = cmd
        .output()
        .await
        .context("running sandbox git config --get")?;
    if output.status.success() {
        return Ok(Some(
            String::from_utf8_lossy(&output.stdout).trim().to_string(),
        ));
    }
    if output.status.code() == Some(1) {
        return Ok(None);
    }
    bail!(
        "sandbox git config --get failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    )
}

pub(super) async fn sandbox_git_config_set(
    state: &DaemonState,
    workspace: &Workspace,
    worktree: &Worktree,
    execution: &WorktreeHookExecution,
    key: &str,
    value: &str,
) -> Result<()> {
    let mut cmd = sandbox_command(
        state,
        workspace,
        worktree,
        execution,
        "git",
        &[
            "config".to_string(),
            "--worktree".to_string(),
            key.to_string(),
            value.to_string(),
        ],
    )?;
    let output = cmd.output().await.context("running sandbox git config")?;
    if !output.status.success() {
        bail!(
            "sandbox git config --worktree failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

pub(super) async fn sandbox_git_config_unset(
    state: &DaemonState,
    workspace: &Workspace,
    worktree: &Worktree,
    execution: &WorktreeHookExecution,
    key: &str,
) -> Result<()> {
    let mut cmd = sandbox_command(
        state,
        workspace,
        worktree,
        execution,
        "git",
        &[
            "config".to_string(),
            "--worktree".to_string(),
            "--unset-all".to_string(),
            key.to_string(),
        ],
    )?;
    let output = cmd
        .output()
        .await
        .context("running sandbox git config --unset-all")?;
    if output.status.success() || matches!(output.status.code(), Some(1)) {
        return Ok(());
    }
    bail!(
        "sandbox git config --unset-all failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    )
}
