use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::Digest;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ReleaseArtifact {
    pub url_path: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ReleasePlatform {
    #[serde(default)]
    pub appimage: Option<ReleaseArtifact>,
    #[serde(default)]
    pub deb: Option<ReleaseArtifact>,
    #[serde(default)]
    pub daemon: Option<ReleaseArtifact>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ReleaseManifest {
    pub channel: String,
    pub latest_version: String,
    pub published_at: String,
    pub platforms: HashMap<String, ReleasePlatform>,
}

pub fn default_download_base_url() -> String {
    std::env::var("CONTEXT_DOWNLOAD_BASE_URL")
        .unwrap_or_else(|_| "https://api.context.rs/functions/v1".to_string())
}

pub fn platform_key() -> Option<&'static str> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    match (os, arch) {
        ("linux", "x86_64") => Some("linux-x64"),
        ("linux", "aarch64") => Some("linux-arm64"),
        ("macos", "x86_64") => Some("macos-x64"),
        ("macos", "aarch64") => Some("macos-arm64"),
        _ => None,
    }
}

pub fn normalize_version_str(s: &str) -> Option<Version> {
    let trimmed = s.trim().trim_start_matches('v');
    Version::parse(trimmed).ok()
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

    let current_version =
        normalize_version_str(env!("CARGO_PKG_VERSION")).context("parsing current version")?;

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
    let tmp_dir = std::env::temp_dir().join("context-self-update");
    tokio::fs::create_dir_all(&tmp_dir).await.ok();
    let tmp_path = tmp_dir.join("context.new");

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

pub fn appimage_path_env() -> Option<PathBuf> {
    std::env::var("CONTEXT_APPIMAGE_PATH")
        .ok()
        .map(PathBuf::from)
}

pub fn join_url(base_url: &str, url_path: &str) -> String {
    format!("{}{}", base_url.trim_end_matches('/'), url_path)
}

pub async fn download_and_verify(url: &str, expected_sha256: &str, dest: &Path) -> Result<()> {
    download_to_path(url, dest).await?;
    let got = sha256_hex_file(dest).await?;
    if !got.eq_ignore_ascii_case(expected_sha256) {
        anyhow::bail!(
            "checksum mismatch: expected {}, got {}",
            expected_sha256,
            got
        );
    }
    Ok(())
}

pub async fn atomic_replace_file(target: &Path, new_file: &Path) -> Result<()> {
    let backup = target.with_extension("bak");
    if backup.exists() {
        tokio::fs::remove_file(&backup).await.ok();
    }
    tokio::fs::rename(target, &backup)
        .await
        .with_context(|| format!("moving target to backup: {}", backup.display()))?;
    if let Err(e) = tokio::fs::rename(new_file, target).await {
        let _ = tokio::fs::rename(&backup, target).await;
        return Err(e).with_context(|| format!("moving new file into place: {}", target.display()));
    }
    Ok(())
}
