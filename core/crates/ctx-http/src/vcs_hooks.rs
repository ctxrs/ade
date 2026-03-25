use anyhow::{bail, Context, Result};
use ctx_core::ids::{TaskId, WorkspaceId, WorktreeId};
use ctx_core::models::{VcsKind, Workspace, Worktree};
use std::path::{Path, PathBuf};
use tokio::process::Command;

use crate::execution_effective;
use crate::harness_runtime::{podman_command, workspace_container_name};
use crate::settings::{ContainerRuntimeKind, ExecutionMode};
use crate::worktree_data_plane::live_worktree_root_for_mode;

const COMMIT_MSG_HOOK: &str = r#"#!/bin/sh
set -e

msg_file="$1"
if [ -z "$msg_file" ] || [ ! -f "$msg_file" ]; then
  exit 0
fi

if grep -q "Task-Id:" "$msg_file"; then
  exit 0
fi

task_id="$(git config --worktree --get ctx.taskId || true)"
if [ -z "$task_id" ]; then
  echo "ctx: missing ctx.taskId for commit; set ctx.taskId or create a task." >&2
  exit 1
fi

printf '\nTask-Id: %s\n' "$task_id" >> "$msg_file"
"#;

const CORE_HOOKS_PATH_KEY: &str = "core.hooksPath";
const CTX_TASK_ID_KEY: &str = "ctx.taskId";
const CTX_PREV_HOOKS_PATH_KEY: &str = "ctx.prevHooksPath";

pub fn vcs_hooks_root(data_root: &Path) -> PathBuf {
    data_root.join("vcs-hooks")
}

pub fn worktree_hooks_dir(
    data_root: &Path,
    workspace_id: WorkspaceId,
    worktree_id: WorktreeId,
) -> PathBuf {
    vcs_hooks_root(data_root)
        .join(workspace_id.0.to_string())
        .join(worktree_id.0.to_string())
}

async fn ensure_task_commit_hook_host(
    data_root: &Path,
    workspace_id: WorkspaceId,
    worktree_id: WorktreeId,
    worktree_root: &Path,
    vcs_kind: Option<VcsKind>,
    task_id: TaskId,
) -> Result<()> {
    if vcs_kind != Some(VcsKind::Git) {
        return Ok(());
    }
    if tokio::fs::metadata(worktree_root).await.is_err() {
        return Ok(());
    }

    let hooks_dir = worktree_hooks_dir(data_root, workspace_id, worktree_id);
    tokio::fs::create_dir_all(&hooks_dir)
        .await
        .context("creating vcs hooks dir")?;

    let hook_path = hooks_dir.join("commit-msg");
    tokio::fs::write(&hook_path, COMMIT_MSG_HOOK)
        .await
        .context("writing commit-msg hook")?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut perms = tokio::fs::metadata(&hook_path)
            .await
            .context("loading commit-msg hook metadata")?
            .permissions();
        perms.set_mode(0o755);
        tokio::fs::set_permissions(&hook_path, perms)
            .await
            .context("setting commit-msg hook permissions")?;
    }

    let hooks_path = hooks_dir.to_string_lossy().to_string();
    if let Some(existing) = get_git_config(worktree_root, CORE_HOOKS_PATH_KEY).await? {
        if existing != hooks_path {
            set_git_config(worktree_root, CTX_PREV_HOOKS_PATH_KEY, &existing).await?;
        }
    }
    set_git_config(worktree_root, CORE_HOOKS_PATH_KEY, &hooks_path).await?;
    set_git_config(worktree_root, CTX_TASK_ID_KEY, &task_id.0.to_string()).await?;
    Ok(())
}

pub async fn ensure_task_commit_hook(
    state: &crate::daemon::AppState,
    workspace: &Workspace,
    worktree: &Worktree,
    task_id: TaskId,
) -> Result<()> {
    let settings = execution_effective::effective_execution_settings(state, workspace.id).await?;
    if matches!(settings.mode, ExecutionMode::Host) {
        return ensure_task_commit_hook_host(
            &state.core.data_root,
            workspace.id,
            worktree.id,
            Path::new(&worktree.root_path),
            worktree.vcs_kind.clone(),
            task_id,
        )
        .await;
    }
    if worktree.vcs_kind != Some(VcsKind::Git) {
        return Ok(());
    }

    state
        .execution
        .harness
        .ensure_workspace_container_for_worktree(
            workspace,
            worktree,
            &settings,
            &state.core.daemon_url,
        )
        .await?;

    let live_worktree_root = live_worktree_root_for_mode(
        &state.core.data_root,
        workspace,
        worktree,
        settings.mode.clone(),
    );
    let hooks_dir = live_worktree_root.join(".ctx-hooks");
    let hook_path = hooks_dir.join("commit-msg");
    sandbox_write_hook(
        state, workspace, worktree, &settings, &hooks_dir, &hook_path,
    )
    .await?;

    let hooks_path = hooks_dir.to_string_lossy().to_string();
    if let Some(existing) =
        sandbox_git_config_get(state, workspace, worktree, &settings, CORE_HOOKS_PATH_KEY).await?
    {
        if existing != hooks_path {
            sandbox_git_config_set(
                state,
                workspace,
                worktree,
                &settings,
                CTX_PREV_HOOKS_PATH_KEY,
                &existing,
            )
            .await?;
        }
    }
    sandbox_git_config_set(
        state,
        workspace,
        worktree,
        &settings,
        CORE_HOOKS_PATH_KEY,
        &hooks_path,
    )
    .await?;
    sandbox_git_config_set(
        state,
        workspace,
        worktree,
        &settings,
        CTX_TASK_ID_KEY,
        &task_id.0.to_string(),
    )
    .await?;
    Ok(())
}

pub async fn cleanup_worktree_hooks(
    data_root: &Path,
    workspace_id: WorkspaceId,
    worktree_id: WorktreeId,
    worktree_root: Option<&Path>,
    vcs_kind: Option<VcsKind>,
) -> Result<()> {
    let hooks_dir = worktree_hooks_dir(data_root, workspace_id, worktree_id);
    let hooks_path = hooks_dir.to_string_lossy().to_string();
    if vcs_kind == Some(VcsKind::Git) {
        if let Some(root) = worktree_root {
            if tokio::fs::metadata(root).await.is_ok() {
                if let Some(current) = get_git_config(root, CORE_HOOKS_PATH_KEY).await? {
                    if current == hooks_path {
                        if let Some(prev) = get_git_config(root, CTX_PREV_HOOKS_PATH_KEY).await? {
                            set_git_config(root, CORE_HOOKS_PATH_KEY, &prev).await?;
                        } else {
                            unset_git_config(root, CORE_HOOKS_PATH_KEY).await?;
                        }
                        unset_git_config(root, CTX_TASK_ID_KEY).await?;
                        unset_git_config(root, CTX_PREV_HOOKS_PATH_KEY).await?;
                    }
                }
            }
        }
    }
    if tokio::fs::metadata(&hooks_dir).await.is_ok() {
        tokio::fs::remove_dir_all(&hooks_dir)
            .await
            .context("removing vcs hooks dir")?;
    }
    Ok(())
}

pub async fn cleanup_workspace_hooks(data_root: &Path, workspace_id: WorkspaceId) -> Result<()> {
    let hooks_dir = vcs_hooks_root(data_root).join(workspace_id.0.to_string());
    if tokio::fs::metadata(&hooks_dir).await.is_ok() {
        tokio::fs::remove_dir_all(&hooks_dir)
            .await
            .context("removing workspace vcs hooks dir")?;
    }
    Ok(())
}

async fn set_git_config(worktree_root: &Path, key: &str, value: &str) -> Result<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(worktree_root)
        .arg("config")
        .arg("--worktree")
        .arg(key)
        .arg(value)
        .output()
        .await
        .context("running git config")?;
    if !output.status.success() {
        bail!(
            "git config --worktree failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

async fn sandbox_write_hook(
    state: &crate::daemon::AppState,
    workspace: &Workspace,
    worktree: &Worktree,
    settings: &crate::settings::ExecutionSettings,
    hooks_dir: &Path,
    hook_path: &Path,
) -> Result<()> {
    let script = "set -eu; mkdir -p -- \"$1\"; cat > \"$2\"; chmod 755 \"$2\"";
    let mut cmd = sandbox_command(
        state,
        workspace,
        worktree,
        settings,
        "sh",
        &[
            "-lc".to_string(),
            script.to_string(),
            "--".to_string(),
            hooks_dir.to_string_lossy().to_string(),
            hook_path.to_string_lossy().to_string(),
        ],
    )?;
    cmd.stdin(std::process::Stdio::piped());
    cmd.kill_on_drop(true);
    let mut child = cmd.spawn().context("spawning sandbox vcs hook install")?;
    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        stdin.write_all(COMMIT_MSG_HOOK.as_bytes()).await?;
    }
    let output = child
        .wait_with_output()
        .await
        .context("waiting on sandbox vcs hook install")?;
    if !output.status.success() {
        bail!(
            "sandbox hook install failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

fn sandbox_command(
    state: &crate::daemon::AppState,
    workspace: &Workspace,
    worktree: &Worktree,
    settings: &crate::settings::ExecutionSettings,
    command: &str,
    args: &[String],
) -> Result<Command> {
    let live_worktree_root = live_worktree_root_for_mode(
        &state.core.data_root,
        workspace,
        worktree,
        settings.mode.clone(),
    );
    match settings.container.runtime {
        ContainerRuntimeKind::Podman => {
            let mut cmd = podman_command(&state.core.data_root)?;
            cmd.arg("exec")
                .arg("--interactive")
                .arg("--workdir")
                .arg(&live_worktree_root)
                .arg(workspace_container_name(workspace.id))
                .arg(command);
            cmd.args(args);
            Ok(cmd)
        }
        ContainerRuntimeKind::AvfLinuxVm => {
            crate::workspace_runtime::build_avf_linux_guest_exec_command(
                &state.core.data_root,
                workspace.id,
                worktree.id,
                &live_worktree_root,
                command,
                args,
                &std::collections::HashMap::new(),
                None,
                false,
            )
        }
    }
}

async fn sandbox_git_config_get(
    state: &crate::daemon::AppState,
    workspace: &Workspace,
    worktree: &Worktree,
    settings: &crate::settings::ExecutionSettings,
    key: &str,
) -> Result<Option<String>> {
    let mut cmd = sandbox_command(
        state,
        workspace,
        worktree,
        settings,
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

async fn sandbox_git_config_set(
    state: &crate::daemon::AppState,
    workspace: &Workspace,
    worktree: &Worktree,
    settings: &crate::settings::ExecutionSettings,
    key: &str,
    value: &str,
) -> Result<()> {
    let mut cmd = sandbox_command(
        state,
        workspace,
        worktree,
        settings,
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

async fn get_git_config(worktree_root: &Path, key: &str) -> Result<Option<String>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(worktree_root)
        .arg("config")
        .arg("--worktree")
        .arg("--get")
        .arg(key)
        .output()
        .await
        .context("running git config --get")?;
    if output.status.success() {
        let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if value.is_empty() {
            return Ok(None);
        }
        return Ok(Some(value));
    }
    if matches!(output.status.code(), Some(1)) {
        return Ok(None);
    }
    bail!(
        "git config --get failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn unset_git_config(worktree_root: &Path, key: &str) -> Result<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(worktree_root)
        .arg("config")
        .arg("--worktree")
        .arg("--unset-all")
        .arg(key)
        .output()
        .await
        .context("running git config --unset-all")?;
    if output.status.success() || matches!(output.status.code(), Some(1)) {
        return Ok(());
    }
    bail!(
        "git config --unset-all failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
