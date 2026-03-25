use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;
use tokio::process::Command;
use toml::Value as TomlValue;

mod container_mounts;

use self::container_mounts::{
    cleanup_removed_attachment, container_ensure_git_exclude, ensure_attachment_mount,
};

use ctx_core::ids::{WorkspaceAttachmentId, WorkspaceId, WorktreeId};
use ctx_core::models::{
    AttachmentMode, AttachmentUpdatePolicy, Workspace, WorkspaceAttachment,
    WorkspaceAttachmentKind, WorkspaceAttachmentStatus, Worktree, WorktreeAttachmentMount,
    WorktreeAttachmentStatus,
};

use crate::container_fs::is_container_path;
use crate::daemon::{AppState, AttachmentMaterializationTask};
use crate::execution_effective;
use crate::harness_runtime::{
    podman_command, workspace_container_name, CTX_CONTAINER_WORKSPACE_ROOT,
};
use crate::worktree_data_plane::live_worktree_root_for_mode;

const CONTAINER_ATTACHMENTS_SUBDIR: &str = "attachments";

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
    let store = state.store_for_workspace(workspace.id).await?;
    let existing = store.list_workspace_attachments(workspace.id).await?;

    let mut out = Vec::with_capacity(existing.len());
    let mut sync_plans = Vec::new();
    for mut attachment in existing {
        let now = Utc::now();
        let should_refresh = refresh || attachment.update_policy != AttachmentUpdatePolicy::Manual;
        let materialized_exists =
            materialized_path_for_attachment(state.as_ref(), &attachment).exists();
        let should_materialize = should_refresh || !materialized_exists;
        if should_materialize && attachment.status != WorkspaceAttachmentStatus::Syncing {
            attachment.status = WorkspaceAttachmentStatus::Pending;
            attachment.error_message = None;
            attachment.updated_at = now;
            sync_plans.push(AttachmentSyncPlan {
                id: attachment.id,
                refresh: should_refresh,
            });
        } else if !should_materialize && attachment.status != WorkspaceAttachmentStatus::Ready {
            // Heal stale pending/error states when the materialized content already exists
            // and no refresh is required (e.g. manual-policy attachments after daemon restarts).
            attachment.status = WorkspaceAttachmentStatus::Ready;
            attachment.error_message = None;
            if attachment.last_sync_at.is_none() {
                attachment.last_sync_at = Some(now);
            }
            attachment.updated_at = now;
        }
        store.upsert_workspace_attachment(&attachment).await?;
        out.push(attachment);
    }

    for plan in sync_plans {
        spawn_attachment_materialization(
            Arc::clone(&state),
            workspace.clone(),
            plan.id,
            plan.refresh,
        )
        .await;
    }

    Ok(out)
}

pub async fn upsert_workspace_attachment(
    state: &AppState,
    workspace_id: WorkspaceId,
    cfg: AttachmentConfig,
) -> Result<WorkspaceAttachment> {
    let store = state.store_for_workspace(workspace_id).await?;
    let existing = store.list_workspace_attachments(workspace_id).await?;
    let existing = existing.into_iter().find(|attachment| {
        attachment.kind == cfg.kind && attachment.name.trim() == cfg.name.trim()
    });
    let attachment = normalize_attachment_config(workspace_id, cfg, existing);
    store.upsert_workspace_attachment(&attachment).await?;
    Ok(attachment)
}

pub async fn delete_workspace_attachment(
    state: &AppState,
    workspace_id: WorkspaceId,
    kind: WorkspaceAttachmentKind,
    name: &str,
) -> Result<bool> {
    let store = state.store_for_workspace(workspace_id).await?;
    let existing = store.list_workspace_attachments(workspace_id).await?;
    let Some(target) = existing
        .into_iter()
        .find(|attachment| attachment.kind == kind && attachment.name.trim() == name.trim())
    else {
        return Ok(false);
    };
    cancel_attachment_materialization(state, target.id).await;
    cleanup_removed_attachment(state, &target).await?;
    store.delete_workspace_attachment(target.id).await?;
    Ok(true)
}

async fn spawn_attachment_materialization(
    state: Arc<AppState>,
    workspace: Workspace,
    attachment_id: WorkspaceAttachmentId,
    refresh: bool,
) {
    cancel_attachment_materialization(state.as_ref(), attachment_id).await;
    let generation = state
        .workspaces
        .attachment_materialization_generation
        .fetch_add(1, Ordering::SeqCst)
        + 1;
    let task_state = Arc::clone(&state);
    let handle = tokio::spawn(async move {
        run_attachment_materialization(Arc::clone(&task_state), workspace, attachment_id, refresh)
            .await;
        clear_attachment_materialization_task(task_state.as_ref(), attachment_id, generation).await;
    });
    let mut tasks = state.workspaces.attachment_materializations.lock().await;
    tasks.insert(
        attachment_id,
        AttachmentMaterializationTask { generation, handle },
    );
}

async fn cancel_attachment_materialization(state: &AppState, attachment_id: WorkspaceAttachmentId) {
    let existing = {
        let mut tasks = state.workspaces.attachment_materializations.lock().await;
        tasks.remove(&attachment_id)
    };
    if let Some(task) = existing {
        task.handle.abort();
        let _ = task.handle.await;
    }
}

async fn clear_attachment_materialization_task(
    state: &AppState,
    attachment_id: WorkspaceAttachmentId,
    generation: u64,
) {
    let mut tasks = state.workspaces.attachment_materializations.lock().await;
    if tasks
        .get(&attachment_id)
        .is_some_and(|task| task.generation == generation)
    {
        tasks.remove(&attachment_id);
    }
}

async fn run_attachment_materialization(
    state: Arc<AppState>,
    workspace: Workspace,
    attachment_id: WorkspaceAttachmentId,
    refresh: bool,
) {
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

    let effective = execution_effective::effective_execution_settings(state, workspace.id).await?;
    let worktree_root =
        live_worktree_root_for_mode(&state.core.data_root, workspace, worktree, effective.mode);
    ensure_git_exclude(state, workspace, worktree.id, &worktree_root).await?;

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

fn normalize_attachment_config(
    workspace_id: WorkspaceId,
    cfg: AttachmentConfig,
    existing: Option<WorkspaceAttachment>,
) -> WorkspaceAttachment {
    let name = cfg.name.trim().to_string();
    let now = Utc::now();
    let (id, created_at, status, last_sync_at, error_message) = match existing {
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
    cmd.arg("clone")
        .arg("--depth")
        .arg("1")
        .arg("--no-tags")
        .kill_on_drop(true);
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
            let mut fetch_cmd = Command::new("git");
            fetch_cmd
                .arg("-C")
                .arg(dest)
                .arg("fetch")
                .arg("--depth")
                .arg("1")
                .arg("origin")
                .arg(rev)
                .kill_on_drop(true);
            let fetch = fetch_cmd.output().await.context("running git fetch")?;
            if !fetch.status.success() {
                anyhow::bail!(
                    "git fetch failed: {}",
                    String::from_utf8_lossy(&fetch.stderr)
                );
            }
            let mut checkout_cmd = Command::new("git");
            checkout_cmd
                .arg("-C")
                .arg(dest)
                .arg("checkout")
                .arg(rev)
                .kill_on_drop(true);
            let checkout = checkout_cmd
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
        .env("CTX_DOCS_OUTPUT_DIR", dest)
        .kill_on_drop(true);
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
        .current_dir(&workspace.root_path)
        .kill_on_drop(true);
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
    worktree_id: WorktreeId,
    worktree_root: &Path,
) -> Result<()> {
    if is_container_path(worktree_root) {
        return container_ensure_git_exclude(state, workspace, worktree_id, worktree_root).await;
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
