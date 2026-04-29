use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ctx_core::models::{AttachmentMode, WorkspaceAttachment};
use ctx_workspace_services::workspace_attachments;

use crate::daemon::AppState;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum SymlinkCopyMode {
    Preserve,
    DereferenceFiles,
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

pub(crate) fn validate_mount_path_in_worktree(worktree_root: &Path, target: &Path) -> Result<()> {
    validate_mount_parent_chain(worktree_root, target, true)
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
        tokio::task::spawn_blocking(move || {
            copy_path_recursive(&source, &target_for_copy, SymlinkCopyMode::DereferenceFiles)
        })
        .await??;
        tokio::task::spawn_blocking(move || apply_read_only_mode(&target)).await??;
        return Ok(());
    }

    if let Err(err) = try_symlink_path(source, target).await {
        tracing::debug!("symlink failed ({err}); falling back to copy");
        let source = source.to_path_buf();
        let target = target.to_path_buf();
        tokio::task::spawn_blocking(move || {
            copy_path_recursive(&source, &target, SymlinkCopyMode::Preserve)
        })
        .await??;
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

fn copy_path_recursive(source: &Path, target: &Path, symlink_mode: SymlinkCopyMode) -> Result<()> {
    let source_root = std::fs::canonicalize(source)
        .with_context(|| format!("canonicalizing attachment source {}", source.display()))?;
    copy_path_recursive_inner(&source_root, source, target, symlink_mode)
}

fn copy_path_recursive_inner(
    source_root: &Path,
    source: &Path,
    target: &Path,
    symlink_mode: SymlinkCopyMode,
) -> Result<()> {
    let metadata = std::fs::symlink_metadata(source)
        .with_context(|| format!("reading attachment source metadata {}", source.display()))?;
    if metadata.file_type().is_symlink() {
        match symlink_mode {
            SymlinkCopyMode::Preserve => copy_symlink(source, target)?,
            SymlinkCopyMode::DereferenceFiles => {
                copy_dereferenced_symlink_file(source_root, source, target)?
            }
        }
        return Ok(());
    }
    if metadata.is_dir() {
        std::fs::create_dir_all(target)?;
        for entry in std::fs::read_dir(source)? {
            let entry = entry?;
            let dest = target.join(entry.file_name());
            copy_path_recursive_inner(source_root, &entry.path(), &dest, symlink_mode)?;
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

fn copy_dereferenced_symlink_file(source_root: &Path, source: &Path, target: &Path) -> Result<()> {
    let canonical_target = std::fs::canonicalize(source).with_context(|| {
        format!(
            "canonicalizing attachment symlink target {}",
            source.display()
        )
    })?;
    if !canonical_target.starts_with(source_root) {
        anyhow::bail!(
            "read-only attachment copy refuses symlink outside source root: {} -> {}",
            source.display(),
            canonical_target.display()
        );
    }
    let mut source_file = open_dereferenced_symlink_target(&canonical_target, source)?;
    let metadata = source_file
        .metadata()
        .with_context(|| format!("reading attachment symlink target {}", source.display()))?;
    if metadata.is_dir() {
        anyhow::bail!(
            "read-only attachment copy refuses directory symlink: {}",
            source.display()
        );
    }
    if !metadata.is_file() {
        anyhow::bail!(
            "read-only attachment copy refuses non-file symlink target: {}",
            source.display()
        );
    }
    #[cfg(test)]
    run_after_dereferenced_symlink_target_opened_hook(source);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut target_file = std::fs::File::create(target)
        .with_context(|| format!("creating dereferenced attachment copy {}", target.display()))?;
    std::io::copy(&mut source_file, &mut target_file).with_context(|| {
        format!(
            "copying dereferenced attachment symlink {}",
            source.display()
        )
    })?;
    Ok(())
}

fn open_dereferenced_symlink_target(
    canonical_target: &Path,
    source: &Path,
) -> Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;

        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    options.open(canonical_target).with_context(|| {
        format!(
            "opening dereferenced attachment symlink target {}",
            source.display()
        )
    })
}

#[cfg(test)]
thread_local! {
    static AFTER_DEREFERENCED_SYMLINK_TARGET_OPENED_HOOK:
        std::cell::RefCell<Option<fn(&Path)>> = std::cell::RefCell::new(None);
}

#[cfg(test)]
fn run_after_dereferenced_symlink_target_opened_hook(source: &Path) {
    AFTER_DEREFERENCED_SYMLINK_TARGET_OPENED_HOOK.with(|hook| {
        if let Some(hook) = *hook.borrow() {
            hook(source);
        }
    });
}

#[cfg(test)]
fn set_after_dereferenced_symlink_target_opened_hook(
    hook: fn(&Path),
) -> DereferencedSymlinkTargetOpenedHookGuard {
    AFTER_DEREFERENCED_SYMLINK_TARGET_OPENED_HOOK.with(|current| {
        *current.borrow_mut() = Some(hook);
    });
    DereferencedSymlinkTargetOpenedHookGuard
}

#[cfg(test)]
struct DereferencedSymlinkTargetOpenedHookGuard;

#[cfg(test)]
impl Drop for DereferencedSymlinkTargetOpenedHookGuard {
    fn drop(&mut self) {
        AFTER_DEREFERENCED_SYMLINK_TARGET_OPENED_HOOK.with(|hook| {
            *hook.borrow_mut() = None;
        });
    }
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

    #[cfg(unix)]
    #[tokio::test]
    async fn ensure_mount_ro_copy_dereferences_file_symlink() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        std::fs::create_dir_all(&source).expect("create source");
        std::fs::write(source.join("guide.md"), "guide\n").expect("write source file");
        std::os::unix::fs::symlink(source.join("guide.md"), source.join("guide-link"))
            .expect("symlink guide");

        ensure_mount(&target, &source, AttachmentMode::Ro)
            .await
            .expect("mount ro attachment");

        let link_meta =
            std::fs::symlink_metadata(target.join("guide-link")).expect("stat copied link path");
        assert!(
            !link_meta.file_type().is_symlink(),
            "ro copy must not preserve symlinks into daemon materialization"
        );
        let _ = std::fs::write(target.join("guide-link"), "mutated\n");
        assert_eq!(
            std::fs::read_to_string(source.join("guide.md")).expect("read source"),
            "guide\n",
            "writes through copied ro symlink path must not mutate source materialization"
        );
    }

    #[cfg(unix)]
    #[test]
    fn ensure_mount_ro_copy_dereferenced_symlink_uses_validated_open_file() {
        fn swap_race_link_to_outside(source: &Path) {
            if source.file_name() != Some(std::ffi::OsStr::new("race-link")) {
                return;
            }
            let source_dir = source.parent().expect("source symlink parent");
            let outside = source_dir
                .parent()
                .expect("tempdir parent")
                .join("outside-secret.txt");
            std::fs::remove_file(source).expect("replace race symlink");
            std::os::unix::fs::symlink(&outside, source).expect("swap symlink outside");
        }

        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        let outside = temp.path().join("outside-secret.txt");
        std::fs::create_dir_all(&source).expect("create source");
        std::fs::write(source.join("good.txt"), "good\n").expect("write internal source file");
        std::fs::write(&outside, "outside\n").expect("write outside file");
        std::os::unix::fs::symlink(source.join("good.txt"), source.join("race-link"))
            .expect("symlink internal file");

        let _hook = set_after_dereferenced_symlink_target_opened_hook(swap_race_link_to_outside);
        copy_path_recursive(&source, &target, SymlinkCopyMode::DereferenceFiles)
            .expect("copy ro attachment tree");

        assert_eq!(
            std::fs::read_to_string(target.join("race-link")).expect("read copied link"),
            "good\n",
            "ro copy must use the validated opened file, not refollow the swapped symlink path"
        );
        assert_eq!(
            std::fs::read_link(source.join("race-link")).expect("read swapped link"),
            outside
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn ensure_mount_ro_copy_rejects_directory_symlink() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        std::fs::create_dir_all(source.join("real-dir")).expect("create source dir");
        std::os::unix::fs::symlink(source.join("real-dir"), source.join("dir-link"))
            .expect("symlink dir");

        let err = ensure_mount(&target, &source, AttachmentMode::Ro)
            .await
            .expect_err("directory symlink should fail closed for ro copies");

        assert!(format!("{err:#}").contains("refuses directory symlink"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn ensure_mount_ro_copy_rejects_external_file_symlink() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        let outside = temp.path().join("outside-secret.txt");
        std::fs::create_dir_all(&source).expect("create source");
        std::fs::write(&outside, "outside\n").expect("write outside file");
        std::os::unix::fs::symlink(&outside, source.join("secret-link"))
            .expect("symlink outside file");

        let err = ensure_mount(&target, &source, AttachmentMode::Ro)
            .await
            .expect_err("external file symlink should fail closed for ro copies");

        assert!(format!("{err:#}").contains("outside source root"));
        assert!(!target.join("secret-link").exists());
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
