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

fn safe_archive_dest(out_dir: &Path, raw_path: &Path, label: &str) -> Result<PathBuf> {
    Ok(out_dir.join(normalize_archive_entry_path(raw_path, label)?))
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

fn ensure_no_symlink_ancestors(out_dir: &Path, dest: &Path) -> Result<()> {
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
                reject_existing_symlink(&current)?;
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

fn prepare_archive_entry_parent(out_dir: &Path, dest: &Path) -> Result<()> {
    ensure_no_symlink_ancestors(out_dir, dest)?;
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    ensure_no_symlink_ancestors(out_dir, dest)?;
    Ok(())
}

fn create_archive_dir(out_dir: &Path, dest: &Path) -> Result<()> {
    prepare_archive_entry_parent(out_dir, dest)?;
    reject_existing_symlink(dest)?;
    std::fs::create_dir_all(dest).with_context(|| format!("create {}", dest.display()))?;
    reject_existing_symlink(dest)?;
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

fn create_archive_symlink(out_dir: &Path, dest: &Path, target: &Path) -> Result<()> {
    prepare_archive_entry_parent(out_dir, dest)?;
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
        return Ok(());
    }
    #[cfg(not(unix))]
    {
        let _ = (dest, target);
        anyhow::bail!("archive symlink entries are not supported on this platform");
    }
}

fn create_archive_file<R: Read>(
    out_dir: &Path,
    dest: &Path,
    reader: &mut R,
    mode: Option<u32>,
) -> Result<()> {
    prepare_archive_entry_parent(out_dir, dest)?;
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

pub(crate) fn extract_zip_to_dir(zip_path: &Path, out_dir: &Path) -> Result<()> {
    let file =
        std::fs::File::open(zip_path).with_context(|| format!("open {}", zip_path.display()))?;
    let mut archive = zip::ZipArchive::new(file).context("parsing zip")?;
    for i in 0..archive.len() {
        let mut f = archive.by_index(i).context("zip entry")?;
        let entry_name = f.name().to_string();
        let dest = safe_archive_dest(out_dir, Path::new(&entry_name), "zip entry path")?;
        let mode = f.unix_mode();
        let file_type = mode.unwrap_or(0) & 0o170000;
        if f.is_dir() || file_type == 0o040000 {
            create_archive_dir(out_dir, &dest)?;
            continue;
        }

        if file_type == 0o120000 {
            let mut target = String::new();
            f.read_to_string(&mut target)
                .context("read zip symlink target")?;
            create_archive_symlink(out_dir, &dest, Path::new(&target))?;
            continue;
        }

        if file_type != 0 && file_type != 0o100000 {
            anyhow::bail!("unsupported zip entry type for {}", entry_name);
        }
        create_archive_file(out_dir, &dest, &mut f, mode)?;
    }
    Ok(())
}

pub(crate) fn extract_tar_gz_to_dir(tar_gz_path: &Path, out_dir: &Path) -> Result<()> {
    let tar_gz = std::fs::File::open(tar_gz_path)
        .with_context(|| format!("open {}", tar_gz_path.display()))?;
    let dec = flate2::read::GzDecoder::new(tar_gz);
    extract_tar_stream_to_dir(dec, out_dir, "tar.gz")
}

pub(crate) fn extract_tar_bz2_to_dir(tar_bz2_path: &Path, out_dir: &Path) -> Result<()> {
    let tar_bz2 = std::fs::File::open(tar_bz2_path)
        .with_context(|| format!("open {}", tar_bz2_path.display()))?;
    let dec = bzip2::read::BzDecoder::new(tar_bz2);
    extract_tar_stream_to_dir(dec, out_dir, "tar.bz2")
}

fn extract_tar_stream_to_dir<R: Read>(reader: R, out_dir: &Path, label: &str) -> Result<()> {
    let mut archive = tar::Archive::new(reader);
    for entry in archive
        .entries()
        .with_context(|| format!("read {label} entries"))?
    {
        let mut entry = entry.with_context(|| format!("read {label} entry"))?;
        let raw_path = entry
            .path()
            .with_context(|| format!("read {label} entry path"))?
            .into_owned();
        let dest = safe_archive_dest(out_dir, &raw_path, "tar entry path")?;
        let entry_type = entry.header().entry_type();

        if entry_type.is_dir() {
            create_archive_dir(out_dir, &dest)?;
            continue;
        }
        if entry_type.is_file() {
            let mode = entry.header().mode().ok();
            create_archive_file(out_dir, &dest, &mut entry, mode)?;
            continue;
        }
        if entry_type.is_symlink() {
            let target = entry
                .link_name()
                .with_context(|| format!("read {label} symlink target"))?
                .ok_or_else(|| {
                    anyhow::anyhow!("tar symlink missing target: {}", raw_path.display())
                })?
                .into_owned();
            create_archive_symlink(out_dir, &dest, &target)?;
            continue;
        }
        if entry_type.is_hard_link() {
            anyhow::bail!(
                "archive hardlinks are not supported: {}",
                raw_path.display()
            );
        }

        match entry_type.as_byte() {
            b'g' | b'x' | b'L' | b'K' => {}
            kind => anyhow::bail!(
                "unsupported tar entry type {} for {}",
                kind,
                raw_path.display()
            ),
        }
    }
    Ok(())
}

#[cfg(test)]
mod archive_extraction_tests {
    use super::*;
    use std::io::Write;

    fn write_tar_gz(
        path: &Path,
        write_entries: impl FnOnce(
            &mut tar::Builder<flate2::write::GzEncoder<std::fs::File>>,
        ) -> Result<()>,
    ) -> Result<()> {
        let file = std::fs::File::create(path)?;
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        write_entries(&mut builder)?;
        let encoder = builder.into_inner()?;
        encoder.finish()?;
        Ok(())
    }

    #[test]
    fn archive_extraction_zip_rejects_parent_traversal() {
        let temp = tempfile::tempdir().expect("tempdir");
        let zip_path = temp.path().join("traversal.zip");
        let file = std::fs::File::create(&zip_path).expect("create zip");
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default().unix_permissions(0o644);
        zip.start_file("../escape.txt", options)
            .expect("start zip entry");
        zip.write_all(b"escape").expect("write zip entry");
        zip.finish().expect("finish zip");

        let out_dir = temp.path().join("out");
        let err = extract_zip_to_dir(&zip_path, &out_dir).expect_err("zip traversal should fail");
        assert!(
            err.to_string().contains("parent directory"),
            "unexpected error: {err:#}"
        );
        assert!(!temp.path().join("escape.txt").exists());
    }

    #[test]
    fn archive_extraction_tar_gz_rejects_parent_traversal() {
        let temp = tempfile::tempdir().expect("tempdir");
        let tar_path = temp.path().join("traversal.tar.gz");
        write_tar_gz(&tar_path, |builder| {
            let data = b"escape";
            let mut header = tar::Header::new_gnu();
            header.set_mode(0o644);
            header.set_size(data.len() as u64);
            header.as_gnu_mut().expect("gnu header").name[..13].copy_from_slice(b"../escape.txt");
            header.set_cksum();
            builder.append(&header, &data[..])?;
            Ok(())
        })
        .expect("write tar.gz");

        let out_dir = temp.path().join("out");
        let err =
            extract_tar_gz_to_dir(&tar_path, &out_dir).expect_err("tar traversal should fail");
        assert!(
            err.to_string().contains("parent directory"),
            "unexpected error: {err:#}"
        );
        assert!(!temp.path().join("escape.txt").exists());
    }

    #[test]
    fn archive_extraction_tar_gz_rejects_symlink_escape() {
        let temp = tempfile::tempdir().expect("tempdir");
        let tar_path = temp.path().join("symlink-escape.tar.gz");
        write_tar_gz(&tar_path, |builder| {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_mode(0o777);
            header.set_size(0);
            header.set_path("bin/ctx")?;
            header.set_link_name("../../outside")?;
            header.set_cksum();
            builder.append(&header, std::io::empty())?;
            Ok(())
        })
        .expect("write tar.gz");

        let out_dir = temp.path().join("out");
        let err =
            extract_tar_gz_to_dir(&tar_path, &out_dir).expect_err("symlink escape should fail");
        assert!(
            err.to_string().contains("escapes extraction root"),
            "unexpected error: {err:#}"
        );
        assert!(!temp.path().join("outside").exists());
    }

    #[test]
    fn archive_extraction_zip_rejects_symlink_escape() {
        let temp = tempfile::tempdir().expect("tempdir");
        let zip_path = temp.path().join("symlink-escape.zip");
        let file = std::fs::File::create(&zip_path).expect("create zip");
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default().unix_permissions(0o777);
        zip.add_symlink("bin/ctx", "../../outside", options)
            .expect("write zip symlink");
        zip.finish().expect("finish zip");

        let out_dir = temp.path().join("out");
        let err = extract_zip_to_dir(&zip_path, &out_dir).expect_err("symlink escape should fail");
        assert!(
            err.to_string().contains("escapes extraction root"),
            "unexpected error: {err:#}"
        );
        assert!(!temp.path().join("outside").exists());
    }

    #[cfg(unix)]
    #[test]
    fn archive_extraction_tar_gz_allows_in_root_symlink() {
        let temp = tempfile::tempdir().expect("tempdir");
        let tar_path = temp.path().join("safe-symlink.tar.gz");
        write_tar_gz(&tar_path, |builder| {
            let mut dir_header = tar::Header::new_gnu();
            dir_header.set_entry_type(tar::EntryType::Directory);
            dir_header.set_mode(0o755);
            dir_header.set_size(0);
            dir_header.set_cksum();
            builder.append_data(&mut dir_header, "bin", std::io::empty())?;

            let mut symlink_header = tar::Header::new_gnu();
            symlink_header.set_entry_type(tar::EntryType::Symlink);
            symlink_header.set_mode(0o777);
            symlink_header.set_size(0);
            symlink_header.set_path("bin/npm")?;
            symlink_header.set_link_name("../lib/node_modules/npm/bin/npm-cli.js")?;
            symlink_header.set_cksum();
            builder.append(&symlink_header, std::io::empty())?;
            Ok(())
        })
        .expect("write tar.gz");

        let out_dir = temp.path().join("out");
        extract_tar_gz_to_dir(&tar_path, &out_dir).expect("extract safe symlink");
        let target = std::fs::read_link(out_dir.join("bin/npm")).expect("read symlink");
        assert_eq!(target, Path::new("../lib/node_modules/npm/bin/npm-cli.js"));
    }
}
