use super::*;
use crate::execution_effective;
use crate::settings::ContainerRuntimeKind;
use crate::worktree_data_plane::resolve_worktree_data_plane;
use chrono::Utc;
use ctx_core::models::{AttachmentUpdatePolicy, WorktreeAttachmentStatus};
use ctx_worktree_data_plane::apply_data_plane_to_execution_settings;

mod avf;
mod native;

use avf::{avf_copy_source_to_mount, avf_rm_rf, avf_run_success};
use native::{
    container_ensure_mount, container_mkdir_p, container_path_exists, container_rm_rf,
    ensure_attachment_imported_to_container,
};

fn command_failure_detail(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !stderr.is_empty() {
        stderr
    } else if !stdout.is_empty() {
        stdout
    } else {
        "unknown sandbox command failure".to_string()
    }
}

#[derive(Debug, Clone)]
enum AttachmentRuntime {
    NativeContainer {
        container_id: String,
    },
    SharedVmContainer {
        workspace_id: WorkspaceId,
        worktree_id: WorktreeId,
        worktree_root: PathBuf,
    },
}

async fn ensure_workspace_container_for_attachments(
    state: &AppState,
    workspace: &Workspace,
    worktree: &Worktree,
) -> Result<ContainerRuntimeKind> {
    let effective = execution_effective::effective_execution_settings(state, workspace.id).await?;
    let data_plane = resolve_worktree_data_plane(state, worktree).await?;
    let effective = apply_data_plane_to_execution_settings(&effective, &data_plane)?;
    state
        .execution
        .harness
        .ensure_workspace_container_for_worktree(
            workspace,
            worktree,
            &effective,
            &state.core.daemon_url,
        )
        .await?;
    Ok(effective.container.runtime)
}

async fn attachment_runtime_for_worktree(
    state: &AppState,
    workspace: &Workspace,
    worktree_id: WorktreeId,
    worktree_root: &Path,
) -> Result<AttachmentRuntime> {
    let worktree = state
        .global_store()
        .get_worktree(worktree_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("worktree not found for attachment mount"))?;
    let runtime = ensure_workspace_container_for_attachments(state, workspace, &worktree).await?;
    Ok(match runtime {
        ContainerRuntimeKind::NativeContainer => AttachmentRuntime::NativeContainer {
            container_id: workspace_container_name(workspace.id),
        },
        ContainerRuntimeKind::SharedVmContainer => AttachmentRuntime::SharedVmContainer {
            workspace_id: workspace.id,
            worktree_id,
            worktree_root: worktree_root.to_path_buf(),
        },
    })
}

fn container_attachment_materialized_root(attachment: &WorkspaceAttachment) -> PathBuf {
    PathBuf::from(CTX_CONTAINER_WORKSPACE_ROOT)
        .join(CONTAINER_ATTACHMENTS_SUBDIR)
        .join(attachment.id.0.to_string())
        .join(revision_key(attachment))
}

fn container_attachment_root(attachment: &WorkspaceAttachment) -> PathBuf {
    PathBuf::from(CTX_CONTAINER_WORKSPACE_ROOT)
        .join(CONTAINER_ATTACHMENTS_SUBDIR)
        .join(attachment.id.0.to_string())
}

pub(super) async fn container_ensure_git_exclude(
    state: &AppState,
    workspace: &Workspace,
    worktree_id: WorktreeId,
    worktree_root: &Path,
) -> Result<()> {
    let runtime =
        attachment_runtime_for_worktree(state, workspace, worktree_id, worktree_root).await?;
    let script = r#"
set -e
gitdir="$(git rev-parse --git-dir)"
mkdir -p "$gitdir/info"
path="$gitdir/info/exclude"
touch "$path"
for line in ".ctx/attachments/refs/" ".ctx/attachments/docs/"; do
  if ! grep -Fxq "$line" "$path"; then
    printf '%s\n' "$line" >> "$path"
  fi
done
"#;
    match runtime {
        AttachmentRuntime::NativeContainer { container_id } => {
            let mut cmd = sandbox_container_command(&state.core.data_root)?;
            cmd.arg("exec")
                .arg("--interactive")
                .arg("--workdir")
                .arg(worktree_root)
                .arg(&container_id)
                .arg("sh")
                .arg("-lc")
                .arg(script);
            let out = cmd.output().await.context("sandbox exec git exclude")?;
            if out.status.success() {
                Ok(())
            } else {
                anyhow::bail!(
                    "container git exclude failed (status {}): {}",
                    out.status,
                    String::from_utf8_lossy(&out.stderr).trim()
                );
            }
        }
        AttachmentRuntime::SharedVmContainer {
            workspace_id,
            worktree_id,
            worktree_root,
        } => {
            avf_run_success(
                state,
                workspace_id,
                worktree_id,
                &worktree_root,
                "sh",
                &["-lc".to_string(), script.to_string()],
            )
            .await
        }
    }
}

async fn container_remove_mount_path(
    state: &AppState,
    workspace_id: WorkspaceId,
    worktree_id: WorktreeId,
    target: &Path,
) -> Result<()> {
    let effective = execution_effective::effective_execution_settings(state, workspace_id).await?;
    let store = state.store_for_workspace(workspace_id).await?;
    let Some(worktree) = store.get_worktree(worktree_id).await? else {
        return Ok(());
    };
    let data_plane = resolve_worktree_data_plane(state, &worktree).await?;
    let effective = apply_data_plane_to_execution_settings(&effective, &data_plane)?;
    match effective.container.runtime {
        ContainerRuntimeKind::NativeContainer => {
            let container_id = workspace_container_name(workspace_id);
            // Best-effort: if the container doesn't exist, skip.
            let mut exists = sandbox_container_command(&state.core.data_root)?;
            exists.arg("container").arg("inspect").arg(&container_id);
            let out = exists.output().await.context("container inspect")?;
            if !out.status.success() {
                return Ok(());
            }
            let _ = container_rm_rf(state, &container_id, target).await;
            Ok(())
        }
        ContainerRuntimeKind::SharedVmContainer => {
            let worktree_root = PathBuf::from(worktree.root_path);
            let _ = avf_rm_rf(state, workspace_id, worktree_id, &worktree_root, target).await;
            Ok(())
        }
    }
}

async fn container_remove_attachment_data_best_effort(
    state: &AppState,
    workspace_id: WorkspaceId,
    attachment: &WorkspaceAttachment,
) -> Result<()> {
    let effective = execution_effective::effective_execution_settings(state, workspace_id).await?;
    if matches!(
        effective.container.runtime,
        ContainerRuntimeKind::SharedVmContainer
    ) {
        return Ok(());
    }
    let container_id = workspace_container_name(workspace_id);
    let mut exists = sandbox_container_command(&state.core.data_root)?;
    exists.arg("container").arg("inspect").arg(&container_id);
    let out = exists.output().await.context("container inspect")?;
    if !out.status.success() {
        return Ok(());
    }
    let root = container_attachment_root(attachment);
    let _ = container_rm_rf(state, &container_id, &root).await;
    Ok(())
}

async fn resolve_attachment_source_path(root: &Path, subpath: Option<&str>) -> Result<PathBuf> {
    let root_canonical = tokio::fs::canonicalize(root)
        .await
        .unwrap_or_else(|_| root.to_path_buf());
    let candidate = match subpath {
        Some(subpath) => root.join(subpath),
        None => root.to_path_buf(),
    };
    let candidate_canonical = tokio::fs::canonicalize(&candidate)
        .await
        .with_context(|| format!("attachment source not found at {}", candidate.display()))?;
    if !candidate_canonical.starts_with(&root_canonical) {
        anyhow::bail!("attachment subpath escapes the materialized root");
    }
    Ok(candidate_canonical)
}

pub(crate) async fn ensure_attachment_mount(
    state: &AppState,
    workspace: &Workspace,
    worktree_id: WorktreeId,
    worktree_root: &Path,
    attachment: &WorkspaceAttachment,
    refresh: bool,
    materialize: bool,
) -> Result<WorktreeAttachmentMount> {
    let materialized = if materialize {
        let should_refresh = refresh || attachment.update_policy != AttachmentUpdatePolicy::Manual;
        materialize_attachment(state, workspace, attachment, should_refresh).await?
    } else {
        let path = materialized_path_for_attachment(state, attachment);
        if !path.exists() {
            anyhow::bail!("attachment materialization not found at {}", path.display());
        }
        MaterializationResult {
            path,
            materialized_id: revision_key(attachment),
        }
    };
    let mount_rel = sanitize_mount_relpath(&attachment.mount_relpath)?;
    let mount_abs = worktree_root.join(&mount_rel);
    let worktree = state
        .global_store()
        .get_worktree(worktree_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("worktree not found for attachment mount"))?;
    let data_plane = resolve_worktree_data_plane(state, &worktree).await?;
    let container_mode = matches!(
        data_plane.execution_mode,
        crate::settings::ExecutionMode::Sandbox
    );
    if container_mode {
        let runtime =
            attachment_runtime_for_worktree(state, workspace, worktree_id, worktree_root).await?;
        match runtime {
            AttachmentRuntime::NativeContainer { container_id } => {
                let should_refresh =
                    refresh || attachment.update_policy != AttachmentUpdatePolicy::Manual;
                let imported = ensure_attachment_imported_to_container(
                    state,
                    &container_id,
                    attachment,
                    &materialized.path,
                    should_refresh,
                )
                .await?;
                let source_path =
                    resolve_attachment_source_path(&imported, attachment.subpath.as_deref())
                        .await?;
                container_ensure_mount(state, &container_id, &mount_abs, &source_path).await?;
            }
            AttachmentRuntime::SharedVmContainer {
                workspace_id,
                worktree_id,
                worktree_root,
            } => {
                let source_path = resolve_attachment_source_path(
                    &materialized.path,
                    attachment.subpath.as_deref(),
                )
                .await?;
                avf_copy_source_to_mount(
                    state,
                    workspace_id,
                    worktree_id,
                    &worktree_root,
                    &source_path,
                    &mount_abs,
                )
                .await?;
            }
        }
    } else {
        if let Some(parent) = mount_abs.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let source_path =
            resolve_attachment_source_path(&materialized.path, attachment.subpath.as_deref())
                .await?;
        ensure_mount(&mount_abs, &source_path).await?;
    }

    let now = Utc::now();
    let mount = WorktreeAttachmentMount {
        worktree_id,
        attachment_id: attachment.id,
        mount_abs_path: mount_abs.to_string_lossy().to_string(),
        materialized_id: materialized.materialized_id,
        status: WorktreeAttachmentStatus::Ready,
        last_sync_at: Some(now),
        error_message: None,
        created_at: now,
        updated_at: now,
    };
    let store = state
        .store_for_workspace(workspace.id)
        .await
        .context("load workspace store for attachment mount update")?;
    store.upsert_worktree_attachment_mount(&mount).await?;
    Ok(mount)
}

pub(crate) async fn cleanup_removed_attachment(
    state: &AppState,
    attachment: &WorkspaceAttachment,
) -> Result<()> {
    let store = state.store_for_workspace(attachment.workspace_id).await?;
    let mounts = store
        .list_worktree_attachment_mounts_for_attachment(attachment.id)
        .await?;
    for mount in mounts {
        let path = PathBuf::from(&mount.mount_abs_path);
        let worktree = state.global_store().get_worktree(mount.worktree_id).await?;
        let container_mode = match worktree {
            Some(worktree) => {
                let data_plane = resolve_worktree_data_plane(state, &worktree).await?;
                matches!(
                    data_plane.execution_mode,
                    crate::settings::ExecutionMode::Sandbox
                )
            }
            None => store
                .get_sandbox_binding(mount.worktree_id)
                .await?
                .is_some(),
        };
        if container_mode {
            let _ = container_remove_mount_path(
                state,
                attachment.workspace_id,
                mount.worktree_id,
                &path,
            )
            .await;
        } else {
            remove_mount_path(&path).await?;
        }
    }
    store
        .delete_worktree_attachment_mounts_for_attachment(attachment.id)
        .await?;
    let root = materialized_root_for_attachment(state, attachment);
    if root.exists() {
        tokio::fs::remove_dir_all(root).await?;
    }
    let _ =
        container_remove_attachment_data_best_effort(state, attachment.workspace_id, attachment)
            .await;
    Ok(())
}

#[cfg(test)]
mod tests;
