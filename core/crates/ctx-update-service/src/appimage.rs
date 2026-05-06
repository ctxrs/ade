use super::fs_ops::{
    download_to_path, next_update_temp_sequence, now_ms, sha256_hex_file, write_json_atomically,
};
use super::*;

pub fn updates_dir(data_root: &Path) -> PathBuf {
    data_root.join("updates")
}

pub fn appimage_candidate_path(data_root: &Path) -> PathBuf {
    updates_dir(data_root).join("ctx.AppImage.verified")
}

pub fn appimage_candidate_meta_path(data_root: &Path) -> PathBuf {
    updates_dir(data_root).join("ctx.AppImage.verified.json")
}

pub fn appimage_candidate_partial_path(data_root: &Path) -> PathBuf {
    updates_dir(data_root).join("ctx.AppImage.partial")
}

pub fn appimage_path_env() -> Option<PathBuf> {
    std::env::var("CTX_APPIMAGE_PATH").ok().map(PathBuf::from)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifiedAppImageCandidateMeta {
    pub schema_version: u32,
    pub candidate_path: PathBuf,
    pub target_path: PathBuf,
    pub channel: String,
    pub platform: String,
    pub target_version: String,
    pub current_version: String,
    pub artifact_url: String,
    pub artifact_url_path: String,
    pub manifest_url: String,
    pub base_url: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub verified_at_ms: u64,
}

impl VerifiedAppImageCandidateMeta {
    pub const SCHEMA_VERSION: u32 = 1;
}

pub async fn download_and_verify(url: &str, expected_sha256: &str, dest: &Path) -> Result<()> {
    download_to_path(url, dest).await?;
    let got = sha256_hex_file(dest).await?;
    if !got.eq_ignore_ascii_case(expected_sha256) {
        let _ = tokio::fs::remove_file(dest).await;
        anyhow::bail!("checksum mismatch: expected {expected_sha256}, got {got}");
    }
    Ok(())
}

pub struct AppImageCandidateRequest<'a> {
    pub data_root: &'a Path,
    pub target_path: &'a Path,
    pub channel: &'a str,
    pub platform: &'a str,
    pub target_version: &'a str,
    pub current_version: &'a str,
    pub artifact_url: &'a str,
    pub artifact_url_path: &'a str,
    pub manifest_url: &'a str,
    pub base_url: &'a str,
    pub sha256: &'a str,
}

pub async fn download_verified_appimage_candidate(
    req: AppImageCandidateRequest<'_>,
) -> Result<VerifiedAppImageCandidateMeta> {
    if !is_sha256_hex(req.sha256) {
        anyhow::bail!("AppImage artifact sha256 is missing or invalid");
    }
    let partial = appimage_candidate_partial_path(req.data_root);
    let candidate = appimage_candidate_path(req.data_root);
    let meta_path = appimage_candidate_meta_path(req.data_root);
    clear_appimage_candidate(req.data_root).await;

    download_and_verify(req.artifact_url, req.sha256, &partial).await?;
    let size_bytes = tokio::fs::metadata(&partial)
        .await
        .with_context(|| format!("reading {}", partial.display()))?
        .len();
    let meta = VerifiedAppImageCandidateMeta {
        schema_version: VerifiedAppImageCandidateMeta::SCHEMA_VERSION,
        candidate_path: candidate.clone(),
        target_path: req.target_path.to_path_buf(),
        channel: req.channel.to_string(),
        platform: req.platform.to_string(),
        target_version: req.target_version.to_string(),
        current_version: req.current_version.to_string(),
        artifact_url: req.artifact_url.to_string(),
        artifact_url_path: req.artifact_url_path.to_string(),
        manifest_url: req.manifest_url.to_string(),
        base_url: req.base_url.to_string(),
        sha256: req.sha256.to_string(),
        size_bytes,
        verified_at_ms: now_ms(),
    };
    tokio::fs::rename(&partial, &candidate)
        .await
        .with_context(|| {
            format!(
                "committing verified AppImage candidate {}",
                candidate.display()
            )
        })?;
    write_json_atomically(&meta_path, &meta).await?;
    Ok(meta)
}

pub async fn read_verified_appimage_candidate_meta(
    data_root: &Path,
) -> Result<VerifiedAppImageCandidateMeta> {
    let meta_path = appimage_candidate_meta_path(data_root);
    let raw = tokio::fs::read_to_string(&meta_path)
        .await
        .with_context(|| format!("reading {}", meta_path.display()))?;
    serde_json::from_str(&raw).with_context(|| format!("parsing {}", meta_path.display()))
}

fn path_is_inside(path: &Path, root: &Path) -> bool {
    !path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
        && path.starts_with(root)
}

pub async fn validate_verified_appimage_candidate(
    data_root: &Path,
    target_path: &Path,
    expected_channel: &str,
    expected_platform: &str,
    expected_base_url: &str,
    current_version: &str,
) -> Result<(PathBuf, VerifiedAppImageCandidateMeta)> {
    let updates = updates_dir(data_root);
    let meta = read_verified_appimage_candidate_meta(data_root).await?;
    if meta.schema_version != VerifiedAppImageCandidateMeta::SCHEMA_VERSION {
        anyhow::bail!("unsupported AppImage update metadata schema");
    }
    if meta.target_path != target_path {
        anyhow::bail!("downloaded AppImage update targets a different install path");
    }
    if meta.candidate_path != appimage_candidate_path(data_root) {
        anyhow::bail!(
            "downloaded AppImage candidate path does not match the verified candidate path"
        );
    }
    if !path_is_inside(&meta.candidate_path, &updates) {
        anyhow::bail!("downloaded AppImage candidate path is outside the updates directory");
    }
    if meta.channel != expected_channel {
        anyhow::bail!("downloaded AppImage update was staged for a different channel");
    }
    if meta.platform != expected_platform {
        anyhow::bail!("downloaded AppImage update was staged for a different platform");
    }
    if meta.base_url != expected_base_url {
        anyhow::bail!("downloaded AppImage update was staged from a different release base URL");
    }
    if !is_sha256_hex(&meta.sha256) {
        anyhow::bail!("downloaded AppImage metadata has invalid sha256");
    }
    let current = normalize_version_str(current_version).with_context(|| {
        format!("running AppImage version is not semver-compatible: {current_version}")
    })?;
    let target = normalize_version_str(&meta.target_version).with_context(|| {
        format!(
            "downloaded AppImage update version is not semver-compatible: {}",
            meta.target_version
        )
    })?;
    if current >= target {
        anyhow::bail!("downloaded AppImage update is not newer than the running version");
    }
    let symlink_md = tokio::fs::symlink_metadata(&meta.candidate_path)
        .await
        .with_context(|| format!("reading {}", meta.candidate_path.display()))?;
    if !symlink_md.file_type().is_file() {
        anyhow::bail!("downloaded AppImage candidate is not a regular file");
    }
    let md = tokio::fs::metadata(&meta.candidate_path)
        .await
        .with_context(|| format!("reading {}", meta.candidate_path.display()))?;
    if md.len() != meta.size_bytes {
        anyhow::bail!("downloaded AppImage candidate size changed after verification");
    }
    let got = sha256_hex_file(&meta.candidate_path).await?;
    if !got.eq_ignore_ascii_case(&meta.sha256) {
        anyhow::bail!(
            "downloaded AppImage candidate checksum mismatch: expected {}, got {}",
            meta.sha256,
            got
        );
    }
    Ok((meta.candidate_path.clone(), meta))
}

pub async fn clear_appimage_candidate(data_root: &Path) {
    let _ = tokio::fs::remove_file(appimage_candidate_path(data_root)).await;
    let _ = tokio::fs::remove_file(appimage_candidate_meta_path(data_root)).await;
    let _ = tokio::fs::remove_file(appimage_candidate_partial_path(data_root)).await;
}

fn is_sha256_hex(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.len() == 64 && trimmed.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub async fn atomic_replace_file(target: &Path, new_file: &Path) -> Result<()> {
    let Some(parent) = target.parent() else {
        anyhow::bail!("target path has no parent: {}", target.display());
    };
    let sequence = next_update_temp_sequence();
    let tmp = parent.join(format!(
        ".ctx-appimage-update-{}-{sequence}",
        std::process::id()
    ));
    tokio::fs::copy(new_file, &tmp)
        .await
        .with_context(|| format!("copying update into target directory: {}", tmp.display()))?;
    if let Ok(perms) = tokio::fs::metadata(new_file)
        .await
        .map(|md| md.permissions())
    {
        let _ = tokio::fs::set_permissions(&tmp, perms).await;
    }
    let file = tokio::fs::OpenOptions::new()
        .read(true)
        .open(&tmp)
        .await
        .with_context(|| format!("opening staged replacement {}", tmp.display()))?;
    file.sync_all()
        .await
        .with_context(|| format!("syncing staged replacement {}", tmp.display()))?;
    if let Err(e) = tokio::fs::rename(&tmp, target).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(e).with_context(|| format!("moving new file into place: {}", target.display()));
    }
    Ok(())
}

pub(super) fn in_place_update_capability_with_appimage_path(
    manifest: &ReleaseManifest,
    platform_key: Option<&str>,
    platform_supported: bool,
    appimage_path_set: bool,
) -> (bool, Option<String>) {
    let Some(platform_key) = platform_key else {
        return (
            false,
            Some("unsupported platform for in-place updates".to_string()),
        );
    };
    if !platform_supported {
        return (
            false,
            Some("no compatible desktop artifact for this platform".to_string()),
        );
    }
    if !platform_key.starts_with("linux-") {
        return (
            false,
            Some("in-place updates are only supported for Linux AppImage installs".to_string()),
        );
    }
    let Some(entry) = manifest.platforms.get(platform_key) else {
        return (false, Some("manifest missing platform entry".to_string()));
    };
    if entry.appimage.is_none() {
        return (
            false,
            Some("manifest missing appimage artifact for this platform".to_string()),
        );
    }
    if !appimage_path_set {
        return (
            false,
            Some("current install cannot apply AppImage in place".to_string()),
        );
    }
    (true, None)
}
