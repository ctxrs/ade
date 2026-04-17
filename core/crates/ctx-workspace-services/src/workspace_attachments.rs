use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ctx_core::ids::{WorkspaceAttachmentId, WorkspaceId};
use ctx_core::models::{
    AttachmentMode, AttachmentUpdatePolicy, Workspace, WorkspaceAttachment,
    WorkspaceAttachmentKind, WorkspaceAttachmentStatus,
};
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;
use tokio::process::Command;
use toml::Value as TomlValue;

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
            attachment.status = WorkspaceAttachmentStatus::Pending;
            attachment.error_message = None;
            attachment.updated_at = now;
            plans.push(AttachmentSyncPlan {
                id: attachment.id,
                refresh: should_refresh,
            });
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
    let attachment = normalize_attachment_config(workspace_id, cfg, existing);
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
    if value.trim().is_empty() {
        anyhow::bail!("mount_relpath must not be empty");
    }
    let path = PathBuf::from(value);
    if path.is_absolute() {
        anyhow::bail!("mount_relpath must be relative: {value}");
    }
    for part in path.components() {
        if matches!(part, std::path::Component::ParentDir) {
            anyhow::bail!("mount_relpath must not contain '..': {value}");
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
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        subpath: cfg
            .subpath
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
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

async fn materialize_reference_repo(
    data_root: &Path,
    attachment: &WorkspaceAttachment,
    refresh: bool,
) -> Result<MaterializationResult> {
    let revision = revision_key(attachment);
    let dest = materialized_path_for_attachment(data_root, attachment);
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
    data_root: &Path,
    workspace: &Workspace,
    attachment: &WorkspaceAttachment,
    refresh: bool,
) -> Result<MaterializationResult> {
    let revision = revision_key(attachment);
    let dest = materialized_path_for_attachment(data_root, attachment);
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

    let mut cmd = if script_path.extension().and_then(|value| value.to_str()) == Some("py") {
        let mut cmd = Command::new("python3");
        cmd.arg(&script_path);
        cmd
    } else if script_path.extension().and_then(|value| value.to_str()) == Some("sh") {
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

fn attachment_store_root(data_root: &Path) -> PathBuf {
    data_root.join("attachments")
}

fn resolve_workspace_path(workspace_root: &str, raw: &str) -> PathBuf {
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        path
    } else {
        Path::new(workspace_root).join(path)
    }
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

#[cfg(test)]
mod tests {
    use super::{
        default_mount_relpath, normalize_attachment_config, revision_key, sanitize_mount_relpath,
        AttachmentConfig,
    };
    use ctx_core::ids::WorkspaceId;
    use ctx_core::models::{
        AttachmentMode, AttachmentUpdatePolicy, WorkspaceAttachmentKind, WorkspaceAttachmentStatus,
    };

    #[test]
    fn default_mount_relpath_uses_kind_specific_roots() {
        assert_eq!(
            default_mount_relpath(&WorkspaceAttachmentKind::ReferenceRepo, "My Docs"),
            ".ctx/attachments/refs/my-docs"
        );
        assert_eq!(
            default_mount_relpath(&WorkspaceAttachmentKind::DocMirror, "API Guide"),
            ".ctx/attachments/docs/api-guide"
        );
    }

    #[test]
    fn sanitize_mount_relpath_rejects_invalid_paths() {
        assert!(sanitize_mount_relpath(".ctx/attachments/docs/api-guide").is_ok());
        assert!(sanitize_mount_relpath("").is_err());
        assert!(sanitize_mount_relpath("/absolute/path").is_err());
        assert!(sanitize_mount_relpath("../escape").is_err());
    }

    #[test]
    fn normalize_attachment_config_preserves_existing_identity() {
        let workspace_id = WorkspaceId::new();
        let existing = normalize_attachment_config(
            workspace_id,
            AttachmentConfig {
                kind: WorkspaceAttachmentKind::ReferenceRepo,
                name: "Docs".to_string(),
                source: "https://example.com/repo.git".to_string(),
                revision: None,
                subpath: None,
                mount_relpath: None,
                mode: Some(AttachmentMode::Ro),
                update_policy: Some(AttachmentUpdatePolicy::Manual),
            },
            None,
        );

        let updated = normalize_attachment_config(
            workspace_id,
            AttachmentConfig {
                kind: WorkspaceAttachmentKind::ReferenceRepo,
                name: "Docs".to_string(),
                source: "https://example.com/repo.git".to_string(),
                revision: Some("main".to_string()),
                subpath: Some("guide".to_string()),
                mount_relpath: Some(".ctx/attachments/refs/docs".to_string()),
                mode: Some(AttachmentMode::Ro),
                update_policy: Some(AttachmentUpdatePolicy::OnOpen),
            },
            Some(existing.clone()),
        );

        assert_eq!(updated.id, existing.id);
        assert_eq!(updated.created_at, existing.created_at);
        assert_eq!(updated.status, WorkspaceAttachmentStatus::Pending);
        assert_eq!(updated.mount_relpath, ".ctx/attachments/refs/docs");
        assert_eq!(updated.revision.as_deref(), Some("main"));
        assert_eq!(updated.subpath.as_deref(), Some("guide"));
        assert_eq!(updated.update_policy, AttachmentUpdatePolicy::OnOpen);
    }

    #[test]
    fn revision_key_defaults_and_sanitizes() {
        let attachment = normalize_attachment_config(
            WorkspaceId::new(),
            AttachmentConfig {
                kind: WorkspaceAttachmentKind::DocMirror,
                name: "Docs".to_string(),
                source: "https://example.com".to_string(),
                revision: Some("Feature/Branch".to_string()),
                subpath: None,
                mount_relpath: None,
                mode: None,
                update_policy: None,
            },
            None,
        );
        assert_eq!(revision_key(&attachment), "feature-branch");
    }
}
