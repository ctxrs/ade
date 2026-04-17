use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::Utc;

mod container_mounts;

use self::container_mounts::{
    cleanup_removed_attachment as cleanup_removed_attachment_mounts, container_ensure_git_exclude,
    ensure_attachment_mount,
};

use ctx_core::ids::{WorkspaceAttachmentId, WorkspaceId, WorktreeId};
use ctx_core::models::{
    Workspace, WorkspaceAttachment, WorkspaceAttachmentKind, WorkspaceAttachmentStatus, Worktree,
    WorktreeAttachmentMount, WorktreeAttachmentStatus,
};
use ctx_workspace_services::workspace_attachments::{self, MaterializationResult};

use crate::daemon::{AppState, AttachmentMaterializationTask};
use crate::execution_effective;
use crate::worktree_data_plane::resolve_worktree_data_plane;
use ctx_harness_runtime::sandbox_container_command;
use ctx_sandbox_contract::CTX_CONTAINER_WORKSPACE_ROOT;
use ctx_workspace_container::workspace_container_name;
use ctx_worktree_data_plane::apply_data_plane_to_execution_settings;

const CONTAINER_ATTACHMENTS_SUBDIR: &str = "attachments";

pub use ctx_workspace_services::workspace_attachments::AttachmentConfig;

pub async fn sync_workspace_attachments(
    state: Arc<AppState>,
    workspace: &Workspace,
    refresh: bool,
) -> Result<Vec<WorkspaceAttachment>> {
    let result =
        workspace_attachments::sync_workspace_attachments(state.as_ref(), workspace, refresh)
            .await?;
    for plan in result.plans {
        spawn_attachment_materialization(
            Arc::clone(&state),
            workspace.clone(),
            plan.id,
            plan.refresh,
        )
        .await;
    }
    Ok(result.attachments)
}

pub async fn upsert_workspace_attachment(
    state: &AppState,
    workspace_id: WorkspaceId,
    cfg: AttachmentConfig,
) -> Result<WorkspaceAttachment> {
    workspace_attachments::upsert_workspace_attachment(state, workspace_id, cfg).await
}

pub async fn delete_workspace_attachment(
    state: &AppState,
    workspace_id: WorkspaceId,
    kind: WorkspaceAttachmentKind,
    name: &str,
) -> Result<bool> {
    let Some(target) =
        workspace_attachments::find_workspace_attachment(state, workspace_id, kind, name).await?
    else {
        return Ok(false);
    };
    cancel_attachment_materialization(state, target.id).await;
    workspace_attachments::delete_workspace_attachment(state, &target).await?;
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
    if let Err(err) = workspace_attachments::run_attachment_materialization(
        state.as_ref(),
        &workspace,
        attachment_id,
        refresh,
    )
    .await
    {
        tracing::warn!("attachment sync failed: {err:#}");
    }
}

#[async_trait::async_trait]
impl workspace_attachments::WorkspaceAttachmentsHost for AppState {
    fn data_root(&self) -> &Path {
        &self.core.data_root
    }

    async fn list_workspace_attachments(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<WorkspaceAttachment>> {
        let store = self.store_for_workspace(workspace_id).await?;
        store.list_workspace_attachments(workspace_id).await
    }

    async fn get_workspace_attachment(
        &self,
        workspace_id: WorkspaceId,
        attachment_id: WorkspaceAttachmentId,
    ) -> Result<Option<WorkspaceAttachment>> {
        let store = self.store_for_workspace(workspace_id).await?;
        store.get_workspace_attachment(attachment_id).await
    }

    async fn upsert_workspace_attachment(&self, attachment: &WorkspaceAttachment) -> Result<()> {
        let store = self.store_for_workspace(attachment.workspace_id).await?;
        store.upsert_workspace_attachment(attachment).await
    }

    async fn update_workspace_attachment_status(
        &self,
        workspace_id: WorkspaceId,
        attachment_id: WorkspaceAttachmentId,
        status: WorkspaceAttachmentStatus,
        last_sync_at: Option<chrono::DateTime<Utc>>,
        error_message: Option<String>,
        updated_at: chrono::DateTime<Utc>,
    ) -> Result<()> {
        let store = self.store_for_workspace(workspace_id).await?;
        store
            .update_workspace_attachment_status(
                attachment_id,
                status,
                last_sync_at,
                error_message,
                updated_at,
            )
            .await
    }

    async fn delete_workspace_attachment_record(
        &self,
        workspace_id: WorkspaceId,
        attachment_id: WorkspaceAttachmentId,
    ) -> Result<()> {
        let store = self.store_for_workspace(workspace_id).await?;
        store.delete_workspace_attachment(attachment_id).await
    }

    async fn attachment_became_ready(
        &self,
        workspace: &Workspace,
        attachment: &WorkspaceAttachment,
    ) -> Result<()> {
        ensure_workspace_attachments_for_worktrees_with_attachments(
            self,
            workspace,
            std::slice::from_ref(attachment),
            false,
            false,
        )
        .await
    }

    async fn cleanup_removed_attachment(&self, attachment: &WorkspaceAttachment) -> Result<()> {
        cleanup_removed_attachment_mounts(self, attachment).await
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

    let data_plane = resolve_worktree_data_plane(state, worktree).await?;
    let effective = execution_effective::effective_execution_settings(state, workspace.id).await?;
    let _effective = apply_data_plane_to_execution_settings(&effective, &data_plane)?;
    let worktree_root = data_plane.live_worktree_root;
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

async fn materialize_attachment(
    state: &AppState,
    workspace: &Workspace,
    attachment: &WorkspaceAttachment,
    refresh: bool,
) -> Result<MaterializationResult> {
    workspace_attachments::materialize_attachment(
        &state.core.data_root,
        workspace,
        attachment,
        refresh,
    )
    .await
}

async fn ensure_git_exclude(
    state: &AppState,
    workspace: &Workspace,
    worktree_id: WorktreeId,
    worktree_root: &Path,
) -> Result<()> {
    let worktree = state
        .global_store()
        .get_worktree(worktree_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("worktree not found for attachment git exclude"))?;
    let data_plane = resolve_worktree_data_plane(state, &worktree).await?;
    if matches!(
        data_plane.execution_mode,
        crate::settings::ExecutionMode::Sandbox
    ) {
        return container_ensure_git_exclude(state, workspace, worktree_id, worktree_root).await;
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
        tracing::debug!("symlink failed ({err}); falling back to copy");
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

fn materialized_root_for_attachment(state: &AppState, attachment: &WorkspaceAttachment) -> PathBuf {
    workspace_attachments::materialized_root_for_attachment(&state.core.data_root, attachment)
}

fn materialized_path_for_attachment(state: &AppState, attachment: &WorkspaceAttachment) -> PathBuf {
    workspace_attachments::materialized_path_for_attachment(&state.core.data_root, attachment)
}

fn sanitize_mount_relpath(value: &str) -> Result<PathBuf> {
    workspace_attachments::sanitize_mount_relpath(value)
}

fn revision_key(attachment: &WorkspaceAttachment) -> String {
    workspace_attachments::revision_key(attachment)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command as StdCommand;

    fn git(args: &[&str], cwd: &Path) {
        let status = StdCommand::new("git")
            .args(args)
            .current_dir(cwd)
            .status()
            .expect("run git command");
        assert!(status.success(), "git command failed: {args:?}");
    }

    fn init_git_repo(root: &Path) {
        git(&["init"], root);
        git(&["symbolic-ref", "HEAD", "refs/heads/main"], root);
    }

    #[tokio::test]
    async fn resolve_common_git_dir_follows_linked_worktree_commondir() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("create repo root");
        init_git_repo(&repo_root);
        git(&["config", "user.name", "Test User"], &repo_root);
        git(&["config", "user.email", "test@example.com"], &repo_root);
        std::fs::write(repo_root.join("README.md"), "hello\n").expect("write readme");
        git(&["add", "README.md"], &repo_root);
        git(&["commit", "-m", "initial"], &repo_root);

        let worktree_root = temp.path().join("worktree");
        git(
            &[
                "worktree",
                "add",
                "-b",
                "ctx/test-worktree",
                worktree_root.to_str().expect("worktree path"),
            ],
            &repo_root,
        );

        let git_dir = resolve_git_dir(&worktree_root)
            .await
            .expect("resolve git dir");
        let common_git_dir = resolve_common_git_dir(&git_dir)
            .await
            .expect("resolve common git dir");
        assert_ne!(git_dir, common_git_dir);
        assert!(common_git_dir.join("info").is_dir());
        assert!(common_git_dir.join("objects").exists());
    }
}
