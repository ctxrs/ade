use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ctx_core::ids::{WorkspaceAttachmentId, WorkspaceId};
use ctx_core::models::{
    AttachmentMode, AttachmentUpdatePolicy, Workspace, WorkspaceAttachment,
    WorkspaceAttachmentKind, WorkspaceAttachmentStatus,
};
use serde::{Deserialize, Serialize};
use tokio::process::Command;

mod doc_mirror;
use doc_mirror::{
    materialize_doc_mirror, validate_doc_mirror_source, validate_doc_mirror_source_value,
};

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
pub struct MaterializationResult {
    pub path: PathBuf,
    pub materialized_id: String,
}

#[derive(Debug, Clone, Copy)]
pub struct AttachmentSyncPlan {
    pub id: WorkspaceAttachmentId,
    pub refresh: bool,
}

#[derive(Debug, Clone)]
pub struct WorkspaceAttachmentSyncResult {
    pub attachments: Vec<WorkspaceAttachment>,
    pub plans: Vec<AttachmentSyncPlan>,
}

#[async_trait]
pub trait WorkspaceAttachmentsHost: Send + Sync + 'static {
    fn data_root(&self) -> &Path;

    async fn list_workspace_attachments(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<WorkspaceAttachment>>;

    async fn get_workspace_attachment(
        &self,
        workspace_id: WorkspaceId,
        attachment_id: WorkspaceAttachmentId,
    ) -> Result<Option<WorkspaceAttachment>>;

    async fn upsert_workspace_attachment(&self, attachment: &WorkspaceAttachment) -> Result<()>;

    async fn update_workspace_attachment_status(
        &self,
        workspace_id: WorkspaceId,
        attachment_id: WorkspaceAttachmentId,
        status: WorkspaceAttachmentStatus,
        last_sync_at: Option<DateTime<Utc>>,
        error_message: Option<String>,
        updated_at: DateTime<Utc>,
    ) -> Result<()>;

    async fn delete_workspace_attachment_record(
        &self,
        workspace_id: WorkspaceId,
        attachment_id: WorkspaceAttachmentId,
    ) -> Result<()>;

    async fn attachment_became_ready(
        &self,
        workspace: &Workspace,
        attachment: &WorkspaceAttachment,
    ) -> Result<()>;

    async fn cleanup_removed_attachment(&self, attachment: &WorkspaceAttachment) -> Result<()>;
}

pub async fn sync_workspace_attachments<H>(
    host: &H,
    workspace: &Workspace,
    refresh: bool,
) -> Result<WorkspaceAttachmentSyncResult>
where
    H: WorkspaceAttachmentsHost,
{
    let existing = host.list_workspace_attachments(workspace.id).await?;

    let mut attachments = Vec::with_capacity(existing.len());
    let mut plans = Vec::new();
    for mut attachment in existing {
        let now = Utc::now();
        let should_refresh = refresh || attachment.update_policy != AttachmentUpdatePolicy::Manual;
        let materialized_exists =
            materialized_path_for_attachment(host.data_root(), &attachment).exists();
        let should_materialize = should_refresh || !materialized_exists;
        if should_materialize && attachment.status != WorkspaceAttachmentStatus::Syncing {
            match validate_attachment_source_before_materialization(workspace, &attachment) {
                Ok(()) => {
                    attachment.status = WorkspaceAttachmentStatus::Pending;
                    attachment.error_message = None;
                    attachment.updated_at = now;
                    plans.push(AttachmentSyncPlan {
                        id: attachment.id,
                        refresh: should_refresh,
                    });
                }
                Err(err) => {
                    attachment.status = WorkspaceAttachmentStatus::Error;
                    attachment.error_message = Some(err.to_string());
                    attachment.updated_at = now;
                }
            }
        } else if !should_materialize && attachment.status != WorkspaceAttachmentStatus::Ready {
            attachment.status = WorkspaceAttachmentStatus::Ready;
            attachment.error_message = None;
            if attachment.last_sync_at.is_none() {
                attachment.last_sync_at = Some(now);
            }
            attachment.updated_at = now;
        }
        host.upsert_workspace_attachment(&attachment).await?;
        attachments.push(attachment);
    }

    Ok(WorkspaceAttachmentSyncResult { attachments, plans })
}

fn validate_attachment_source_before_materialization(
    workspace: &Workspace,
    attachment: &WorkspaceAttachment,
) -> Result<()> {
    match attachment.kind {
        WorkspaceAttachmentKind::DocMirror => validate_doc_mirror_source(workspace, attachment),
        WorkspaceAttachmentKind::ReferenceRepo => Ok(()),
    }
}

pub async fn upsert_workspace_attachment<H>(
    host: &H,
    workspace_id: WorkspaceId,
    cfg: AttachmentConfig,
) -> Result<WorkspaceAttachment>
where
    H: WorkspaceAttachmentsHost,
{
    let existing =
        find_workspace_attachment(host, workspace_id, cfg.kind.clone(), &cfg.name).await?;
    let attachment = normalize_attachment_config(workspace_id, cfg, existing)?;
    host.upsert_workspace_attachment(&attachment).await?;
    Ok(attachment)
}

pub async fn find_workspace_attachment<H>(
    host: &H,
    workspace_id: WorkspaceId,
    kind: WorkspaceAttachmentKind,
    name: &str,
) -> Result<Option<WorkspaceAttachment>>
where
    H: WorkspaceAttachmentsHost,
{
    let existing = host.list_workspace_attachments(workspace_id).await?;
    Ok(existing
        .into_iter()
        .find(|attachment| attachment.kind == kind && attachment.name.trim() == name.trim()))
}

pub async fn delete_workspace_attachment<H>(
    host: &H,
    attachment: &WorkspaceAttachment,
) -> Result<()>
where
    H: WorkspaceAttachmentsHost,
{
    host.cleanup_removed_attachment(attachment).await?;
    host.delete_workspace_attachment_record(attachment.workspace_id, attachment.id)
        .await
}

pub async fn run_attachment_materialization<H>(
    host: &H,
    workspace: &Workspace,
    attachment_id: WorkspaceAttachmentId,
    refresh: bool,
) -> Result<()>
where
    H: WorkspaceAttachmentsHost,
{
    let Some(attachment) = host
        .get_workspace_attachment(workspace.id, attachment_id)
        .await?
    else {
        return Ok(());
    };

    let now = Utc::now();
    host.update_workspace_attachment_status(
        workspace.id,
        attachment_id,
        WorkspaceAttachmentStatus::Syncing,
        None,
        None,
        now,
    )
    .await?;

    match materialize_attachment(host.data_root(), workspace, &attachment, refresh).await {
        Ok(_) => {
            let now = Utc::now();
            host.update_workspace_attachment_status(
                workspace.id,
                attachment_id,
                WorkspaceAttachmentStatus::Ready,
                Some(now),
                None,
                now,
            )
            .await?;
            let _ = host.attachment_became_ready(workspace, &attachment).await;
            Ok(())
        }
        Err(err) => {
            let now = Utc::now();
            host.update_workspace_attachment_status(
                workspace.id,
                attachment_id,
                WorkspaceAttachmentStatus::Error,
                None,
                Some(err.to_string()),
                now,
            )
            .await?;
            Err(err)
        }
    }
}

pub async fn materialize_attachment(
    data_root: &Path,
    workspace: &Workspace,
    attachment: &WorkspaceAttachment,
    refresh: bool,
) -> Result<MaterializationResult> {
    match attachment.kind {
        WorkspaceAttachmentKind::ReferenceRepo => {
            materialize_reference_repo(data_root, attachment, refresh).await
        }
        WorkspaceAttachmentKind::DocMirror => {
            materialize_doc_mirror(data_root, workspace, attachment, refresh).await
        }
    }
}

pub fn materialized_root_for_attachment(
    data_root: &Path,
    attachment: &WorkspaceAttachment,
) -> PathBuf {
    match attachment.kind {
        WorkspaceAttachmentKind::ReferenceRepo => attachment_store_root(data_root)
            .join("reference-repos")
            .join("checkouts")
            .join(attachment.id.0.to_string()),
        WorkspaceAttachmentKind::DocMirror => attachment_store_root(data_root)
            .join("doc-mirrors")
            .join(attachment.id.0.to_string()),
    }
}

pub fn materialized_path_for_attachment(
    data_root: &Path,
    attachment: &WorkspaceAttachment,
) -> PathBuf {
    let revision = revision_key(attachment);
    materialized_root_for_attachment(data_root, attachment).join(revision)
}

pub fn sanitize_mount_relpath(value: &str) -> Result<PathBuf> {
    let value = value.trim();
    if value.is_empty() {
        anyhow::bail!("mount_relpath must not be empty");
    }
    if value.contains('\\') {
        anyhow::bail!("mount_relpath must use '/' separators: {value}");
    }
    for segment in value.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            anyhow::bail!("mount_relpath contains unsupported component: {value}");
        }
    }
    let path = PathBuf::from(value);
    if path.is_absolute() {
        anyhow::bail!("mount_relpath must be relative: {value}");
    }
    for part in path.components() {
        if !matches!(part, Component::Normal(_)) {
            anyhow::bail!("mount_relpath contains unsupported component: {value}");
        }
    }
    Ok(path)
}

pub fn sanitize_attachment_subpath(value: &str) -> Result<PathBuf> {
    if value.trim().is_empty() {
        anyhow::bail!("subpath must not be empty");
    }
    let path = PathBuf::from(value);
    if path.is_absolute() {
        anyhow::bail!("subpath must be relative: {value}");
    }
    for part in path.components() {
        if matches!(
            part,
            std::path::Component::ParentDir | std::path::Component::Prefix(_)
        ) {
            anyhow::bail!("subpath must not escape the attachment root: {value}");
        }
    }
    Ok(path)
}

pub fn revision_key(attachment: &WorkspaceAttachment) -> String {
    let base = attachment.revision.as_deref().unwrap_or("default");
    sanitize_name(base)
}

fn normalize_attachment_config(
    workspace_id: WorkspaceId,
    cfg: AttachmentConfig,
    existing: Option<WorkspaceAttachment>,
) -> Result<WorkspaceAttachment> {
    let name = cfg.name.trim().to_string();
    let source = cfg.source.trim().to_string();
    if source.is_empty() {
        anyhow::bail!("source must not be empty");
    }
    match &cfg.kind {
        WorkspaceAttachmentKind::ReferenceRepo => validate_reference_repo_source(&source)?,
        WorkspaceAttachmentKind::DocMirror => {
            validate_doc_mirror_source_value(&source)?;
            if cfg.mode == Some(AttachmentMode::Rw) {
                anyhow::bail!("doc_mirror attachments are read-only; mode=rw is not supported");
            }
        }
    }
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
    let mount_relpath = sanitize_mount_relpath(&mount_relpath)?
        .to_string_lossy()
        .to_string();
    let subpath = cfg
        .subpath
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if let Some(value) = subpath.as_deref() {
        sanitize_attachment_subpath(value)?;
    }

    Ok(WorkspaceAttachment {
        id,
        workspace_id,
        kind: cfg.kind,
        name,
        source,
        revision: cfg
            .revision
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        subpath,
        mount_relpath,
        mode: cfg.mode.unwrap_or(AttachmentMode::Ro),
        update_policy: cfg.update_policy.unwrap_or(AttachmentUpdatePolicy::Manual),
        status,
        last_sync_at,
        error_message,
        created_at,
        updated_at: now,
    })
}

async fn materialize_reference_repo(
    data_root: &Path,
    attachment: &WorkspaceAttachment,
    refresh: bool,
) -> Result<MaterializationResult> {
    let revision = revision_key(attachment);
    let dest = materialized_path_for_attachment(data_root, attachment);
    let should_update = refresh || !dest.exists();
    if should_update {
        remove_materialized_path_if_exists(data_root, &dest).await?;
        ensure_materialized_parent(data_root, &dest).await?;
        clone_reference_repo(&attachment.source, attachment.revision.as_deref(), &dest).await?;
    } else {
        validate_materialized_path(data_root, attachment).await?;
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

fn attachment_store_root(data_root: &Path) -> PathBuf {
    data_root.join("attachments")
}

pub async fn remove_materialized_root_if_exists(
    data_root: &Path,
    attachment: &WorkspaceAttachment,
) -> Result<()> {
    let root = materialized_root_for_attachment(data_root, attachment);
    remove_materialized_path_if_exists(data_root, &root).await
}

pub async fn validate_materialized_path(
    data_root: &Path,
    attachment: &WorkspaceAttachment,
) -> Result<()> {
    let path = materialized_path_for_attachment(data_root, attachment);
    let data_root = data_root.to_path_buf();
    tokio::task::spawn_blocking(move || validate_materialized_path_sync(&data_root, &path))
        .await
        .context("joining attachment materialization validation task")?
}

pub(crate) async fn ensure_materialized_revision_parent(
    data_root: &Path,
    attachment: &WorkspaceAttachment,
) -> Result<()> {
    let dest = materialized_path_for_attachment(data_root, attachment);
    ensure_materialized_parent(data_root, &dest).await
}

pub(crate) async fn remove_materialized_revision_if_exists(
    data_root: &Path,
    attachment: &WorkspaceAttachment,
) -> Result<()> {
    let dest = materialized_path_for_attachment(data_root, attachment);
    remove_materialized_path_if_exists(data_root, &dest).await
}

async fn remove_materialized_path_if_exists(data_root: &Path, path: &Path) -> Result<()> {
    let data_root = data_root.to_path_buf();
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || remove_materialized_path_if_exists_sync(&data_root, &path))
        .await
        .context("joining attachment materialization cleanup task")?
}

async fn ensure_materialized_parent(data_root: &Path, path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("attachment materialization path missing parent"))?
        .to_path_buf();
    let data_root = data_root.to_path_buf();
    tokio::task::spawn_blocking(move || ensure_materialized_dir_chain_sync(&data_root, &parent))
        .await
        .context("joining attachment materialization parent task")?
}

fn remove_materialized_path_if_exists_sync(data_root: &Path, path: &Path) -> Result<()> {
    validate_materialized_child_path(data_root, path)?;
    if let Some(parent) = path.parent() {
        ensure_materialized_existing_chain_sync(data_root, parent)?;
    }
    match std::fs::symlink_metadata(path) {
        Ok(meta) => {
            if meta.file_type().is_symlink() {
                anyhow::bail!(
                    "attachment materialization path must not be a symlink: {}",
                    path.display()
                );
            }
            if !meta.is_dir() {
                anyhow::bail!(
                    "attachment materialization path must be a directory: {}",
                    path.display()
                );
            }
            std::fs::remove_dir_all(path)
                .with_context(|| format!("removing attachment materialization {}", path.display()))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err)
            .with_context(|| format!("reading attachment materialization {}", path.display())),
    }
}

fn validate_materialized_path_sync(data_root: &Path, path: &Path) -> Result<()> {
    validate_materialized_child_path(data_root, path)?;
    if let Some(parent) = path.parent() {
        ensure_materialized_existing_chain_sync(data_root, parent)?;
    }
    let meta = std::fs::symlink_metadata(path)
        .with_context(|| format!("reading attachment materialization {}", path.display()))?;
    if meta.file_type().is_symlink() {
        anyhow::bail!(
            "attachment materialization path must not be a symlink: {}",
            path.display()
        );
    }
    if !meta.is_dir() {
        anyhow::bail!(
            "attachment materialization path must be a directory: {}",
            path.display()
        );
    }
    Ok(())
}

fn ensure_materialized_dir_chain_sync(data_root: &Path, path: &Path) -> Result<()> {
    validate_materialized_child_path(data_root, path)?;
    let rel = path.strip_prefix(data_root).with_context(|| {
        format!(
            "attachment materialization path {} is outside data root {}",
            path.display(),
            data_root.display()
        )
    })?;
    let mut current = data_root.to_path_buf();
    for component in rel.components() {
        let Component::Normal(segment) = component else {
            anyhow::bail!(
                "attachment materialization path contains unsupported component: {}",
                path.display()
            );
        };
        current.push(segment);
        match std::fs::symlink_metadata(&current) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    anyhow::bail!(
                        "attachment materialization parent must not be a symlink: {}",
                        current.display()
                    );
                }
                if !meta.is_dir() {
                    anyhow::bail!(
                        "attachment materialization parent must be a directory: {}",
                        current.display()
                    );
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&current).with_context(|| {
                    format!(
                        "creating attachment materialization parent {}",
                        current.display()
                    )
                })?;
                let meta = std::fs::symlink_metadata(&current).with_context(|| {
                    format!(
                        "verifying attachment materialization parent {}",
                        current.display()
                    )
                })?;
                if meta.file_type().is_symlink() || !meta.is_dir() {
                    anyhow::bail!(
                        "attachment materialization parent was not created as a directory: {}",
                        current.display()
                    );
                }
            }
            Err(err) => {
                return Err(err).with_context(|| {
                    format!(
                        "reading attachment materialization parent {}",
                        current.display()
                    )
                });
            }
        }
    }
    Ok(())
}

fn ensure_materialized_existing_chain_sync(data_root: &Path, path: &Path) -> Result<()> {
    validate_materialized_child_path(data_root, path)?;
    let rel = path.strip_prefix(data_root).with_context(|| {
        format!(
            "attachment materialization path {} is outside data root {}",
            path.display(),
            data_root.display()
        )
    })?;
    let mut current = data_root.to_path_buf();
    for component in rel.components() {
        let Component::Normal(segment) = component else {
            anyhow::bail!(
                "attachment materialization path contains unsupported component: {}",
                path.display()
            );
        };
        current.push(segment);
        match std::fs::symlink_metadata(&current) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    anyhow::bail!(
                        "attachment materialization parent must not be a symlink: {}",
                        current.display()
                    );
                }
                if !meta.is_dir() {
                    anyhow::bail!(
                        "attachment materialization parent must be a directory: {}",
                        current.display()
                    );
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(err) => {
                return Err(err).with_context(|| {
                    format!(
                        "reading attachment materialization parent {}",
                        current.display()
                    )
                });
            }
        }
    }
    Ok(())
}

fn validate_materialized_child_path(data_root: &Path, path: &Path) -> Result<()> {
    let store_root = attachment_store_root(data_root);
    if !path.starts_with(&store_root) {
        anyhow::bail!(
            "attachment materialization path {} is outside attachment store {}",
            path.display(),
            store_root.display()
        );
    }
    let rel = path.strip_prefix(data_root).with_context(|| {
        format!(
            "attachment materialization path {} is outside data root {}",
            path.display(),
            data_root.display()
        )
    })?;
    for component in rel.components() {
        if !matches!(component, Component::Normal(_)) {
            anyhow::bail!(
                "attachment materialization path contains unsupported component: {}",
                path.display()
            );
        }
    }
    Ok(())
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

fn looks_like_sha(value: &str) -> bool {
    let len = value.len();
    if !(7..=40).contains(&len) {
        return false;
    }
    value.chars().all(|c| c.is_ascii_hexdigit())
}

fn looks_like_remote_repo_source(source: &str) -> bool {
    if source.contains("://") {
        return true;
    }
    let Some((user_host, path)) = source.split_once(':') else {
        return false;
    };
    if path.is_empty() {
        return false;
    }
    if user_host.contains('/') || user_host.contains('\\') {
        return false;
    }
    if user_host == "." || user_host == ".." {
        return false;
    }
    if user_host.len() == 1 && user_host.chars().all(|ch| ch.is_ascii_alphabetic()) {
        return false;
    }
    true
}

fn validate_reference_repo_source(source: &str) -> Result<()> {
    if looks_like_remote_repo_source(source) || Path::new(source).is_absolute() {
        return Ok(());
    }
    anyhow::bail!(
        "reference_repo local source must be an absolute path or repository URL: {source}"
    );
}

#[cfg(test)]
mod tests;
