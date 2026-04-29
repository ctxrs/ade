use std::ffi::OsString;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

fn normalize_archive_entry_path(raw_path: &Path, label: &str) -> Result<PathBuf> {
    let raw_display = raw_path.display().to_string();
    if raw_display.contains('\\') {
        anyhow::bail!("{label} must not contain backslashes: {raw_display}");
    }

    let mut normalized = PathBuf::new();
    for component in raw_path.components() {
        match component {
            Component::Normal(segment) => normalized.push(segment),
            Component::CurDir => {}
            Component::ParentDir => {
                anyhow::bail!("{label} must not contain parent directory segments: {raw_display}");
            }
            Component::RootDir | Component::Prefix(_) => {
                anyhow::bail!("{label} must be relative: {raw_display}");
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        anyhow::bail!("{label} is empty");
    }
    Ok(normalized)
}

pub(super) fn safe_archive_dest(out_dir: &Path, raw_path: &Path, label: &str) -> Result<PathBuf> {
    Ok(out_dir.join(normalize_archive_entry_path(raw_path, label)?))
}

pub(super) fn ensure_archive_root(out_dir: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(out_dir).with_context(|| format!("create {}", out_dir.display()))?;
    std::fs::canonicalize(out_dir).with_context(|| format!("canonicalize {}", out_dir.display()))
}

fn ensure_canonical_path_inside_root(root: &Path, path: &Path, label: &str) -> Result<()> {
    let canonical =
        std::fs::canonicalize(path).with_context(|| format!("canonicalize {}", path.display()))?;
    if !canonical.starts_with(root) {
        anyhow::bail!(
            "{label} escaped extraction root: {} -> {}",
            path.display(),
            canonical.display()
        );
    }
    Ok(())
}

fn reject_existing_symlink(path: &Path) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            anyhow::bail!(
                "archive extraction refused to write through symlink: {}",
                path.display()
            )
        }
        Ok(_) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_context(|| format!("stat {}", path.display())),
    }
}

fn ensure_existing_ancestors_inside_root(root: &Path, out_dir: &Path, dest: &Path) -> Result<()> {
    let rel = dest
        .strip_prefix(out_dir)
        .with_context(|| format!("archive destination escaped root: {}", dest.display()))?;
    let Some(parent) = rel.parent() else {
        return Ok(());
    };

    let mut current = out_dir.to_path_buf();
    for component in parent.components() {
        match component {
            Component::Normal(segment) => {
                current.push(segment);
                match std::fs::symlink_metadata(&current) {
                    Ok(_) => ensure_canonical_path_inside_root(root, &current, "archive ancestor")?,
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => break,
                    Err(err) => {
                        return Err(err).with_context(|| format!("stat {}", current.display()))
                    }
                }
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                anyhow::bail!(
                    "archive destination has unsafe ancestor: {}",
                    dest.display()
                );
            }
        }
    }
    Ok(())
}

fn prepare_archive_entry_parent(root: &Path, out_dir: &Path, dest: &Path) -> Result<()> {
    ensure_existing_ancestors_inside_root(root, out_dir, dest)?;
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        ensure_canonical_path_inside_root(root, parent, "archive parent")?;
    }
    ensure_existing_ancestors_inside_root(root, out_dir, dest)?;
    Ok(())
}

pub(super) fn create_archive_dir(root: &Path, out_dir: &Path, dest: &Path) -> Result<()> {
    prepare_archive_entry_parent(root, out_dir, dest)?;
    reject_existing_symlink(dest)?;
    std::fs::create_dir_all(dest).with_context(|| format!("create {}", dest.display()))?;
    reject_existing_symlink(dest)?;
    ensure_canonical_path_inside_root(root, dest, "archive directory")?;
    Ok(())
}

fn validate_symlink_target(out_dir: &Path, dest: &Path, target: &Path) -> Result<()> {
    let target_display = target.display().to_string();
    if target_display.is_empty() {
        anyhow::bail!("archive symlink target is empty for {}", dest.display());
    }
    if target_display.contains('\\') {
        anyhow::bail!("archive symlink target must not contain backslashes: {target_display}");
    }

    let dest_rel = dest.strip_prefix(out_dir).with_context(|| {
        format!(
            "archive symlink destination escaped root: {}",
            dest.display()
        )
    })?;
    let mut stack: Vec<OsString> = dest_rel
        .parent()
        .map(|parent| {
            parent
                .components()
                .filter_map(|component| match component {
                    Component::Normal(segment) => Some(segment.to_os_string()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();

    for component in target.components() {
        match component {
            Component::Normal(segment) => stack.push(segment.to_os_string()),
            Component::CurDir => {}
            Component::ParentDir => {
                if stack.pop().is_none() {
                    anyhow::bail!(
                        "archive symlink target escapes extraction root: {} -> {}",
                        dest.display(),
                        target.display()
                    );
                }
            }
            Component::RootDir | Component::Prefix(_) => {
                anyhow::bail!(
                    "archive symlink target must be relative: {} -> {}",
                    dest.display(),
                    target.display()
                );
            }
        }
    }
    Ok(())
}

pub(super) fn create_archive_symlink(
    root: &Path,
    out_dir: &Path,
    dest: &Path,
    target: &Path,
) -> Result<()> {
    prepare_archive_entry_parent(root, out_dir, dest)?;
    reject_existing_symlink(dest)?;
    if std::fs::symlink_metadata(dest).is_ok() {
        anyhow::bail!(
            "archive extraction refused to replace existing path with symlink: {}",
            dest.display()
        );
    }
    validate_symlink_target(out_dir, dest, target)?;
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, dest).with_context(|| {
            format!("create symlink {} -> {}", dest.display(), target.display())
        })?;
        if std::fs::canonicalize(dest).is_ok() {
            ensure_canonical_path_inside_root(root, dest, "archive symlink target")?;
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = (dest, target);
        anyhow::bail!("archive symlink entries are not supported on this platform");
    }
}

pub(super) fn create_archive_file<R: Read>(
    root: &Path,
    out_dir: &Path,
    dest: &Path,
    reader: &mut R,
    mode: Option<u32>,
) -> Result<()> {
    prepare_archive_entry_parent(root, out_dir, dest)?;
    reject_existing_symlink(dest)?;
    let mut out =
        std::fs::File::create(dest).with_context(|| format!("create {}", dest.display()))?;
    std::io::copy(reader, &mut out).context("extract archive entry")?;
    #[cfg(unix)]
    if let Some(mode) = mode {
        std::fs::set_permissions(dest, std::fs::Permissions::from_mode(mode & 0o777))
            .with_context(|| format!("chmod {}", dest.display()))?;
    }
    Ok(())
}

pub(super) fn create_archive_hardlink(
    root: &Path,
    out_dir: &Path,
    dest: &Path,
    target: &Path,
) -> Result<()> {
    let target_dest = safe_archive_dest(out_dir, target, "tar hardlink target")?;
    ensure_canonical_path_inside_root(root, &target_dest, "tar hardlink target")?;
    let target_metadata = std::fs::symlink_metadata(&target_dest)
        .with_context(|| format!("stat tar hardlink target {}", target_dest.display()))?;
    if target_metadata.file_type().is_symlink() {
        anyhow::bail!(
            "archive hardlink target must not be a symlink: {}",
            target.display()
        );
    }
    if !target_metadata.file_type().is_file() {
        anyhow::bail!(
            "archive hardlink target must be a file: {}",
            target.display()
        );
    }

    prepare_archive_entry_parent(root, out_dir, dest)?;
    reject_existing_symlink(dest)?;
    if std::fs::symlink_metadata(dest).is_ok() {
        anyhow::bail!(
            "archive extraction refused to replace existing path with hardlink: {}",
            dest.display()
        );
    }
    std::fs::hard_link(&target_dest, dest).with_context(|| {
        format!(
            "create hardlink {} -> {}",
            dest.display(),
            target_dest.display()
        )
    })?;
    Ok(())
}
