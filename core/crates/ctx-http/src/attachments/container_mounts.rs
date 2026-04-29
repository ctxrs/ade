use super::*;
use crate::execution_effective;
use crate::settings::ContainerRuntimeKind;
use crate::worktree_data_plane::resolve_worktree_data_plane;
use chrono::Utc;
use ctx_core::models::{AttachmentMode, AttachmentUpdatePolicy, WorktreeAttachmentStatus};
use ctx_worktree_data_plane::apply_data_plane_to_execution_settings;

mod avf;
mod native;

use avf::{avf_copy_source_to_mount, avf_remove_mount_path_in_worktree, avf_run_success};
use native::{
    container_ensure_mount, container_remove_mount_path_in_worktree, container_rm_rf,
    ensure_attachment_imported_to_container,
};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum AttachmentSourceSymlinkPolicy {
    AllowInternal,
    Reject,
}

fn symlink_policy_for_mode(mode: &AttachmentMode) -> AttachmentSourceSymlinkPolicy {
    match mode {
        AttachmentMode::Ro => AttachmentSourceSymlinkPolicy::Reject,
        AttachmentMode::Rw => AttachmentSourceSymlinkPolicy::AllowInternal,
    }
}

fn sandbox_mount_parent_chain_functions_script() -> &'static str {
    r#"
ensure_mount_parent_chain() {
  root="$1"
  target="$2"
  if [ -L "$root" ]; then
    printf 'attachment mount worktree root must not be a symlink: %s\n' "$root" >&2
    exit 2
  fi
  if [ ! -d "$root" ]; then
    printf 'attachment mount worktree root must be a directory: %s\n' "$root" >&2
    exit 2
  fi
  case "$target" in
    "$root"/*) rel="${target#"$root"/}" ;;
    *)
      printf 'attachment mount path escapes sandbox worktree: %s\n' "$target" >&2
      exit 2
      ;;
  esac
  parent="${rel%/*}"
  if [ "$parent" = "$rel" ]; then
    return 0
  fi
  current="$root"
  remaining="$parent"
  while [ -n "$remaining" ]; do
    segment="${remaining%%/*}"
    if [ "$remaining" = "$segment" ]; then
      remaining=""
    else
      remaining="${remaining#*/}"
    fi
    if [ -z "$segment" ] || [ "$segment" = "." ] || [ "$segment" = ".." ]; then
      printf 'attachment mount path contains unsupported component: %s\n' "$target" >&2
      exit 2
    fi
    current="$current/$segment"
    if [ -L "$current" ]; then
      printf 'attachment mount parent must not be a symlink: %s\n' "$current" >&2
      exit 2
    fi
    if [ ! -e "$current" ]; then
      mkdir "$current" 2>/dev/null || true
    fi
    if [ -L "$current" ]; then
      printf 'attachment mount parent must not be a symlink: %s\n' "$current" >&2
      exit 2
    fi
    if [ ! -d "$current" ]; then
      printf 'attachment mount parent must be a directory: %s\n' "$current" >&2
      exit 2
    fi
  done
}

remove_mount_path_if_parent_safe() {
  root="$1"
  target="$2"
  if [ -L "$root" ]; then
    printf 'attachment mount worktree root must not be a symlink: %s\n' "$root" >&2
    exit 2
  fi
  if [ ! -d "$root" ]; then
    exit 0
  fi
  case "$target" in
    "$root"/*) rel="${target#"$root"/}" ;;
    *)
      printf 'attachment mount path escapes sandbox worktree: %s\n' "$target" >&2
      exit 2
      ;;
  esac
  parent="${rel%/*}"
  if [ "$parent" != "$rel" ]; then
    current="$root"
    remaining="$parent"
    while [ -n "$remaining" ]; do
      segment="${remaining%%/*}"
      if [ "$remaining" = "$segment" ]; then
        remaining=""
      else
        remaining="${remaining#*/}"
      fi
      if [ -z "$segment" ] || [ "$segment" = "." ] || [ "$segment" = ".." ]; then
        printf 'attachment mount path contains unsupported component: %s\n' "$target" >&2
        exit 2
      fi
      current="$current/$segment"
      if [ -L "$current" ]; then
        printf 'attachment mount parent must not be a symlink: %s\n' "$current" >&2
        exit 2
      fi
      if [ ! -e "$current" ]; then
        exit 0
      fi
      if [ ! -d "$current" ]; then
        printf 'attachment mount parent must be a directory: %s\n' "$current" >&2
        exit 2
      fi
    done
  fi
  if [ -L "$target" ]; then
    :
  elif [ -e "$target" ]; then
    chmod -R u+w -- "$target"
  fi
  rm -rf -- "$target"
}
"#
}

#[cfg(test)]
fn sandbox_mount_parent_chain_ensure_test_script() -> String {
    format!(
        "{}\nensure_mount_parent_chain \"$1\" \"$2\"\n",
        sandbox_mount_parent_chain_functions_script()
    )
}

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

async fn resolve_attachment_source_path(
    root: &Path,
    subpath: Option<&str>,
    symlink_policy: AttachmentSourceSymlinkPolicy,
) -> Result<PathBuf> {
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
    validate_attachment_tree_within_root(&root_canonical, &candidate_canonical, symlink_policy)
        .await?;
    Ok(candidate_canonical)
}

async fn validate_attachment_tree_within_root(
    root: &Path,
    candidate: &Path,
    symlink_policy: AttachmentSourceSymlinkPolicy,
) -> Result<()> {
    let root = root.to_path_buf();
    let candidate = candidate.to_path_buf();
    tokio::task::spawn_blocking(move || {
        validate_attachment_tree_within_root_blocking(&root, &candidate, symlink_policy)
    })
    .await??;
    Ok(())
}

fn validate_attachment_tree_within_root_blocking(
    root: &Path,
    candidate: &Path,
    symlink_policy: AttachmentSourceSymlinkPolicy,
) -> Result<()> {
    let metadata = std::fs::symlink_metadata(candidate)
        .with_context(|| format!("reading attachment metadata {}", candidate.display()))?;
    if metadata.file_type().is_symlink() {
        match symlink_policy {
            AttachmentSourceSymlinkPolicy::AllowInternal => {
                validate_attachment_symlink_target(root, candidate)?;
            }
            AttachmentSourceSymlinkPolicy::Reject => {
                anyhow::bail!(
                    "read-only attachment copy refuses symlink: {}",
                    candidate.display()
                );
            }
        }
        return Ok(());
    }
    if metadata.is_dir() {
        for entry in std::fs::read_dir(candidate)
            .with_context(|| format!("reading attachment dir {}", candidate.display()))?
        {
            let entry = entry?;
            validate_attachment_tree_within_root_blocking(root, &entry.path(), symlink_policy)?;
        }
    }
    Ok(())
}

fn container_path_for_resolved_source(
    materialized_root: &Path,
    resolved_source: &Path,
    imported_root: &Path,
) -> Result<PathBuf> {
    let root_canonical = std::fs::canonicalize(materialized_root).with_context(|| {
        format!(
            "canonicalizing attachment materialized root {}",
            materialized_root.display()
        )
    })?;
    let relative = resolved_source
        .strip_prefix(&root_canonical)
        .with_context(|| {
            format!(
                "resolved attachment source {} is outside materialized root {}",
                resolved_source.display(),
                root_canonical.display()
            )
        })?;
    if relative.as_os_str().is_empty() {
        Ok(imported_root.to_path_buf())
    } else {
        Ok(imported_root.join(relative))
    }
}

fn validate_attachment_symlink_target(root: &Path, path: &Path) -> Result<()> {
    let target = std::fs::read_link(path)
        .with_context(|| format!("reading attachment symlink {}", path.display()))?;
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("attachment path missing parent for symlink validation"))?;
    let lexical = resolve_attachment_path_lexical(root, parent, &target)?;
    if let Ok(canonical) = std::fs::canonicalize(&lexical) {
        if !canonical.starts_with(root) {
            anyhow::bail!(
                "attachment symlink escapes the materialized root: {} -> {}",
                path.display(),
                target.display()
            );
        }
    }
    Ok(())
}

fn resolve_attachment_path_lexical(root: &Path, base: &Path, target: &Path) -> Result<PathBuf> {
    let candidate = if target.is_absolute() {
        target.to_path_buf()
    } else {
        base.join(target)
    };
    let mut is_abs = false;
    let mut parts = Vec::new();
    for component in candidate.components() {
        use std::path::Component;
        match component {
            Component::Prefix(_) => anyhow::bail!("unsupported attachment path prefix"),
            Component::RootDir => {
                is_abs = true;
                parts.clear();
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if parts.is_empty() {
                    continue;
                }
                parts.pop();
            }
            Component::Normal(segment) => parts.push(segment.to_os_string()),
        }
    }
    let mut normalized = PathBuf::new();
    if is_abs {
        normalized.push(std::path::MAIN_SEPARATOR.to_string());
    }
    for part in parts {
        normalized.push(part);
    }
    if !normalized.starts_with(root) {
        anyhow::bail!(
            "attachment symlink escapes the materialized root: {}",
            target.display()
        );
    }
    Ok(normalized)
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
            let _ = container_remove_mount_path(
                state,
                attachment.workspace_id,
                mount.worktree_id,
                &path,
            )
            .await;
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
    store
        .delete_worktree_attachment_mounts_for_attachment(attachment.id)
        .await?;
    workspace_attachments::remove_materialized_root_if_exists(&state.core.data_root, attachment)
        .await?;
    let _ =
        container_remove_attachment_data_best_effort(state, attachment.workspace_id, attachment)
            .await;
    Ok(())
}

#[cfg(test)]
mod tests;
