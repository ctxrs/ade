use super::*;

fn managed_artifact_extension(uri: &str) -> &'static str {
    let path = Url::parse(uri)
        .ok()
        .map(|parsed| parsed.path().to_string())
        .unwrap_or_else(|| uri.to_string());
    let path_lc = path.to_ascii_lowercase();
    if path_lc.ends_with(".tar.gz") {
        "tar.gz"
    } else if path_lc.ends_with(".tgz") {
        "tgz"
    } else if path_lc.ends_with(".tar") {
        "tar"
    } else {
        "zip"
    }
}

pub(super) fn managed_podman_archive_path(
    data_root: &Path,
    source: &bundled_assets::ManagedRuntimeSource,
) -> PathBuf {
    let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
    let ext = managed_artifact_extension(&source.uri);
    data_root
        .join("managed")
        .join("downloads")
        .join("podman")
        .join(os)
        .join(arch)
        .join(format!(
            "sha256-{}.{}",
            source.sha256.trim().to_ascii_lowercase(),
            ext
        ))
}

fn extract_zip_to_dir(zip_path: &Path, out_dir: &Path) -> Result<()> {
    let file =
        std::fs::File::open(zip_path).with_context(|| format!("open {}", zip_path.display()))?;
    let mut archive = zip::ZipArchive::new(file).context("parsing zip archive")?;
    for idx in 0..archive.len() {
        let mut entry = archive.by_index(idx).context("zip entry")?;
        if entry.is_dir() {
            continue;
        }
        let Some(enclosed) = entry.enclosed_name().map(|p| p.to_path_buf()) else {
            continue;
        };
        let dest = out_dir.join(enclosed);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
        }
        let mut out =
            std::fs::File::create(&dest).with_context(|| format!("create {}", dest.display()))?;
        std::io::copy(&mut entry, &mut out).context("extract zip entry")?;
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            let _ = std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(mode));
        }
    }
    Ok(())
}

pub(crate) fn extract_archive_to_dir(
    archive_path: &Path,
    source_uri: &str,
    out_dir: &Path,
) -> Result<()> {
    let kind = managed_artifact_extension(source_uri);
    match kind {
        "zip" => extract_zip_to_dir(archive_path, out_dir),
        "tar.gz" | "tgz" => {
            let archive_file = std::fs::File::open(archive_path)
                .with_context(|| format!("open {}", archive_path.display()))?;
            let decoder = flate2::read::GzDecoder::new(archive_file);
            let mut archive = tar::Archive::new(decoder);
            archive.unpack(out_dir).context("extract tar.gz archive")
        }
        "tar" => {
            let archive_file = std::fs::File::open(archive_path)
                .with_context(|| format!("open {}", archive_path.display()))?;
            let mut archive = tar::Archive::new(archive_file);
            archive.unpack(out_dir).context("extract tar archive")
        }
        _ => anyhow::bail!("unsupported podman archive type for {source_uri}"),
    }
}

pub(crate) fn resolve_single_extracted_root(extract_dir: &Path) -> Result<PathBuf> {
    let mut dirs = Vec::new();
    let mut has_files = false;
    for entry in std::fs::read_dir(extract_dir)
        .with_context(|| format!("read_dir {}", extract_dir.display()))?
    {
        let entry = entry.with_context(|| format!("read_dir entry {}", extract_dir.display()))?;
        let path = entry.path();
        if path.is_dir() {
            dirs.push(path);
        } else {
            has_files = true;
        }
    }
    if has_files || dirs.len() != 1 {
        return Ok(extract_dir.to_path_buf());
    }
    Ok(dirs.remove(0))
}
