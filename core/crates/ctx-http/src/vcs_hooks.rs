use anyhow::{bail, Context, Result};
use ctx_core::ids::{TaskId, WorkspaceId, WorktreeId};
use ctx_core::models::VcsKind;
use std::path::{Path, PathBuf};
use tokio::process::Command;

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

pub async fn ensure_task_commit_hook(
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
