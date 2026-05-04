use super::*;
use crate::execution_effective;
use crate::settings::{ContainerRuntimeKind, ExecutionMode};
use crate::worktree_data_plane::resolve_worktree_data_plane;
use chrono::Utc;
use ctx_core::models::{AttachmentMode, AttachmentUpdatePolicy, WorktreeAttachmentStatus};
use ctx_worktree_data_plane::apply_data_plane_to_execution_settings;

mod avf;
mod native;
mod sandbox_parent_chain;
mod source_resolution;
mod tree_validation;

use avf::{avf_copy_source_to_mount, avf_remove_mount_path_in_worktree, avf_run_success};
use native::{
    container_ensure_mount, container_remove_mount_path_in_worktree, container_rm_rf,
    ensure_attachment_imported_to_container,
};
#[cfg(test)]
use sandbox_parent_chain::sandbox_mount_parent_chain_ensure_test_script;
use sandbox_parent_chain::sandbox_mount_parent_chain_functions_script;
use source_resolution::{
    container_path_for_resolved_source, resolve_attachment_source_path, symlink_policy_for_mode,
    AttachmentSourceSymlinkPolicy,
};
use tree_validation::{
    validate_attachment_tree_within_root, validate_read_only_attachment_import_tree,
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
            container_remove_mount_path_in_worktree(
                state,
                &container_id,
                &data_plane.live_worktree_root,
                target,
            )
            .await?;
            Ok(())
        }
        ContainerRuntimeKind::SharedVmContainer => {
            let worktree_root = data_plane.live_worktree_root;
            avf_remove_mount_path_in_worktree(
                state,
                workspace_id,
                worktree_id,
                &worktree_root,
                target,
            )
            .await?;
            Ok(())
        }
    }
}

async fn container_remove_attachment_data_if_present(
    state: &AppState,
    workspace_id: WorkspaceId,
    attachment: &WorkspaceAttachment,
) -> Result<()> {
    let effective = execution_effective::effective_execution_settings(state, workspace_id).await?;
    if matches!(effective.mode, ExecutionMode::Host) {
        return Ok(());
    }
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
    container_rm_rf(state, &container_id, &root).await
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
        workspace_attachments::validate_materialized_path(&state.core.data_root, attachment)
            .await?;
        MaterializationResult {
            path,
            materialized_id: revision_key(attachment),
        }
    };
    let mount_rel = sanitize_mount_relpath(&attachment.mount_relpath)?;
    let mount_abs = worktree_root.join(&mount_rel);
    validate_mount_path_in_worktree(worktree_root, &mount_abs)?;
    let symlink_policy = symlink_policy_for_mode(&attachment.mode);
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
                let host_source_path = resolve_attachment_source_path(
                    &materialized.path,
                    attachment.subpath.as_deref(),
                    symlink_policy,
                )
                .await?;
                if matches!(&attachment.mode, AttachmentMode::Ro) {
                    validate_read_only_attachment_import_tree(&materialized.path).await?;
                }
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
                let source_path = container_path_for_resolved_source(
                    &materialized.path,
                    &host_source_path,
                    &imported,
                )?;
                container_ensure_mount(
                    state,
                    &container_id,
                    worktree_root,
                    &mount_abs,
                    &source_path,
                    attachment.mode.clone(),
                )
                .await?;
            }
            AttachmentRuntime::SharedVmContainer {
                workspace_id,
                worktree_id,
                worktree_root,
            } => {
                let source_path = resolve_attachment_source_path(
                    &materialized.path,
                    attachment.subpath.as_deref(),
                    symlink_policy,
                )
                .await?;
                avf_copy_source_to_mount(
                    state,
                    workspace_id,
                    worktree_id,
                    &worktree_root,
                    &source_path,
                    &mount_abs,
                    attachment.mode.clone(),
                )
                .await?;
            }
        }
    } else {
        let source_path = resolve_attachment_source_path(
            &materialized.path,
            attachment.subpath.as_deref(),
            symlink_policy,
        )
        .await?;
        let checked_mount_abs = ensure_mount_in_worktree(
            worktree_root,
            &mount_rel,
            &source_path,
            attachment.mode.clone(),
        )
        .await?;
        debug_assert_eq!(checked_mount_abs, mount_abs);
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
        let container_mode = match &worktree {
            Some(worktree) => {
                let data_plane = resolve_worktree_data_plane(state, worktree).await?;
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
            let Some(worktree) = worktree.as_ref() else {
                anyhow::bail!(
                    "cannot safely remove sandbox attachment mount without worktree metadata"
                );
            };
            let data_plane = resolve_worktree_data_plane(state, worktree).await?;
            validate_mount_path_in_worktree(&data_plane.live_worktree_root, &path)?;
            container_remove_mount_path(state, attachment.workspace_id, mount.worktree_id, &path)
                .await?;
        } else {
            let Some(worktree) = worktree.as_ref() else {
                anyhow::bail!(
                    "cannot safely remove host attachment mount without worktree metadata"
                );
            };
            let data_plane = resolve_worktree_data_plane(state, worktree).await?;
            remove_mount_path_in_worktree(&data_plane.live_worktree_root, &path).await?;
        }
    }
    container_remove_attachment_data_if_present(state, attachment.workspace_id, attachment).await?;
    store
        .delete_worktree_attachment_mounts_for_attachment(attachment.id)
        .await?;
    workspace_attachments::remove_materialized_root_if_exists(&state.core.data_root, attachment)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests;
