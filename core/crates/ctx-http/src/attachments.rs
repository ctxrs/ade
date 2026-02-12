use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;
use tokio::process::Command;
use toml::Value as TomlValue;

use ctx_core::ids::{WorkspaceAttachmentId, WorkspaceId, WorktreeId};
use ctx_core::models::{
    AttachmentMode, AttachmentUpdatePolicy, Workspace, WorkspaceAttachment,
    WorkspaceAttachmentKind, WorkspaceAttachmentStatus, Worktree, WorktreeAttachmentMount,
    WorktreeAttachmentStatus,
};

use crate::container_fs::is_container_path;
use crate::daemon::AppState;
use crate::execution_effective;
use crate::harness_runtime::{
    podman_command, workspace_container_name, CTX_CONTAINER_WORKSPACE_ROOT,
};

const ATTACHMENTS_CONFIG_PATH: &str = ".ctx/attachments.toml";
const CONTAINER_ATTACHMENTS_SUBDIR: &str = "attachments";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AttachmentsConfigFile {
    #[serde(default)]
    attachments: Vec<AttachmentConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentConfig {
    pub kind: WorkspaceAttachmentKind,
    pub name: String,
    pub source: String,
    #[serde(default)]
    pub revision: Option<String>,
    #[serde(default)]
    pub subpath: Option<String>,
    #[serde(default)]
    pub mount_relpath: Option<String>,
    #[serde(default)]
    pub mode: Option<AttachmentMode>,
    #[serde(default)]
    pub update_policy: Option<AttachmentUpdatePolicy>,
}

#[derive(Debug, Clone)]
struct MaterializationResult {
    path: PathBuf,
    materialized_id: String,
}

#[derive(Debug, Clone)]
struct AttachmentSyncPlan {
    id: WorkspaceAttachmentId,
    refresh: bool,
}

pub async fn sync_workspace_attachments(
    state: Arc<AppState>,
    workspace: &Workspace,
    refresh: bool,
) -> Result<Vec<WorkspaceAttachment>> {
    let cfg = load_attachments_config(Path::new(&workspace.root_path)).await?;
    let store = state.store_for_workspace(workspace.id).await?;
    let existing = store.list_workspace_attachments(workspace.id).await?;
    if cfg.is_none() {
        for attachment in existing {
            cleanup_removed_attachment(state.as_ref(), &attachment).await?;
            store.delete_workspace_attachment(attachment.id).await?;
        }
        return Ok(vec![]);
    }
    let cfg = cfg.expect("attachments config missing");

    let mut existing_map: HashMap<(WorkspaceAttachmentKind, String), WorkspaceAttachment> =
        HashMap::new();
    for attachment in existing {
        existing_map.insert(
            (attachment.kind.clone(), attachment.name.clone()),
            attachment,
        );
    }

    let mut keep_ids = HashSet::new();
    let mut out = Vec::with_capacity(cfg.attachments.len());
    let mut sync_plans = Vec::new();
    for entry in cfg.attachments {
        let mut attachment =
            normalize_attachment_config(workspace.id, entry, |key| existing_map.remove(key));
        let should_refresh = refresh || attachment.update_policy != AttachmentUpdatePolicy::Manual;
        let should_materialize = should_refresh
            || !materialized_path_for_attachment(state.as_ref(), &attachment).exists();
        if should_materialize && attachment.status != WorkspaceAttachmentStatus::Syncing {
            attachment.status = WorkspaceAttachmentStatus::Pending;
            attachment.error_message = None;
            sync_plans.push(AttachmentSyncPlan {
                id: attachment.id,
                refresh: should_refresh,
            });
        }
        keep_ids.insert(attachment.id);
        store.upsert_workspace_attachment(&attachment).await?;
        out.push(attachment);
    }

    let mut removed = Vec::new();
    for (_, attachment) in existing_map {
        if !keep_ids.contains(&attachment.id) {
            removed.push(attachment);
        }
    }

    for attachment in removed {
        cleanup_removed_attachment(state.as_ref(), &attachment).await?;
        store.delete_workspace_attachment(attachment.id).await?;
    }

    for plan in sync_plans {
        spawn_attachment_materialization(
            Arc::clone(&state),
            workspace.clone(),
            plan.id,
            plan.refresh,
        );
    }

    Ok(out)
}

fn spawn_attachment_materialization(
    state: Arc<AppState>,
    workspace: Workspace,
    attachment_id: WorkspaceAttachmentId,
    refresh: bool,
) {
    tokio::spawn(async move {
        let store = match state.store_for_workspace(workspace.id).await {
            Ok(store) => store,
            Err(err) => {
                tracing::warn!("attachment sync store load failed: {err:#}");
                return;
            }
        };
        let attachment = match store.get_workspace_attachment(attachment_id).await {
            Ok(Some(attachment)) => attachment,
            Ok(None) => return,
            Err(err) => {
                tracing::warn!("attachment sync lookup failed: {err:#}");
                return;
            }
        };

        let now = Utc::now();
        if let Err(err) = store
            .update_workspace_attachment_status(
                attachment_id,
                WorkspaceAttachmentStatus::Syncing,
                None,
                None,
                now,
            )
            .await
        {
            tracing::warn!("attachment sync status update failed: {err:#}");
            return;
        }

        match materialize_attachment(&state, &workspace, &attachment, refresh).await {
            Ok(_) => {
                let now = Utc::now();
                if let Err(err) = store
                    .update_workspace_attachment_status(
                        attachment_id,
                        WorkspaceAttachmentStatus::Ready,
                        Some(now),
                        None,
                        now,
                    )
                    .await
                {
                    tracing::warn!("attachment sync status update failed: {err:#}");
                    return;
                }
                let _ = ensure_workspace_attachments_for_worktrees_with_attachments(
                    &state,
                    &workspace,
                    &[attachment],
                    false,
                    false,
                )
                .await;
            }
            Err(err) => {
                let now = Utc::now();
                let _ = store
                    .update_workspace_attachment_status(
                        attachment_id,
                        WorkspaceAttachmentStatus::Error,
                        None,
                        Some(err.to_string()),
                        now,
                    )
                    .await;
            }
        }
    });
}

pub async fn ensure_worktree_attachment_mounts(
    state: &AppState,
    workspace: &Workspace,
    worktree: &Worktree,
    refresh: bool,
) -> Result<Vec<WorktreeAttachmentMount>> {
    let store = state.store_for_workspace(workspace.id).await?;
    let attachments = store.list_workspace_attachments(workspace.id).await?;
    ensure_worktree_attachment_mounts_for_attachments(
        state,
        workspace,
        worktree,
        &attachments,
        refresh,
        true,
    )
    .await
}

pub async fn ensure_worktree_attachment_mounts_if_materialized(
    state: &AppState,
    workspace: &Workspace,
    worktree: &Worktree,
) -> Result<Vec<WorktreeAttachmentMount>> {
    let store = state.store_for_workspace(workspace.id).await?;
    let attachments = store.list_workspace_attachments(workspace.id).await?;
    let ready = attachments
        .into_iter()
        .filter(|attachment| attachment.status == WorkspaceAttachmentStatus::Ready)
        .filter(|attachment| materialized_path_for_attachment(state, attachment).exists())
        .collect::<Vec<_>>();
    ensure_worktree_attachment_mounts_for_attachments(
        state, workspace, worktree, &ready, false, false,
    )
    .await
}

pub async fn ensure_worktree_attachment_mounts_for_attachments(
    state: &AppState,
    workspace: &Workspace,
    worktree: &Worktree,
    attachments: &[WorkspaceAttachment],
    refresh: bool,
    materialize: bool,
) -> Result<Vec<WorktreeAttachmentMount>> {
    if attachments.is_empty() {
        return Ok(vec![]);
    }

    let store = state.store_for_workspace(workspace.id).await?;

    let worktree_root = PathBuf::from(&worktree.root_path);
    ensure_git_exclude(state, workspace, &worktree_root).await?;

    let mut mounts = Vec::with_capacity(attachments.len());
    for attachment in attachments {
        match ensure_attachment_mount(
            state,
            workspace,
            worktree.id,
            &worktree_root,
            attachment,
            refresh,
            materialize,
        )
        .await
        {
            Ok(mount) => mounts.push(mount),
            Err(e) => {
                let now = Utc::now();
                let mount = WorktreeAttachmentMount {
                    worktree_id: worktree.id,
                    attachment_id: attachment.id,
                    mount_abs_path: worktree_root
                        .join(&attachment.mount_relpath)
                        .to_string_lossy()
                        .to_string(),
                    materialized_id: revision_key(attachment),
                    status: WorktreeAttachmentStatus::Error,
                    last_sync_at: Some(now),
                    error_message: Some(e.to_string()),
                    created_at: now,
                    updated_at: now,
                };
                store.upsert_worktree_attachment_mount(&mount).await?;
                mounts.push(mount);
            }
        }
    }

    Ok(mounts)
}

pub async fn ensure_workspace_attachments_for_worktrees(
    state: &AppState,
    workspace: &Workspace,
    refresh: bool,
) -> Result<()> {
    let store = state.store_for_workspace(workspace.id).await?;
    let attachments = store.list_workspace_attachments(workspace.id).await?;
    ensure_workspace_attachments_for_worktrees_with_attachments(
        state,
        workspace,
        &attachments,
        refresh,
        true,
    )
    .await
}

pub async fn ensure_workspace_attachments_for_worktrees_with_attachments(
    state: &AppState,
    workspace: &Workspace,
    attachments: &[WorkspaceAttachment],
    refresh: bool,
    materialize: bool,
) -> Result<()> {
    let store = state.store_for_workspace(workspace.id).await?;
    let worktrees = store.list_worktrees(workspace.id).await?;
    for worktree in worktrees {
        let _ = ensure_worktree_attachment_mounts_for_attachments(
            state,
            workspace,
            &worktree,
            attachments,
            refresh,
            materialize,
        )
        .await;
    }
    Ok(())
}

async fn load_attachments_config(workspace_root: &Path) -> Result<Option<AttachmentsConfigFile>> {
    let path = workspace_root.join(ATTACHMENTS_CONFIG_PATH);
    if !path.exists() {
        return Ok(None);
    }
    let txt = tokio::fs::read_to_string(&path)
        .await
        .with_context(|| format!("reading {}", path.display()))?;
    let cfg: AttachmentsConfigFile = toml::from_str(&txt).context("parsing attachments.toml")?;
    Ok(Some(cfg))
}

pub async fn upsert_attachment_config(
    workspace_root: &Path,
    new_attachment: AttachmentConfig,
) -> Result<()> {
    let mut cfg = load_attachments_config(workspace_root)
        .await?
        .unwrap_or(AttachmentsConfigFile {
            attachments: Vec::new(),
        });
    let mut replaced = false;
    for entry in cfg.attachments.iter_mut() {
        if entry.kind == new_attachment.kind && entry.name == new_attachment.name {
            *entry = new_attachment.clone();
            replaced = true;
            break;
        }
    }
    if !replaced {
        cfg.attachments.push(new_attachment);
    }
    write_attachments_config(workspace_root, &cfg).await?;
    Ok(())
}

pub async fn remove_attachment_config(
    workspace_root: &Path,
    kind: WorkspaceAttachmentKind,
    name: &str,
) -> Result<bool> {
    let Some(mut cfg) = load_attachments_config(workspace_root).await? else {
        return Ok(false);
    };
    let trimmed = name.trim();
    let before = cfg.attachments.len();
    cfg.attachments
        .retain(|entry| !(entry.kind == kind && entry.name.trim() == trimmed));
    if cfg.attachments.len() == before {
        return Ok(false);
    }
    if cfg.attachments.is_empty() {
        let path = workspace_root.join(ATTACHMENTS_CONFIG_PATH);
        if path.exists() {
            tokio::fs::remove_file(&path).await?;
        }
        return Ok(true);
    }
    write_attachments_config(workspace_root, &cfg).await?;
    Ok(true)
}

async fn write_attachments_config(
    workspace_root: &Path,
    cfg: &AttachmentsConfigFile,
) -> Result<()> {
    let path = workspace_root.join(ATTACHMENTS_CONFIG_PATH);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let txt = toml::to_string_pretty(cfg).context("serializing attachments config")?;
    tokio::fs::write(&path, txt)
        .await
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

fn normalize_attachment_config(
    workspace_id: WorkspaceId,
    cfg: AttachmentConfig,
    mut take_existing: impl FnMut(&(WorkspaceAttachmentKind, String)) -> Option<WorkspaceAttachment>,
) -> WorkspaceAttachment {
    let name = cfg.name.trim().to_string();
    let key = (cfg.kind.clone(), name.clone());
    let now = Utc::now();
    let (id, created_at, status, last_sync_at, error_message) = match take_existing(&key) {
        Some(existing) => (
            existing.id,
            existing.created_at,
            existing.status,
            existing.last_sync_at,
            existing.error_message,
        ),
        None => (
            WorkspaceAttachmentId::new(),
            now,
            WorkspaceAttachmentStatus::Pending,
            None,
            None,
        ),
    };

    let mount_relpath = cfg
        .mount_relpath
        .clone()
        .unwrap_or_else(|| default_mount_relpath(&cfg.kind, &name));

    WorkspaceAttachment {
        id,
        workspace_id,
        kind: cfg.kind,
        name,
        source: cfg.source.trim().to_string(),
        revision: cfg
            .revision
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty()),
        subpath: cfg
            .subpath
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty()),
        mount_relpath,
        mode: cfg.mode.unwrap_or(AttachmentMode::Ro),
        update_policy: cfg.update_policy.unwrap_or(AttachmentUpdatePolicy::Manual),
        status,
        last_sync_at,
        error_message,
        created_at,
        updated_at: now,
    }
}

async fn ensure_workspace_container_for_attachments(
    state: &AppState,
    workspace: &Workspace,
) -> Result<String> {
    let effective = execution_effective::effective_execution_settings(state, workspace.id).await?;
    state
        .execution
        .harness
        .ensure_workspace_container(workspace, &effective, &state.core.daemon_url)
        .await?;
    Ok(workspace_container_name(workspace.id))
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

async fn container_path_exists(state: &AppState, container_id: &str, path: &Path) -> Result<bool> {
    let mut cmd = podman_command(&state.core.data_root)?;
    cmd.arg("exec")
        .arg("--interactive")
        .arg(container_id)
        .arg("test")
        .arg("-e")
        .arg("--")
        .arg(path);
    let out = cmd.output().await.context("podman exec test -e")?;
    Ok(out.status.success())
}

async fn container_rm_rf(state: &AppState, container_id: &str, path: &Path) -> Result<()> {
    let mut cmd = podman_command(&state.core.data_root)?;
    cmd.arg("exec")
        .arg("--interactive")
        .arg(container_id)
        .arg("rm")
        .arg("-rf")
        .arg("--")
        .arg(path);
    let out = cmd.output().await.context("podman exec rm -rf")?;
    if out.status.success() {
        Ok(())
    } else {
        anyhow::bail!(
            "container rm -rf failed (status {}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
}

async fn container_mkdir_p(state: &AppState, container_id: &str, path: &Path) -> Result<()> {
    let mut cmd = podman_command(&state.core.data_root)?;
    cmd.arg("exec")
        .arg("--interactive")
        .arg(container_id)
        .arg("mkdir")
        .arg("-p")
        .arg("--")
        .arg(path);
    let out = cmd.output().await.context("podman exec mkdir -p")?;
    if out.status.success() {
        Ok(())
    } else {
        anyhow::bail!(
            "container mkdir -p failed (status {}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
}

async fn import_dir_to_container(
    state: &AppState,
    container_id: &str,
    src: &Path,
    dest: &Path,
) -> Result<()> {
    // Stream a tar archive into the container so extracted files are writable by the execution
    // user (avoids `podman cp` ownership quirks).
    let mut tar_cmd = Command::new("tar");
    tar_cmd.arg("-C").arg(src).arg("-cf").arg("-").arg(".");
    tar_cmd.stdout(Stdio::piped());
    let mut tar_child = tar_cmd.spawn().context("spawning tar")?;
    let mut tar_out = tar_child.stdout.take().context("taking tar stdout")?;

    let mut pod_cmd = podman_command(&state.core.data_root)?;
    pod_cmd
        .arg("exec")
        .arg("--interactive")
        .arg("--workdir")
        .arg(dest)
        .arg(container_id)
        .arg("tar")
        .arg("-xf")
        .arg("-");
    pod_cmd.stdin(Stdio::piped());
    let mut pod_child = pod_cmd.spawn().context("spawning podman exec tar")?;
    let mut pod_in = pod_child.stdin.take().context("taking podman exec stdin")?;

    tokio::io::copy(&mut tar_out, &mut pod_in)
        .await
        .context("streaming tar to podman exec")?;
    drop(pod_in);

    let tar_status = tar_child.wait().await.context("waiting on tar")?;
    if !tar_status.success() {
        anyhow::bail!("tar failed with status {tar_status}");
    }
    let out = pod_child
        .wait_with_output()
        .await
        .context("waiting on podman exec tar")?;
    if !out.status.success() {
        anyhow::bail!(
            "podman exec tar failed (status {}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

async fn ensure_attachment_imported_to_container(
    state: &AppState,
    workspace: &Workspace,
    attachment: &WorkspaceAttachment,
    src_dir: &Path,
    refresh: bool,
) -> Result<PathBuf> {
    let container_id = ensure_workspace_container_for_attachments(state, workspace).await?;
    let dest = container_attachment_materialized_root(attachment);
    let exists = if refresh {
        false
    } else {
        container_path_exists(state, &container_id, &dest)
            .await
            .unwrap_or(false)
    };
    if exists {
        return Ok(dest);
    }
    // Reset and re-import.
    let _ = container_rm_rf(state, &container_id, &dest).await;
    container_mkdir_p(state, &container_id, &dest).await?;
    import_dir_to_container(state, &container_id, src_dir, &dest).await?;
    Ok(dest)
}

async fn container_ensure_mount(
    state: &AppState,
    workspace: &Workspace,
    target: &Path,
    source: &Path,
) -> Result<()> {
    let container_id = ensure_workspace_container_for_attachments(state, workspace).await?;
    if let Some(parent) = target.parent() {
        container_mkdir_p(state, &container_id, parent).await?;
    }
    // Remove any existing mount path (file/dir/symlink).
    let _ = container_rm_rf(state, &container_id, target).await;

    // Prefer symlink; if unavailable, fall back to a recursive copy.
    let mut ln = podman_command(&state.core.data_root)?;
    ln.arg("exec")
        .arg("--interactive")
        .arg(&container_id)
        .arg("ln")
        .arg("-s")
        .arg("--")
        .arg(source)
        .arg(target);
    let out = ln.output().await.context("podman exec ln -s")?;
    if out.status.success() {
        return Ok(());
    }

    let mut cp = podman_command(&state.core.data_root)?;
    cp.arg("exec")
        .arg("--interactive")
        .arg(&container_id)
        .arg("cp")
        .arg("-a")
        .arg("--")
        .arg(source)
        .arg(target);
    let out = cp.output().await.context("podman exec cp -a")?;
    if out.status.success() {
        Ok(())
    } else {
        anyhow::bail!(
            "container mount failed (ln+cp) (status {}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
}

async fn container_ensure_git_exclude(
    state: &AppState,
    workspace: &Workspace,
    worktree_root: &Path,
) -> Result<()> {
    let container_id = ensure_workspace_container_for_attachments(state, workspace).await?;
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
    let mut cmd = podman_command(&state.core.data_root)?;
    cmd.arg("exec")
        .arg("--interactive")
        .arg("--workdir")
        .arg(worktree_root)
        .arg(&container_id)
        .arg("sh")
        .arg("-lc")
        .arg(script);
    let out = cmd.output().await.context("podman exec git exclude")?;
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

async fn container_remove_mount_path(
    state: &AppState,
    workspace_id: WorkspaceId,
    target: &Path,
) -> Result<()> {
    let container_id = workspace_container_name(workspace_id);
    // Best-effort: if the container doesn't exist, skip.
    let mut exists = podman_command(&state.core.data_root)?;
    exists.arg("container").arg("exists").arg(&container_id);
    let out = exists.output().await.context("podman container exists")?;
    if !out.status.success() {
        return Ok(());
    }
    let _ = container_rm_rf(state, &container_id, target).await;
    Ok(())
}

async fn container_remove_attachment_data_best_effort(
    state: &AppState,
    workspace_id: WorkspaceId,
    attachment: &WorkspaceAttachment,
) -> Result<()> {
    let container_id = workspace_container_name(workspace_id);
    let mut exists = podman_command(&state.core.data_root)?;
    exists.arg("container").arg("exists").arg(&container_id);
    let out = exists.output().await.context("podman container exists")?;
    if !out.status.success() {
        return Ok(());
    }
    let root = container_attachment_root(attachment);
    let _ = container_rm_rf(state, &container_id, &root).await;
    Ok(())
}

async fn ensure_attachment_mount(
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

    let container_mode = is_container_path(worktree_root);
    if container_mode {
        let should_refresh = refresh || attachment.update_policy != AttachmentUpdatePolicy::Manual;
        let imported = ensure_attachment_imported_to_container(
            state,
            workspace,
            attachment,
            &materialized.path,
            should_refresh,
        )
        .await?;
        let source_path = if let Some(subpath) = &attachment.subpath {
            imported.join(subpath)
        } else {
            imported
        };
        container_ensure_mount(state, workspace, &mount_abs, &source_path).await?;
    } else {
        if let Some(parent) = mount_abs.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let source_path = if let Some(subpath) = &attachment.subpath {
            materialized.path.join(subpath)
        } else {
            materialized.path.clone()
        };
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

async fn cleanup_removed_attachment(
    state: &AppState,
    attachment: &WorkspaceAttachment,
) -> Result<()> {
    let store = state.store_for_workspace(attachment.workspace_id).await?;
    let mounts = store
        .list_worktree_attachment_mounts_for_attachment(attachment.id)
        .await?;
    for mount in mounts {
        let path = PathBuf::from(&mount.mount_abs_path);
        if is_container_path(&path) {
            let _ = container_remove_mount_path(state, attachment.workspace_id, &path).await;
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

async fn materialize_attachment(
    state: &AppState,
    workspace: &Workspace,
    attachment: &WorkspaceAttachment,
    refresh: bool,
) -> Result<MaterializationResult> {
    match attachment.kind {
        WorkspaceAttachmentKind::ReferenceRepo => {
            materialize_reference_repo(state, attachment, refresh).await
        }
        WorkspaceAttachmentKind::DocMirror => {
            materialize_doc_mirror(state, workspace, attachment, refresh).await
        }
    }
}

async fn materialize_reference_repo(
    state: &AppState,
    attachment: &WorkspaceAttachment,
    refresh: bool,
) -> Result<MaterializationResult> {
    let revision = revision_key(attachment);
    let dest = materialized_path_for_attachment(state, attachment);
    let should_update = refresh || !dest.exists();
    if should_update {
        if dest.exists() {
            tokio::fs::remove_dir_all(&dest).await?;
        }
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        clone_reference_repo(&attachment.source, attachment.revision.as_deref(), &dest).await?;
    }
    Ok(MaterializationResult {
        path: dest,
        materialized_id: revision,
    })
}

async fn clone_reference_repo(source: &str, revision: Option<&str>, dest: &Path) -> Result<()> {
    let mut cmd = Command::new("git");
    cmd.arg("clone").arg("--depth").arg("1").arg("--no-tags");
    if let Some(rev) = revision {
        if !looks_like_sha(rev) {
            cmd.arg("--branch").arg(rev);
        }
    }
    cmd.arg(source).arg(dest);
    let output = cmd.output().await.context("running git clone")?;
    if !output.status.success() {
        anyhow::bail!(
            "git clone failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    if let Some(rev) = revision {
        if looks_like_sha(rev) {
            let fetch = Command::new("git")
                .arg("-C")
                .arg(dest)
                .arg("fetch")
                .arg("--depth")
                .arg("1")
                .arg("origin")
                .arg(rev)
                .output()
                .await
                .context("running git fetch")?;
            if !fetch.status.success() {
                anyhow::bail!(
                    "git fetch failed: {}",
                    String::from_utf8_lossy(&fetch.stderr)
                );
            }
            let checkout = Command::new("git")
                .arg("-C")
                .arg(dest)
                .arg("checkout")
                .arg(rev)
                .output()
                .await
                .context("running git checkout")?;
            if !checkout.status.success() {
                anyhow::bail!(
                    "git checkout failed: {}",
                    String::from_utf8_lossy(&checkout.stderr)
                );
            }
        }
    }
    Ok(())
}

async fn materialize_doc_mirror(
    state: &AppState,
    workspace: &Workspace,
    attachment: &WorkspaceAttachment,
    refresh: bool,
) -> Result<MaterializationResult> {
    let revision = revision_key(attachment);
    let dest = materialized_path_for_attachment(state, attachment);
    let should_update = refresh || !dest.exists();
    if should_update {
        if dest.exists() {
            tokio::fs::remove_dir_all(&dest).await?;
        }
        tokio::fs::create_dir_all(&dest).await?;
        run_doc_mirror_script(workspace, attachment, &dest).await?;
    }
    Ok(MaterializationResult {
        path: dest,
        materialized_id: revision,
    })
}

async fn run_doc_mirror_script(
    workspace: &Workspace,
    attachment: &WorkspaceAttachment,
    dest: &Path,
) -> Result<()> {
    if looks_like_url(&attachment.source) {
        return run_doc_mirror_cli(workspace, attachment, dest).await;
    }
    let script_path = resolve_workspace_path(&workspace.root_path, &attachment.source);
    if !script_path.exists() {
        anyhow::bail!("doc mirror script not found: {}", script_path.display());
    }

    let mut cmd = if script_path.extension().and_then(|s| s.to_str()) == Some("py") {
        let mut cmd = Command::new("python3");
        cmd.arg(&script_path);
        cmd
    } else if script_path.extension().and_then(|s| s.to_str()) == Some("sh") {
        let mut cmd = Command::new("bash");
        cmd.arg(&script_path);
        cmd
    } else {
        Command::new(&script_path)
    };

    cmd.arg(dest)
        .current_dir(&workspace.root_path)
        .env("CTX_DOCS_OUTPUT_DIR", dest)
        .env("CTX_DOCS_OUTPUT_DIR", dest);
    let output = cmd.output().await.context("running doc mirror script")?;
    if !output.status.success() {
        anyhow::bail!(
            "doc mirror script failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

fn docs_mirror_bin() -> PathBuf {
    std::env::var_os("CTX_DOCS_MIRROR_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("ctx-docs-mirror"))
}

fn looks_like_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}

async fn run_doc_mirror_cli(
    workspace: &Workspace,
    attachment: &WorkspaceAttachment,
    dest: &Path,
) -> Result<()> {
    let mut table = toml::value::Table::new();
    table.insert(
        "source".to_string(),
        TomlValue::String(attachment.source.clone()),
    );
    table.insert(
        "docs_url".to_string(),
        TomlValue::String(attachment.source.clone()),
    );
    let cfg = TomlValue::Table(table);
    let cfg_text = toml::to_string_pretty(&cfg).context("serializing docs mirror config")?;
    let mut temp = NamedTempFile::new().context("creating docs mirror config file")?;
    temp.write_all(cfg_text.as_bytes())
        .context("writing docs mirror config")?;
    temp.flush().context("flushing docs mirror config")?;

    let bin = docs_mirror_bin();
    let mut cmd = Command::new(&bin);
    cmd.arg("mirror")
        .arg("--config")
        .arg(temp.path())
        .arg("--out")
        .arg(dest)
        .current_dir(&workspace.root_path);
    let output = cmd.output().await.context("running ctx-docs-mirror")?;
    if !output.status.success() {
        anyhow::bail!(
            "ctx-docs-mirror failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

async fn ensure_git_exclude(
    state: &AppState,
    workspace: &Workspace,
    worktree_root: &Path,
) -> Result<()> {
    if is_container_path(worktree_root) {
        return container_ensure_git_exclude(state, workspace, worktree_root).await;
    }
    let git_dir = resolve_git_dir(worktree_root).await?;
    let git_info = git_dir.join("info");
    tokio::fs::create_dir_all(&git_info).await?;
    let path = git_info.join("exclude");
    let mut content = if path.exists() {
        tokio::fs::read_to_string(&path).await?
    } else {
        String::new()
    };

    let lines = [".ctx/attachments/refs/", ".ctx/attachments/docs/"];
    let mut changed = false;
    for line in lines {
        if !content.lines().any(|l| l.trim() == line) {
            if !content.ends_with('\n') && !content.is_empty() {
                content.push('\n');
            }
            content.push_str(line);
            content.push('\n');
            changed = true;
        }
    }
    if changed {
        tokio::fs::write(&path, content).await?;
    }
    Ok(())
}

async fn resolve_git_dir(worktree_root: &Path) -> Result<PathBuf> {
    let dotgit = worktree_root.join(".git");
    let meta = tokio::fs::metadata(&dotgit).await?;
    if meta.is_dir() {
        return Ok(dotgit);
    }
    let txt = tokio::fs::read_to_string(&dotgit).await?;
    let line = txt
        .lines()
        .find(|l| l.trim_start().starts_with("gitdir:"))
        .ok_or_else(|| anyhow::anyhow!("invalid .git file: missing gitdir"))?;
    let raw = line.trim_start().trim_start_matches("gitdir:").trim();
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(worktree_root.join(path))
    }
}

async fn remove_mount_path(target: &Path) -> Result<()> {
    if let Ok(meta) = tokio::fs::symlink_metadata(target).await {
        if meta.file_type().is_symlink() || meta.is_file() {
            tokio::fs::remove_file(target).await?;
        } else if meta.is_dir() {
            tokio::fs::remove_dir_all(target).await?;
        }
    }
    Ok(())
}

async fn ensure_mount(target: &Path, source: &Path) -> Result<()> {
    if let Ok(meta) = tokio::fs::symlink_metadata(target).await {
        if meta.file_type().is_symlink() {
            if let Ok(current) = tokio::fs::read_link(target).await {
                if current == source {
                    return Ok(());
                }
            }
            tokio::fs::remove_file(target).await?;
        } else if meta.is_dir() {
            tokio::fs::remove_dir_all(target).await?;
        } else {
            tokio::fs::remove_file(target).await?;
        }
    }

    if let Err(err) = try_symlink_dir(source, target).await {
        tracing::debug!("symlink failed ({}); falling back to copy", err);
        let source = source.to_path_buf();
        let target = target.to_path_buf();
        tokio::task::spawn_blocking(move || copy_dir_recursive(&source, &target)).await??;
    }
    Ok(())
}

async fn try_symlink_dir(source: &Path, target: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        tokio::task::spawn_blocking({
            let source = source.to_path_buf();
            let target = target.to_path_buf();
            move || symlink(source, target)
        })
        .await??;
        Ok(())
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::symlink_dir;
        tokio::task::spawn_blocking({
            let source = source.to_path_buf();
            let target = target.to_path_buf();
            move || symlink_dir(source, target)
        })
        .await??;
        Ok(())
    }
}

fn copy_dir_recursive(source: &Path, target: &Path) -> Result<()> {
    std::fs::create_dir_all(target)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let dest = target.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&entry.path(), &dest)?;
        } else if file_type.is_file() {
            std::fs::copy(entry.path(), dest)?;
        }
    }
    Ok(())
}

fn attachment_store_root(data_root: &Path) -> PathBuf {
    data_root.join("attachments")
}

fn materialized_root_for_attachment(state: &AppState, attachment: &WorkspaceAttachment) -> PathBuf {
    match attachment.kind {
        WorkspaceAttachmentKind::ReferenceRepo => attachment_store_root(&state.core.data_root)
            .join("reference-repos")
            .join("checkouts")
            .join(attachment.id.0.to_string()),
        WorkspaceAttachmentKind::DocMirror => attachment_store_root(&state.core.data_root)
            .join("doc-mirrors")
            .join(attachment.id.0.to_string()),
    }
}

fn materialized_path_for_attachment(state: &AppState, attachment: &WorkspaceAttachment) -> PathBuf {
    let revision = revision_key(attachment);
    match attachment.kind {
        WorkspaceAttachmentKind::ReferenceRepo => attachment_store_root(&state.core.data_root)
            .join("reference-repos")
            .join("checkouts")
            .join(attachment.id.0.to_string())
            .join(revision),
        WorkspaceAttachmentKind::DocMirror => attachment_store_root(&state.core.data_root)
            .join("doc-mirrors")
            .join(attachment.id.0.to_string())
            .join(revision),
    }
}

fn resolve_workspace_path(workspace_root: &str, raw: &str) -> PathBuf {
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        path
    } else {
        Path::new(workspace_root).join(path)
    }
}

fn sanitize_mount_relpath(value: &str) -> Result<PathBuf> {
    if value.trim().is_empty() {
        anyhow::bail!("mount_relpath must not be empty");
    }
    let path = PathBuf::from(value);
    if path.is_absolute() {
        anyhow::bail!("mount_relpath must be relative: {}", value);
    }
    for part in path.components() {
        if matches!(part, std::path::Component::ParentDir) {
            anyhow::bail!("mount_relpath must not contain '..': {}", value);
        }
    }
    Ok(path)
}

fn default_mount_relpath(kind: &WorkspaceAttachmentKind, name: &str) -> String {
    let safe_name = sanitize_name(name);
    match kind {
        WorkspaceAttachmentKind::ReferenceRepo => format!(".ctx/attachments/refs/{safe_name}"),
        WorkspaceAttachmentKind::DocMirror => format!(".ctx/attachments/docs/{safe_name}"),
    }
}

fn sanitize_name(name: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "attachment".to_string()
    } else {
        trimmed.to_string()
    }
}

fn revision_key(attachment: &WorkspaceAttachment) -> String {
    let base = attachment.revision.as_deref().unwrap_or("default");
    sanitize_name(base)
}

fn looks_like_sha(value: &str) -> bool {
    let len = value.len();
    if !(7..=40).contains(&len) {
        return false;
    }
    value.chars().all(|c| c.is_ascii_hexdigit())
}
