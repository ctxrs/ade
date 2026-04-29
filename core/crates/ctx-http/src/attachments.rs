use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};

pub(crate) mod container_mounts;

use self::container_mounts::container_ensure_git_exclude;
pub(crate) use self::container_mounts::{
    cleanup_removed_attachment as cleanup_removed_attachment_mounts, ensure_attachment_mount,
};

use ctx_core::ids::{WorkspaceId, WorktreeId};
use ctx_core::models::{
    AttachmentMode, Workspace, WorkspaceAttachment, WorkspaceAttachmentKind, Worktree,
    WorktreeAttachmentMount,
};
use ctx_workspace_services::workspace_attachments::{self, MaterializationResult};

use crate::daemon::AppState;
use crate::worktree_data_plane::resolve_worktree_data_plane;
use ctx_harness_runtime::sandbox_container_command;
use ctx_sandbox_contract::CTX_CONTAINER_WORKSPACE_ROOT;
use ctx_workspace_container::workspace_container_name;

const CONTAINER_ATTACHMENTS_SUBDIR: &str = "attachments";

pub use ctx_workspace_services::workspace_attachments::AttachmentConfig;

pub async fn sync_workspace_attachments(
    state: Arc<AppState>,
    workspace: &Workspace,
    refresh: bool,
) -> Result<Vec<WorkspaceAttachment>> {
    crate::daemon::workspaces::attachments::sync_workspace_attachments(state, workspace, refresh)
        .await
}

pub async fn upsert_workspace_attachment(
    state: &AppState,
    workspace_id: WorkspaceId,
    cfg: AttachmentConfig,
) -> Result<WorkspaceAttachment> {
    crate::daemon::workspaces::attachments::upsert_workspace_attachment(state, workspace_id, cfg)
        .await
}

pub async fn delete_workspace_attachment(
    state: &AppState,
    workspace_id: WorkspaceId,
    kind: WorkspaceAttachmentKind,
    name: &str,
) -> Result<bool> {
    crate::daemon::workspaces::attachments::delete_workspace_attachment(
        state,
        workspace_id,
        kind,
        name,
    )
    .await
}

pub async fn ensure_worktree_attachment_mounts(
    state: &AppState,
    workspace: &Workspace,
    worktree: &Worktree,
    refresh: bool,
) -> Result<Vec<WorktreeAttachmentMount>> {
    crate::daemon::workspaces::attachments::ensure_worktree_attachment_mounts(
        state, workspace, worktree, refresh,
    )
    .await
}

pub async fn ensure_worktree_attachment_mounts_if_materialized(
    state: &AppState,
    workspace: &Workspace,
    worktree: &Worktree,
) -> Result<Vec<WorktreeAttachmentMount>> {
    crate::daemon::workspaces::attachments::ensure_worktree_attachment_mounts_if_materialized(
        state, workspace, worktree,
    )
    .await
}

pub async fn ensure_workspace_attachments_for_worktrees(
    state: &AppState,
    workspace: &Workspace,
    refresh: bool,
) -> Result<()> {
    crate::daemon::workspaces::attachments::ensure_workspace_attachments_for_worktrees(
        state, workspace, refresh,
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
    crate::daemon::workspaces::attachments::ensure_workspace_attachments_for_worktrees_with_attachments(
        state,
        workspace,
        attachments,
        refresh,
        materialize,
    )
    .await
}

pub(crate) async fn materialize_attachment(
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

pub(crate) async fn ensure_git_exclude(
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

pub(crate) async fn remove_mount_path(target: &Path) -> Result<()> {
    if let Ok(meta) = tokio::fs::symlink_metadata(target).await {
        if meta.file_type().is_symlink() || meta.is_file() {
            let target = target.to_path_buf();
            let target_for_clear = target.clone();
            tokio::task::spawn_blocking(move || clear_read_only_mode(&target_for_clear)).await??;
            tokio::fs::remove_file(target).await?;
        } else if meta.is_dir() {
            let target = target.to_path_buf();
            let target_for_clear = target.clone();
            tokio::task::spawn_blocking(move || clear_read_only_mode(&target_for_clear)).await??;
            tokio::fs::remove_dir_all(target).await?;
        }
    }
    Ok(())
}

pub(crate) async fn remove_mount_path_in_worktree(
    worktree_root: &Path,
    target: &Path,
) -> Result<()> {
    validate_mount_parent_chain(worktree_root, target, true)?;
    remove_mount_path(target).await
}

pub(crate) async fn ensure_mount_in_worktree(
    worktree_root: &Path,
    mount_relpath: &Path,
    source: &Path,
    mode: AttachmentMode,
) -> Result<PathBuf> {
    let target = ensure_mount_parent_chain(worktree_root, mount_relpath)?;
    ensure_mount(&target, source, mode).await?;
    Ok(target)
}

fn ensure_mount_parent_chain(worktree_root: &Path, mount_relpath: &Path) -> Result<PathBuf> {
    validate_safe_relative_mount_path(mount_relpath)?;
    let target = worktree_root.join(mount_relpath);
    let parent = mount_relpath
        .parent()
        .ok_or_else(|| anyhow::anyhow!("attachment mount path must have a parent"))?;
    let mut current = worktree_root.to_path_buf();
    for component in parent.components() {
        let std::path::Component::Normal(segment) = component else {
            anyhow::bail!(
                "attachment mount path contains unsupported component: {}",
                mount_relpath.display()
            );
        };
        current.push(segment);
        match std::fs::symlink_metadata(&current) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    anyhow::bail!(
                        "attachment mount parent must not be a symlink: {}",
                        current.display()
                    );
                }
                if !meta.is_dir() {
                    anyhow::bail!(
                        "attachment mount parent must be a directory: {}",
                        current.display()
                    );
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&current).with_context(|| {
                    format!("creating attachment mount parent {}", current.display())
                })?;
                let meta = std::fs::symlink_metadata(&current).with_context(|| {
                    format!("verifying attachment mount parent {}", current.display())
                })?;
                if meta.file_type().is_symlink() || !meta.is_dir() {
                    anyhow::bail!(
                        "attachment mount parent was not created as a directory: {}",
                        current.display()
                    );
                }
            }
            Err(err) => {
                return Err(err).with_context(|| {
                    format!("reading attachment mount parent {}", current.display())
                });
            }
        }
    }
    Ok(target)
}

fn validate_mount_parent_chain(
    worktree_root: &Path,
    target: &Path,
    allow_missing: bool,
) -> Result<()> {
    let mount_relpath = target.strip_prefix(worktree_root).with_context(|| {
        format!(
            "attachment mount path {} is outside worktree {}",
            target.display(),
            worktree_root.display()
        )
    })?;
    validate_safe_relative_mount_path(mount_relpath)?;
    let parent = mount_relpath
        .parent()
        .ok_or_else(|| anyhow::anyhow!("attachment mount path must have a parent"))?;
    let mut current = worktree_root.to_path_buf();
    for component in parent.components() {
        let std::path::Component::Normal(segment) = component else {
            anyhow::bail!(
                "attachment mount path contains unsupported component: {}",
                mount_relpath.display()
            );
        };
        current.push(segment);
        match std::fs::symlink_metadata(&current) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    anyhow::bail!(
                        "attachment mount parent must not be a symlink: {}",
                        current.display()
                    );
                }
                if !meta.is_dir() {
                    anyhow::bail!(
                        "attachment mount parent must be a directory: {}",
                        current.display()
                    );
                }
            }
            Err(err) if allow_missing && err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(());
            }
            Err(err) => {
                return Err(err).with_context(|| {
                    format!("reading attachment mount parent {}", current.display())
                });
            }
        }
    }
    Ok(())
}

fn validate_safe_relative_mount_path(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty() {
        anyhow::bail!("attachment mount path must not be empty");
    }
    if path.is_absolute() {
        anyhow::bail!("attachment mount path must be relative: {}", path.display());
    }
    for component in path.components() {
        match component {
            std::path::Component::Normal(_) => {}
            _ => anyhow::bail!(
                "attachment mount path contains unsupported component: {}",
                path.display()
            ),
        }
    }
    Ok(())
}

pub(crate) async fn ensure_mount(target: &Path, source: &Path, mode: AttachmentMode) -> Result<()> {
    if let Ok(meta) = tokio::fs::symlink_metadata(target).await {
        if meta.file_type().is_symlink() {
            if let Ok(current) = tokio::fs::read_link(target).await {
                if current == source && mode == AttachmentMode::Rw {
                    return Ok(());
                }
            }
            tokio::fs::remove_file(target).await?;
        } else if meta.is_dir() {
            let target = target.to_path_buf();
            let target_for_clear = target.clone();
            tokio::task::spawn_blocking(move || clear_read_only_mode(&target_for_clear)).await??;
            tokio::fs::remove_dir_all(target).await?;
        } else {
            let target = target.to_path_buf();
            let target_for_clear = target.clone();
            tokio::task::spawn_blocking(move || clear_read_only_mode(&target_for_clear)).await??;
            tokio::fs::remove_file(target).await?;
        }
    }

    if mode == AttachmentMode::Ro {
        let source = source.to_path_buf();
        let target = target.to_path_buf();
        let target_for_copy = target.clone();
        tokio::task::spawn_blocking(move || copy_path_recursive(&source, &target_for_copy))
            .await??;
        tokio::task::spawn_blocking(move || apply_read_only_mode(&target)).await??;
        return Ok(());
    }

    if let Err(err) = try_symlink_path(source, target).await {
        tracing::debug!("symlink failed ({err}); falling back to copy");
        let source = source.to_path_buf();
        let target = target.to_path_buf();
        tokio::task::spawn_blocking(move || copy_path_recursive(&source, &target)).await??;
    }
    Ok(())
}

async fn try_symlink_path(source: &Path, target: &Path) -> Result<()> {
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
        use std::os::windows::fs::{symlink_dir, symlink_file};
        let source_meta = tokio::fs::metadata(source)
            .await
            .with_context(|| format!("stat attachment source {}", source.display()))?;
        tokio::task::spawn_blocking({
            let source = source.to_path_buf();
            let target = target.to_path_buf();
            move || {
                if source_meta.is_dir() {
                    symlink_dir(source, target)
                } else {
                    symlink_file(source, target)
                }
            }
        })
        .await??;
        Ok(())
    }
}

fn copy_path_recursive(source: &Path, target: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(source)
        .with_context(|| format!("reading attachment source metadata {}", source.display()))?;
    if metadata.file_type().is_symlink() {
        copy_symlink(source, target)?;
        return Ok(());
    }
    if metadata.is_dir() {
        std::fs::create_dir_all(target)?;
        for entry in std::fs::read_dir(source)? {
            let entry = entry?;
            let dest = target.join(entry.file_name());
            copy_path_recursive(&entry.path(), &dest)?;
        }
        return Ok(());
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(source, target)
        .with_context(|| format!("copying attachment file {}", source.display()))?;
    Ok(())
}

fn apply_read_only_mode(path: &Path) -> Result<()> {
    apply_read_only_mode_recursive(path)
}

fn clear_read_only_mode(path: &Path) -> Result<()> {
    clear_read_only_mode_recursive(path)
}

fn apply_read_only_mode_recursive(path: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("reading attachment metadata {}", path.display()))?;
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    if metadata.is_dir() {
        for entry in std::fs::read_dir(path)
            .with_context(|| format!("reading attachment dir {}", path.display()))?
        {
            let entry = entry?;
            apply_read_only_mode_recursive(&entry.path())?;
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = metadata.permissions();
        permissions.set_mode(permissions.mode() & !0o222);
        std::fs::set_permissions(path, permissions)
            .with_context(|| format!("setting read-only permissions on {}", path.display()))?;
    }
    #[cfg(not(unix))]
    {
        let mut permissions = metadata.permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(path, permissions)
            .with_context(|| format!("setting read-only permissions on {}", path.display()))?;
    }
    Ok(())
}

fn clear_read_only_mode_recursive(path: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("reading attachment metadata {}", path.display()))?;
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    if metadata.is_dir() {
        for entry in std::fs::read_dir(path)
            .with_context(|| format!("reading attachment dir {}", path.display()))?
        {
            let entry = entry?;
            clear_read_only_mode_recursive(&entry.path())?;
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = metadata.permissions();
        permissions.set_mode(permissions.mode() | 0o200);
        std::fs::set_permissions(path, permissions)
            .with_context(|| format!("setting writable permissions on {}", path.display()))?;
    }
    #[cfg(not(unix))]
    {
        let mut permissions = metadata.permissions();
        permissions.set_readonly(false);
        std::fs::set_permissions(path, permissions)
            .with_context(|| format!("setting writable permissions on {}", path.display()))?;
    }
    Ok(())
}

fn copy_symlink(source: &Path, target: &Path) -> Result<()> {
    let link_target = std::fs::read_link(source)
        .with_context(|| format!("reading attachment symlink {}", source.display()))?;
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&link_target, target)
            .with_context(|| format!("copying attachment symlink {}", source.display()))?;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::{symlink_dir, symlink_file};
        let target_metadata = std::fs::metadata(source)
            .with_context(|| format!("reading attachment symlink target {}", source.display()))?;
        if target_metadata.is_dir() {
            symlink_dir(&link_target, target)
                .with_context(|| format!("copying attachment symlink {}", source.display()))?;
        } else {
            symlink_file(&link_target, target)
                .with_context(|| format!("copying attachment symlink {}", source.display()))?;
        }
    }
    Ok(())
}

pub(crate) fn materialized_root_for_attachment(
    state: &AppState,
    attachment: &WorkspaceAttachment,
) -> PathBuf {
    workspace_attachments::materialized_root_for_attachment(&state.core.data_root, attachment)
}

pub(crate) fn materialized_path_for_attachment(
    state: &AppState,
    attachment: &WorkspaceAttachment,
) -> PathBuf {
    workspace_attachments::materialized_path_for_attachment(&state.core.data_root, attachment)
}

pub(crate) fn sanitize_mount_relpath(value: &str) -> Result<PathBuf> {
    workspace_attachments::sanitize_mount_relpath(value)
}

pub(crate) fn revision_key(attachment: &WorkspaceAttachment) -> String {
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

    #[cfg(unix)]
    #[tokio::test]
    async fn ensure_mount_applies_read_only_mode() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        std::fs::create_dir_all(&source).expect("create source");
        std::fs::write(source.join("notes.txt"), "hello\n").expect("write source file");

        ensure_mount(&target, &source, AttachmentMode::Ro)
            .await
            .expect("mount ro attachment");

        let err = std::fs::write(target.join("notes.txt"), "mutated\n")
            .expect_err("ro attachment mount should reject writes");
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
        std::fs::write(source.join("source-writable.txt"), "still writable\n")
            .expect("ro attachment mount should not mutate source writability");
    }

    #[tokio::test]
    async fn ensure_mount_switches_from_ro_copy_to_rw_mount() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        std::fs::create_dir_all(&source).expect("create source");
        std::fs::write(source.join("notes.txt"), "hello\n").expect("write source file");

        ensure_mount(&target, &source, AttachmentMode::Ro)
            .await
            .expect("mount ro attachment");
        ensure_mount(&target, &source, AttachmentMode::Rw)
            .await
            .expect("remount rw attachment");

        std::fs::write(target.join("notes.txt"), "mutated\n")
            .expect("rw attachment remount should allow writes");
    }

    #[tokio::test]
    async fn remove_mount_path_deletes_read_only_mount_copy() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        std::fs::create_dir_all(&source).expect("create source");
        std::fs::write(source.join("notes.txt"), "hello\n").expect("write source file");

        ensure_mount(&target, &source, AttachmentMode::Ro)
            .await
            .expect("mount ro attachment");
        remove_mount_path(&target)
            .await
            .expect("remove ro attachment mount");

        assert!(!target.exists(), "ro attachment mount should be removed");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn ensure_mount_in_worktree_rejects_symlinked_ctx_parent() {
        let temp = tempfile::tempdir().expect("tempdir");
        let worktree = temp.path().join("worktree");
        let outside = temp.path().join("outside");
        let source = temp.path().join("source");
        std::fs::create_dir_all(&worktree).expect("create worktree");
        std::fs::create_dir_all(&outside).expect("create outside");
        std::fs::create_dir_all(&source).expect("create source");
        std::fs::write(source.join("notes.txt"), "hello\n").expect("write source file");
        std::os::unix::fs::symlink(&outside, worktree.join(".ctx")).expect("symlink .ctx");

        let err = ensure_mount_in_worktree(
            &worktree,
            Path::new(".ctx/attachments/docs/docs"),
            &source,
            AttachmentMode::Ro,
        )
        .await
        .expect_err("symlinked .ctx parent should fail closed");

        assert!(format!("{err:#}").contains("must not be a symlink"));
        assert!(
            !outside.join("attachments").exists(),
            "mount creation must not follow .ctx symlink outside the worktree"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn remove_mount_path_in_worktree_rejects_symlinked_attachments_parent() {
        let temp = tempfile::tempdir().expect("tempdir");
        let worktree = temp.path().join("worktree");
        let outside = temp.path().join("outside");
        std::fs::create_dir_all(worktree.join(".ctx")).expect("create .ctx");
        std::fs::create_dir_all(outside.join("docs").join("docs")).expect("create outside mount");
        std::fs::write(
            outside.join("docs").join("docs").join("notes.txt"),
            "keep\n",
        )
        .expect("write outside file");
        std::os::unix::fs::symlink(&outside, worktree.join(".ctx").join("attachments"))
            .expect("symlink attachments");

        let target = worktree.join(".ctx/attachments/docs/docs");
        let err = remove_mount_path_in_worktree(&worktree, &target)
            .await
            .expect_err("symlinked .ctx/attachments parent should fail closed");

        assert!(format!("{err:#}").contains("must not be a symlink"));
        assert!(
            outside.join("docs").join("docs").join("notes.txt").exists(),
            "mount cleanup must not delete through .ctx/attachments symlink"
        );
    }

    #[tokio::test]
    async fn ensure_mount_leaves_rw_mounts_writable() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        std::fs::create_dir_all(&source).expect("create source");
        std::fs::write(source.join("notes.txt"), "hello\n").expect("write source file");

        ensure_mount(&target, &source, AttachmentMode::Rw)
            .await
            .expect("mount rw attachment");

        std::fs::write(target.join("notes.txt"), "mutated\n")
            .expect("rw attachment mount should allow writes");
    }
}
