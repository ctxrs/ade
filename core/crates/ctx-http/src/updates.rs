use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::path::Component;
use std::sync::atomic::{AtomicU64, Ordering};

static UPDATE_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ReleaseArtifact {
    pub url_path: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ReleasePlatform {
    #[serde(default)]
    pub desktop: Option<ReleaseArtifact>,
    #[serde(default)]
    pub appimage: Option<ReleaseArtifact>,
    #[serde(default)]
    pub deb: Option<ReleaseArtifact>,
    #[serde(default)]
    pub dmg: Option<ReleaseArtifact>,
    #[serde(default)]
    pub msi: Option<ReleaseArtifact>,
    #[serde(default)]
    pub nsis: Option<ReleaseArtifact>,
    #[serde(default)]
    pub exe: Option<ReleaseArtifact>,
    #[serde(default)]
    pub zip: Option<ReleaseArtifact>,
    #[serde(default)]
    pub daemon: Option<ReleaseArtifact>,
}

impl ReleasePlatform {
    pub fn preferred_desktop_artifact<'a>(
        &'a self,
        platform_key: &str,
    ) -> Option<&'a ReleaseArtifact> {
        match platform_key {
            "linux-x64" | "linux-arm64" => self.desktop.as_ref().or(self.appimage.as_ref()),
            "macos-x64" | "macos-arm64" => self
                .desktop
                .as_ref()
                .or(self.dmg.as_ref())
                .or(self.zip.as_ref()),
            "windows-x64" => self
                .desktop
                .as_ref()
                .or(self.nsis.as_ref())
                .or(self.msi.as_ref())
                .or(self.exe.as_ref())
                .or(self.zip.as_ref()),
            _ => self.desktop.as_ref(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ReleaseManifest {
    pub channel: String,
    pub latest_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_supported_version: Option<String>,
    pub published_at: String,
    pub platforms: HashMap<String, ReleasePlatform>,
}

pub fn default_download_base_url() -> String {
    std::env::var("CTX_DOWNLOAD_BASE_URL")
        .unwrap_or_else(|_| "https://api.ctx.rs/functions/v1".to_string())
}

pub fn platform_key() -> Option<&'static str> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    match (os, arch) {
        ("linux", "x86_64") => Some("linux-x64"),
        ("linux", "aarch64") => Some("linux-arm64"),
        ("macos", "x86_64") => Some("macos-x64"),
        ("macos", "aarch64") => Some("macos-arm64"),
        ("windows", "x86_64") => Some("windows-x64"),
        _ => None,
    }
}

pub fn normalize_version_str(s: &str) -> Option<Version> {
    let trimmed = s.trim().trim_start_matches('v');
    Version::parse(trimmed).ok()
}

pub fn in_place_update_capability(
    manifest: &ReleaseManifest,
    platform_key: Option<&str>,
    platform_supported: bool,
) -> (bool, Option<String>) {
    in_place_update_capability_with_appimage_path(
        manifest,
        platform_key,
        platform_supported,
        appimage_path_env().is_some(),
    )
}

pub fn platform_supported(manifest: &ReleaseManifest, platform_key: Option<&str>) -> bool {
    let Some(platform_key) = platform_key else {
        return false;
    };
    let Some(entry) = manifest.platforms.get(platform_key) else {
        return false;
    };
    entry.preferred_desktop_artifact(platform_key).is_some()
}

pub fn is_update_available(
    current_version: &str,
    latest_version: &str,
    platform_supported: bool,
) -> bool {
    if !platform_supported {
        return false;
    }
    match (
        normalize_version_str(current_version),
        normalize_version_str(latest_version),
    ) {
        (Some(cur), Some(lat)) => lat > cur,
        _ => false,
    }
}

pub async fn fetch_latest_manifest(base_url: &str, channel: &str) -> Result<ReleaseManifest> {
    fetch_latest_manifest_with_params(base_url, channel, None).await
}

pub async fn fetch_latest_manifest_with_params(
    base_url: &str,
    channel: &str,
    query: Option<&[(&str, String)]>,
) -> Result<ReleaseManifest> {
    let mut url = format!(
        "{}/releases/{}/latest.json",
        base_url.trim_end_matches('/'),
        channel
    );
    if let Some(q) = query {
        let qs = q
            .iter()
            .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&");
        if !qs.is_empty() {
            url.push('?');
            url.push_str(&qs);
        }
    }
    let txt = reqwest::get(&url)
        .await
        .with_context(|| format!("fetching release manifest: {url}"))?
        .error_for_status()
        .with_context(|| format!("release manifest http error: {url}"))?
        .text()
        .await
        .context("reading release manifest body")?;
    serde_json::from_str(&txt).context("parsing release manifest JSON")
}

pub async fn download_to_path(url: &str, dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }
    let resp = reqwest::get(url)
        .await
        .with_context(|| format!("downloading: {url}"))?
        .error_for_status()
        .with_context(|| format!("download http error: {url}"))?;
    let bytes = resp.bytes().await.context("reading download body")?;
    tokio::fs::write(dest, &bytes)
        .await
        .with_context(|| format!("writing {}", dest.display()))?;
    Ok(())
}

pub async fn sha256_hex_file(path: &Path) -> Result<String> {
    let bytes = tokio::fs::read(path)
        .await
        .with_context(|| format!("reading {}", path.display()))?;
    let mut hasher = sha2::Sha256::new();
    hasher.update(&bytes);
    Ok(hex::encode(hasher.finalize()))
}

pub async fn atomic_replace_exe(current_exe: &Path, new_file: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = tokio::fs::metadata(new_file).await?.permissions();
        perms.set_mode(0o755);
        tokio::fs::set_permissions(new_file, perms).await.ok();
    }

    let backup = current_exe.with_extension("bak");
    if backup.exists() {
        tokio::fs::remove_file(&backup).await.ok();
    }

    // Best-effort backup.
    tokio::fs::rename(current_exe, &backup)
        .await
        .with_context(|| format!("moving current exe to backup: {}", backup.display()))?;
    if let Err(e) = tokio::fs::rename(new_file, current_exe).await {
        // Attempt rollback.
        let _ = tokio::fs::rename(&backup, current_exe).await;
        return Err(e)
            .with_context(|| format!("moving new exe into place: {}", current_exe.display()));
    }

    Ok(())
}

pub async fn self_update_daemon(
    channel: &str,
    base_url: &str,
    yes: bool,
    check_only: bool,
) -> Result<()> {
    let Some(platform) = platform_key() else {
        anyhow::bail!(
            "unsupported platform for self-update: {}/{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        );
    };

    let current_version = normalize_version_str(
        &crate::build_identity::current_build_identity()
            .context("loading build identity")?
            .exact_version,
    )
    .context("parsing current version")?;

    let manifest = fetch_latest_manifest(base_url, channel).await?;
    let latest_version = normalize_version_str(&manifest.latest_version)
        .with_context(|| format!("parsing latest_version: {}", manifest.latest_version))?;

    let update_available = latest_version > current_version;
    println!(
        "Current version: {}\nLatest version:  {}\nChannel:         {}\nUpdate:          {}",
        current_version,
        latest_version,
        channel,
        if update_available {
            "available"
        } else {
            "none"
        }
    );

    if !update_available || check_only {
        return Ok(());
    }

    let platform_entry = manifest
        .platforms
        .get(platform)
        .with_context(|| format!("manifest missing platform entry: {platform}"))?;
    let artifact = platform_entry
        .daemon
        .as_ref()
        .context("manifest missing daemon artifact for this platform")?;

    if !yes {
        if !atty::is(atty::Stream::Stdin) {
            anyhow::bail!("refusing to self-update non-interactively without --yes");
        }
        use std::io::Write;
        print!("Proceed to download and replace this binary? [y/N] ");
        std::io::stdout().flush().ok();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).ok();
        let ok = matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes");
        if !ok {
            println!("Aborted.");
            return Ok(());
        }
    }

    let download_url = format!("{}{}", base_url.trim_end_matches('/'), artifact.url_path);
    let tmp_dir = std::env::temp_dir().join("ctx-self-update");
    tokio::fs::create_dir_all(&tmp_dir).await.ok();
    let tmp_path = tmp_dir.join("ctx.new");

    println!("Downloading: {download_url}");
    download_to_path(&download_url, &tmp_path).await?;

    let got = sha256_hex_file(&tmp_path).await?;
    if !got.eq_ignore_ascii_case(&artifact.sha256) {
        anyhow::bail!(
            "checksum mismatch for downloaded binary: expected {}, got {}",
            artifact.sha256,
            got
        );
    }

    let current_exe = std::env::current_exe().context("resolving current executable path")?;
    atomic_replace_exe(&current_exe, &tmp_path).await?;

    println!(
        "Updated successfully. New binary is in place at {}",
        current_exe.display()
    );
    Ok(())
}

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

pub fn join_url(base_url: &str, url_path: &str) -> String {
    format!("{}{}", base_url.trim_end_matches('/'), url_path)
}

pub fn release_manifest_url(base_url: &str, channel: &str) -> String {
    format!(
        "{}/releases/{}/latest.json",
        base_url.trim_end_matches('/'),
        channel
    )
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

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

async fn write_json_atomically<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let Some(parent) = path.parent() else {
        anyhow::bail!("path has no parent: {}", path.display());
    };
    tokio::fs::create_dir_all(parent)
        .await
        .with_context(|| format!("creating {}", parent.display()))?;
    let sequence = UPDATE_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_extension(format!("tmp-{}-{sequence}", std::process::id()));
    let bytes = serde_json::to_vec_pretty(value).context("serializing JSON")?;
    tokio::fs::write(&tmp, bytes)
        .await
        .with_context(|| format!("writing {}", tmp.display()))?;
    tokio::fs::rename(&tmp, path)
        .await
        .with_context(|| format!("committing {} -> {}", tmp.display(), path.display()))
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
    let current_version = crate::build_identity::current_build_identity()
        .context("loading build identity")?
        .exact_version
        .clone();
    let current = normalize_version_str(&current_version).with_context(|| {
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
    let sequence = UPDATE_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
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

fn in_place_update_capability_with_appimage_path(
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn artifact(path: &str) -> ReleaseArtifact {
        ReleaseArtifact {
            url_path: path.to_string(),
            sha256: "sha".to_string(),
        }
    }

    fn release_platform_with_dmg() -> ReleasePlatform {
        ReleasePlatform {
            desktop: None,
            appimage: None,
            deb: None,
            dmg: Some(artifact("ctx.dmg")),
            msi: None,
            nsis: None,
            exe: None,
            zip: None,
            daemon: Some(artifact("ctx-daemon")),
        }
    }

    fn release_platform_daemon_only() -> ReleasePlatform {
        ReleasePlatform {
            desktop: None,
            appimage: None,
            deb: None,
            dmg: None,
            msi: None,
            nsis: None,
            exe: None,
            zip: None,
            daemon: Some(artifact("ctx-daemon")),
        }
    }

    fn release_platform_with_appimage() -> ReleasePlatform {
        ReleasePlatform {
            desktop: None,
            appimage: Some(artifact("ctx.AppImage")),
            deb: None,
            dmg: None,
            msi: None,
            nsis: None,
            exe: None,
            zip: None,
            daemon: Some(artifact("ctx-daemon")),
        }
    }

    fn manifest_with_platforms(platforms: HashMap<String, ReleasePlatform>) -> ReleaseManifest {
        ReleaseManifest {
            channel: "stable".to_string(),
            latest_version: "1.2.3".to_string(),
            min_supported_version: None,
            published_at: "2026-02-19T00:00:00Z".to_string(),
            platforms,
        }
    }

    #[test]
    fn preferred_desktop_artifact_follows_platform_order() {
        let platform = ReleasePlatform {
            desktop: Some(artifact("/desktop")),
            appimage: Some(artifact("/appimage")),
            deb: Some(artifact("/deb")),
            dmg: Some(artifact("/dmg")),
            msi: Some(artifact("/msi")),
            nsis: Some(artifact("/nsis")),
            exe: Some(artifact("/exe")),
            zip: Some(artifact("/zip")),
            daemon: Some(artifact("/daemon")),
        };

        let linux = platform
            .preferred_desktop_artifact("linux-x64")
            .expect("linux artifact");
        assert_eq!(linux.url_path, "/desktop");

        let mac = platform
            .preferred_desktop_artifact("macos-arm64")
            .expect("mac artifact");
        assert_eq!(mac.url_path, "/desktop");

        let windows = platform
            .preferred_desktop_artifact("windows-x64")
            .expect("windows artifact");
        assert_eq!(windows.url_path, "/desktop");
    }

    #[test]
    fn preferred_desktop_artifact_uses_fallbacks() {
        let linux = ReleasePlatform {
            desktop: None,
            appimage: Some(artifact("/appimage")),
            deb: Some(artifact("/deb")),
            dmg: None,
            msi: None,
            nsis: None,
            exe: None,
            zip: None,
            daemon: Some(artifact("/daemon")),
        };
        assert_eq!(
            linux
                .preferred_desktop_artifact("linux-arm64")
                .expect("linux fallback")
                .url_path,
            "/appimage"
        );

        let windows = ReleasePlatform {
            desktop: None,
            appimage: None,
            deb: None,
            dmg: None,
            msi: Some(artifact("/msi")),
            nsis: None,
            exe: None,
            zip: None,
            daemon: Some(artifact("/daemon")),
        };
        assert_eq!(
            windows
                .preferred_desktop_artifact("windows-x64")
                .expect("windows fallback")
                .url_path,
            "/msi"
        );
    }

    #[test]
    fn normalize_version_accepts_v_prefix() {
        assert_eq!(
            normalize_version_str("v1.2.3").expect("semver").to_string(),
            "1.2.3"
        );
    }

    #[test]
    fn platform_supported_false_when_platform_missing() {
        let manifest = manifest_with_platforms(HashMap::new());
        assert!(!platform_supported(&manifest, Some("macos-arm64")));
    }

    #[test]
    fn platform_supported_false_when_no_desktop_artifact() {
        let mut platforms = HashMap::new();
        platforms.insert("macos-arm64".to_string(), release_platform_daemon_only());
        let manifest = manifest_with_platforms(platforms);
        assert!(!platform_supported(&manifest, Some("macos-arm64")));
    }

    #[test]
    fn platform_supported_true_with_matching_desktop_artifact() {
        let mut platforms = HashMap::new();
        platforms.insert("macos-arm64".to_string(), release_platform_with_dmg());
        let manifest = manifest_with_platforms(platforms);
        assert!(platform_supported(&manifest, Some("macos-arm64")));
    }

    #[test]
    fn update_available_requires_supported_platform() {
        assert!(!is_update_available("1.0.0", "1.2.3", false));
        assert!(is_update_available("1.0.0", "1.2.3", true));
    }

    #[test]
    fn in_place_update_capability_requires_linux_appimage_and_runtime_support() {
        let mut platforms = HashMap::new();
        platforms.insert("linux-x64".to_string(), release_platform_with_appimage());
        let manifest = manifest_with_platforms(platforms);

        let (supported_without_runtime, reason_without_runtime) =
            in_place_update_capability_with_appimage_path(
                &manifest,
                Some("linux-x64"),
                true,
                false,
            );
        assert!(!supported_without_runtime);
        assert_eq!(
            reason_without_runtime.as_deref(),
            Some("current install cannot apply AppImage in place")
        );

        let (supported_with_runtime, reason_with_runtime) =
            in_place_update_capability_with_appimage_path(&manifest, Some("linux-x64"), true, true);
        assert!(supported_with_runtime);
        assert!(reason_with_runtime.is_none());
    }

    #[test]
    fn in_place_update_capability_is_false_when_platform_is_not_supported() {
        let mut platforms = HashMap::new();
        platforms.insert("linux-x64".to_string(), release_platform_with_appimage());
        let manifest = manifest_with_platforms(platforms);

        let (supported, reason) = in_place_update_capability_with_appimage_path(
            &manifest,
            Some("linux-x64"),
            false,
            true,
        );
        assert!(!supported);
        assert_eq!(
            reason.as_deref(),
            Some("no compatible desktop artifact for this platform")
        );
    }

    #[test]
    fn in_place_update_capability_is_false_for_non_linux_platforms() {
        let mut platforms = HashMap::new();
        platforms.insert("macos-arm64".to_string(), release_platform_with_dmg());
        let manifest = manifest_with_platforms(platforms);

        let (supported, reason) = in_place_update_capability_with_appimage_path(
            &manifest,
            Some("macos-arm64"),
            true,
            true,
        );
        assert!(!supported);
        assert_eq!(
            reason.as_deref(),
            Some("in-place updates are only supported for Linux AppImage installs")
        );
    }
}
