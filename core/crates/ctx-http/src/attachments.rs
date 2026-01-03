use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::process::Command;

use ctx_core::ids::{TrackId, WorkspaceAttachmentId, WorkspaceId};
use ctx_core::models::{
    AttachmentMode, AttachmentUpdatePolicy, Track, TrackAttachmentMount, TrackAttachmentStatus,
    Workspace, WorkspaceAttachment, WorkspaceAttachmentKind, Worktree,
};

use crate::daemon::AppState;

const ATTACHMENTS_CONFIG_PATH: &str = ".ctx/attachments.toml";

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

pub async fn sync_workspace_attachments(
    state: &AppState,
    workspace: &Workspace,
    refresh: bool,
) -> Result<Vec<WorkspaceAttachment>> {
    let cfg = load_attachments_config(Path::new(&workspace.root_path)).await?;
    let existing = state.store.list_workspace_attachments(workspace.id).await?;
    if cfg.is_none() {
        for attachment in existing {
            cleanup_removed_attachment(state, &attachment).await?;
            state
                .store
                .delete_workspace_attachment(attachment.id)
                .await?;
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
    for entry in cfg.attachments {
        let attachment =
            normalize_attachment_config(workspace.id, entry, |key| existing_map.remove(key));
        keep_ids.insert(attachment.id);
        state.store.upsert_workspace_attachment(&attachment).await?;
        maybe_materialize_attachment(state, workspace, &attachment, refresh).await?;
        out.push(attachment);
    }

    let mut removed = Vec::new();
    for (_, attachment) in existing_map {
        if !keep_ids.contains(&attachment.id) {
            removed.push(attachment);
        }
    }

    for attachment in removed {
        cleanup_removed_attachment(state, &attachment).await?;
        state
            .store
            .delete_workspace_attachment(attachment.id)
            .await?;
    }

    Ok(out)
}

pub async fn ensure_track_attachment_mounts(
    state: &AppState,
    workspace: &Workspace,
    track: &Track,
    worktree: &Worktree,
    refresh: bool,
) -> Result<Vec<TrackAttachmentMount>> {
    let attachments = state.store.list_workspace_attachments(workspace.id).await?;
    ensure_track_attachment_mounts_for_attachments(
        state,
        workspace,
        track,
        worktree,
        &attachments,
        refresh,
        true,
    )
    .await
}

pub async fn ensure_track_attachment_mounts_for_attachments(
    state: &AppState,
    workspace: &Workspace,
    track: &Track,
    worktree: &Worktree,
    attachments: &[WorkspaceAttachment],
    refresh: bool,
    materialize: bool,
) -> Result<Vec<TrackAttachmentMount>> {
    if attachments.is_empty() {
        return Ok(vec![]);
    }

    let worktree_root = PathBuf::from(&worktree.root_path);
    ensure_git_exclude(&worktree_root).await?;

    let mut mounts = Vec::with_capacity(attachments.len());
    for attachment in attachments {
        match ensure_attachment_mount(
            state,
            workspace,
            track.id,
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
                let mount = TrackAttachmentMount {
                    track_id: track.id,
                    attachment_id: attachment.id,
                    mount_abs_path: worktree_root
                        .join(&attachment.mount_relpath)
                        .to_string_lossy()
                        .to_string(),
                    materialized_id: revision_key(attachment),
                    status: TrackAttachmentStatus::Error,
                    last_sync_at: Some(now),
                    error_message: Some(e.to_string()),
                    created_at: now,
                    updated_at: now,
                };
                state.store.upsert_track_attachment_mount(&mount).await?;
                mounts.push(mount);
            }
        }
    }

    Ok(mounts)
}

pub async fn ensure_workspace_attachments_for_tracks(
    state: &AppState,
    workspace: &Workspace,
    refresh: bool,
) -> Result<()> {
    let attachments = state.store.list_workspace_attachments(workspace.id).await?;
    ensure_workspace_attachments_for_tracks_with_attachments(
        state,
        workspace,
        &attachments,
        refresh,
        true,
    )
    .await
}

pub async fn ensure_workspace_attachments_for_tracks_with_attachments(
    state: &AppState,
    workspace: &Workspace,
    attachments: &[WorkspaceAttachment],
    refresh: bool,
    materialize: bool,
) -> Result<()> {
    let tracks = state.store.list_tracks_for_workspace(workspace.id).await?;
    for track in tracks {
        let Some(worktree) = state.store.get_worktree(track.worktree_id).await? else {
            continue;
        };
        let _ = ensure_track_attachment_mounts_for_attachments(
            state,
            workspace,
            &track,
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
    let (id, created_at) = match take_existing(&key) {
        Some(existing) => (existing.id, existing.created_at),
        None => (WorkspaceAttachmentId::new(), now),
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
        created_at,
        updated_at: now,
    }
}

async fn maybe_materialize_attachment(
    state: &AppState,
    workspace: &Workspace,
    attachment: &WorkspaceAttachment,
    refresh: bool,
) -> Result<()> {
    let should_refresh = refresh || attachment.update_policy != AttachmentUpdatePolicy::Manual;
    let _ = materialize_attachment(state, workspace, attachment, should_refresh).await?;
    Ok(())
}

async fn ensure_attachment_mount(
    state: &AppState,
    workspace: &Workspace,
    track_id: TrackId,
    worktree_root: &Path,
    attachment: &WorkspaceAttachment,
    refresh: bool,
    materialize: bool,
) -> Result<TrackAttachmentMount> {
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

    if let Some(parent) = mount_abs.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let source_path = if let Some(subpath) = &attachment.subpath {
        materialized.path.join(subpath)
    } else {
        materialized.path.clone()
    };

    ensure_mount(&mount_abs, &source_path).await?;

    let now = Utc::now();
    let mount = TrackAttachmentMount {
        track_id,
        attachment_id: attachment.id,
        mount_abs_path: mount_abs.to_string_lossy().to_string(),
        materialized_id: materialized.materialized_id,
        status: TrackAttachmentStatus::Ready,
        last_sync_at: Some(now),
        error_message: None,
        created_at: now,
        updated_at: now,
    };
    state.store.upsert_track_attachment_mount(&mount).await?;
    Ok(mount)
}

async fn cleanup_removed_attachment(
    state: &AppState,
    attachment: &WorkspaceAttachment,
) -> Result<()> {
    let mounts = state
        .store
        .list_track_attachment_mounts_for_attachment(attachment.id)
        .await?;
    for mount in mounts {
        let path = PathBuf::from(&mount.mount_abs_path);
        remove_mount_path(&path).await?;
    }
    state
        .store
        .delete_track_attachment_mounts_for_attachment(attachment.id)
        .await?;
    let root = materialized_root_for_attachment(state, attachment);
    if root.exists() {
        tokio::fs::remove_dir_all(root).await?;
    }
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

async fn ensure_git_exclude(worktree_root: &Path) -> Result<()> {
    let git_dir = resolve_git_dir(worktree_root).await?;
    let git_info = git_dir.join("info");
    tokio::fs::create_dir_all(&git_info).await?;
    let path = git_info.join("exclude");
    let mut content = if path.exists() {
        tokio::fs::read_to_string(&path).await?
    } else {
        String::new()
    };

    let lines = [".ctx/.refs/", ".ctx/.docs/"];
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
        WorkspaceAttachmentKind::ReferenceRepo => attachment_store_root(&state.data_root)
            .join("reference-repos")
            .join("checkouts")
            .join(attachment.id.0.to_string()),
        WorkspaceAttachmentKind::DocMirror => attachment_store_root(&state.data_root)
            .join("doc-mirrors")
            .join(attachment.id.0.to_string()),
    }
}

fn materialized_path_for_attachment(state: &AppState, attachment: &WorkspaceAttachment) -> PathBuf {
    let revision = revision_key(attachment);
    match attachment.kind {
        WorkspaceAttachmentKind::ReferenceRepo => attachment_store_root(&state.data_root)
            .join("reference-repos")
            .join("checkouts")
            .join(attachment.id.0.to_string())
            .join(revision),
        WorkspaceAttachmentKind::DocMirror => attachment_store_root(&state.data_root)
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
        WorkspaceAttachmentKind::ReferenceRepo => format!(".ctx/.refs/{safe_name}"),
        WorkspaceAttachmentKind::DocMirror => format!(".ctx/.docs/{safe_name}"),
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
