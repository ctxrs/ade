use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ctx_core::ids::{WorkspaceId, WorktreeId};
use ctx_core::models::{Workspace, WorkspaceAttachment, Worktree, WorktreeAttachmentMount};
use ctx_execution_runtime::ExecutionSettings;
use ctx_store::Store;
use ctx_worktree_data_plane::WorktreeDataPlane;

mod container_mounts;
mod mount_files;

pub use ctx_workspace_services::workspace_attachments::AttachmentConfig;
pub use mount_files::{
    ensure_mount_in_worktree, materialized_path_for_attachment, remove_mount_path_in_worktree,
    revision_key, sanitize_mount_relpath, validate_mount_path_in_worktree,
};

pub use container_mounts::{cleanup_removed_attachment, ensure_attachment_mount};

#[async_trait::async_trait]
pub trait WorkspaceAttachmentMountHost: Send + Sync {
    fn data_root(&self) -> &Path;
    fn daemon_url(&self) -> &str;

    async fn get_worktree(&self, worktree_id: WorktreeId) -> Result<Option<Worktree>>;
    async fn workspace_store(&self, workspace_id: WorkspaceId) -> Result<Store>;
    async fn resolve_worktree_data_plane(&self, worktree: &Worktree) -> Result<WorktreeDataPlane>;
    async fn effective_execution_settings(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<ExecutionSettings>;
    async fn ensure_workspace_container_for_worktree(
        &self,
        workspace: &Workspace,
        worktree: &Worktree,
        settings: &ExecutionSettings,
    ) -> Result<()>;
}

pub async fn materialize_attachment(
    data_root: &Path,
    workspace: &Workspace,
    attachment: &WorkspaceAttachment,
    refresh: bool,
) -> Result<ctx_workspace_services::workspace_attachments::MaterializationResult> {
    ctx_workspace_services::workspace_attachments::materialize_attachment(
        data_root, workspace, attachment, refresh,
    )
    .await
}

pub async fn ensure_git_exclude<H>(
    host: &H,
    workspace: &Workspace,
    worktree_id: WorktreeId,
    worktree_root: &Path,
) -> Result<()>
where
    H: WorkspaceAttachmentMountHost,
{
    let worktree = host
        .get_worktree(worktree_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("worktree not found for attachment git exclude"))?;
    let data_plane = host.resolve_worktree_data_plane(&worktree).await?;
    if matches!(
        data_plane.execution_mode,
        ctx_execution_runtime::ExecutionMode::Sandbox
    ) {
        return container_mounts::container_ensure_git_exclude(
            host,
            workspace,
            worktree_id,
            worktree_root,
        )
        .await;
    }
    let git_dir = resolve_git_dir(worktree_root).await?;
    let common_git_dir = resolve_common_git_dir(&git_dir).await?;
    let git_info = common_git_dir.join("info");
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

async fn resolve_common_git_dir(git_dir: &Path) -> Result<PathBuf> {
    let commondir = git_dir.join("commondir");
    let meta = match tokio::fs::symlink_metadata(&commondir).await {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(git_dir.to_path_buf());
        }
        Err(err) => return Err(err).with_context(|| format!("reading {}", commondir.display())),
    };
    if !meta.is_file() {
        return Ok(git_dir.to_path_buf());
    }
    let raw = tokio::fs::read_to_string(&commondir)
        .await
        .with_context(|| format!("reading {}", commondir.display()))?;
    let path = PathBuf::from(raw.trim());
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(git_dir.join(path))
    }
}
