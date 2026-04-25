use super::*;
use sha2::Digest;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ReleaseArtifact {
    pub(crate) url_path: String,
    pub(crate) sha256: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct ReleasePlatformEntry {
    #[serde(default)]
    pub(crate) daemon: Option<ReleaseArtifact>,
    #[serde(default)]
    pub(crate) appimage: Option<ReleaseArtifact>,
    #[serde(default)]
    pub(crate) desktop: Option<ReleaseArtifact>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ReleaseManifest {
    #[serde(default)]
    pub(crate) platforms: std::collections::HashMap<String, ReleasePlatformEntry>,
}

pub(crate) fn validate_remote_ctx_bin(value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        anyhow::bail!("remote_ctx_bin is required");
    }
    let valid_absolute = trimmed.starts_with('/');
    let valid_home_relative = trimmed == "~" || trimmed.starts_with("~/");
    if !valid_absolute && !valid_home_relative {
        anyhow::bail!(
            "remote_ctx_bin must be an absolute path or ~/ path (for example ~/.ctx/bin/ctx)"
        );
    }
    Ok(trimmed.to_string())
}

pub(crate) fn remote_ctx_bin_parent_dir(remote_ctx_bin: &str) -> Result<String> {
    let path = validate_remote_ctx_bin(remote_ctx_bin)?;
    if path == "~" {
        return Ok("~".to_string());
    }
    let Some((parent, _name)) = path.rsplit_once('/') else {
        anyhow::bail!("remote_ctx_bin must include a file name");
    };
    if parent.is_empty() {
        return Ok("/".to_string());
    }
    Ok(parent.to_string())
}

pub(crate) fn bootstrap_download_base_url() -> String {
    std::env::var("CTX_DOWNLOAD_BASE_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_DOWNLOAD_BASE_URL.to_string())
}

fn remote_daemon_platform_key_for_arch(arch: &str) -> Result<&'static str> {
    match arch {
        "x86_64" => Ok("linux-x64"),
        "aarch64" => Ok("linux-arm64"),
        other => anyhow::bail!("unsupported remote daemon arch for bootstrap: {other}"),
    }
}

fn normalize_sha256_hex(raw: &str) -> Result<String> {
    let normalized = raw.trim().to_ascii_lowercase();
    if normalized.len() != 64 || !normalized.chars().all(|ch| ch.is_ascii_hexdigit()) {
        anyhow::bail!("invalid sha256 digest format");
    }
    Ok(normalized)
}

fn join_url(base: &str, path_or_url: &str) -> String {
    let trimmed = path_or_url.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return trimmed.to_string();
    }
    format!("{}{}", base.trim_end_matches('/'), trimmed)
}

fn fetch_release_manifest_for_channel(channel: &str, base_url: &str) -> Result<ReleaseManifest> {
    let url = format!(
        "{}/releases/{}/latest.json",
        base_url.trim_end_matches('/'),
        channel
    );
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(REMOTE_DAEMON_DOWNLOAD_TIMEOUT_SECS))
        .build()
        .context("building release manifest http client")?;
    let resp = client
        .get(&url)
        .send()
        .with_context(|| format!("fetching release manifest: {url}"))?
        .error_for_status()
        .with_context(|| format!("release manifest http error: {url}"))?;
    resp.json::<ReleaseManifest>()
        .context("parsing release manifest JSON")
}

fn sha256_hex_file(path: &std::path::Path) -> Result<String> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let mut hasher = sha2::Sha256::new();
    hasher.update(&bytes);
    Ok(format!("{:x}", hasher.finalize()))
}

pub(crate) fn remote_bundle_dir_for_data_dir(remote_data_dir: &str) -> String {
    format!("{}/bundles", remote_data_dir.trim_end_matches('/'))
}

pub(crate) fn remote_bundle_backup_dir_for_data_dir(remote_data_dir: &str) -> String {
    format!(
        "{}.pre-update-backup",
        remote_bundle_dir_for_data_dir(remote_data_dir)
    )
}

fn managed_remote_artifact_download_path(
    app: &tauri::AppHandle,
    namespace: &str,
    channel: &str,
    platform_key: &str,
    sha256: &str,
    file_name: &str,
) -> Result<PathBuf> {
    let channel = channel.trim();
    if channel.is_empty() {
        anyhow::bail!("channel is required");
    }
    let data_dir = daemon_data_dir(app)?;
    Ok(data_dir
        .join("updates")
        .join(namespace)
        .join(channel)
        .join(platform_key)
        .join(format!("sha256-{sha256}"))
        .join(file_name))
}

fn managed_remote_daemon_download_path(
    app: &tauri::AppHandle,
    channel: &str,
    platform_key: &str,
    sha256: &str,
) -> Result<PathBuf> {
    managed_remote_artifact_download_path(
        app,
        "remote-daemon",
        channel,
        platform_key,
        sha256,
        "ctx",
    )
}

fn managed_remote_bundle_appimage_download_path(
    app: &tauri::AppHandle,
    channel: &str,
    platform_key: &str,
    sha256: &str,
) -> Result<PathBuf> {
    managed_remote_artifact_download_path(
        app,
        "remote-bundles",
        channel,
        platform_key,
        sha256,
        "ctx.AppImage",
    )
}

fn cache_release_artifact(
    final_path: &std::path::Path,
    artifact: &ReleaseArtifact,
    base_url: &str,
    error_label: &str,
    make_executable: bool,
) -> Result<PathBuf> {
    let expected_sha = normalize_sha256_hex(&artifact.sha256)?;
    if final_path.exists() {
        let digest = sha256_hex_file(final_path)
            .with_context(|| format!("computing sha256 for {}", final_path.display()))?;
        if digest.eq_ignore_ascii_case(&expected_sha) {
            return Ok(final_path.to_path_buf());
        }
        std::fs::remove_file(final_path).with_context(|| {
            format!(
                "removing corrupted cached {error_label} at {}",
                final_path.display()
            )
        })?;
    }
    let parent = final_path
        .parent()
        .ok_or_else(|| anyhow!("invalid download target path: {}", final_path.display()))?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("creating {error_label} cache dir {}", parent.display()))?;
    let file_name = final_path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| anyhow!("invalid artifact file name: {}", final_path.display()))?;
    let tmp_path = parent.join(format!("{file_name}.tmp-{}", std::process::id()));
    let artifact_url = join_url(base_url, &artifact.url_path);
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(REMOTE_DAEMON_DOWNLOAD_TIMEOUT_SECS))
        .build()
        .context("building managed artifact download client")?;
    let mut resp = client
        .get(&artifact_url)
        .send()
        .with_context(|| format!("downloading {error_label} artifact: {artifact_url}"))?
        .error_for_status()
        .with_context(|| format!("{error_label} artifact download http error: {artifact_url}"))?;
    {
        let mut file = std::fs::File::create(&tmp_path)
            .with_context(|| format!("creating {}", tmp_path.display()))?;
        std::io::copy(&mut resp, &mut file)
            .with_context(|| format!("writing {}", tmp_path.display()))?;
    }
    let digest = sha256_hex_file(&tmp_path)
        .with_context(|| format!("computing sha256 for {}", tmp_path.display()))?;
    if !digest.eq_ignore_ascii_case(&expected_sha) {
        std::fs::remove_file(&tmp_path).ok();
        anyhow::bail!(
            "{error_label} artifact checksum mismatch (expected {}, got {})",
            expected_sha,
            digest
        );
    }
    std::fs::rename(&tmp_path, final_path).with_context(|| {
        format!(
            "moving {error_label} artifact into cache ({} -> {})",
            tmp_path.display(),
            final_path.display()
        )
    })?;
    if make_executable {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(final_path)
                .with_context(|| format!("reading metadata for {}", final_path.display()))?
                .permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(final_path, perms)
                .with_context(|| format!("setting executable bit on {}", final_path.display()))?;
        }
    }
    Ok(final_path.to_path_buf())
}

pub(crate) fn release_bundle_artifact_for_platform(
    platform_entry: &ReleasePlatformEntry,
) -> Option<&ReleaseArtifact> {
    platform_entry
        .appimage
        .as_ref()
        .or(platform_entry.desktop.as_ref())
}

pub(crate) fn ensure_managed_remote_daemon_binary(
    app: &tauri::AppHandle,
    remote_arch: &str,
    channel: &str,
) -> Result<PathBuf> {
    let platform_key = remote_daemon_platform_key_for_arch(remote_arch)?;
    let base_url = bootstrap_download_base_url();
    let manifest = fetch_release_manifest_for_channel(channel, &base_url)?;
    let platform_entry = manifest
        .platforms
        .get(platform_key)
        .ok_or_else(|| anyhow!("release manifest missing platform entry: {platform_key}"))?;
    let daemon_artifact = platform_entry
        .daemon
        .as_ref()
        .ok_or_else(|| anyhow!("release manifest missing daemon artifact for {platform_key}"))?;
    let expected_sha = normalize_sha256_hex(&daemon_artifact.sha256)?;
    let final_path =
        managed_remote_daemon_download_path(app, channel, platform_key, &expected_sha)?;
    cache_release_artifact(
        &final_path,
        daemon_artifact,
        &base_url,
        "managed remote daemon",
        true,
    )
}

pub(crate) fn ensure_managed_remote_bundle_appimage(
    app: &tauri::AppHandle,
    remote_arch: &str,
    channel: &str,
) -> Result<PathBuf> {
    let platform_key = remote_daemon_platform_key_for_arch(remote_arch)?;
    let base_url = bootstrap_download_base_url();
    let manifest = fetch_release_manifest_for_channel(channel, &base_url)?;
    let platform_entry = manifest
        .platforms
        .get(platform_key)
        .ok_or_else(|| anyhow!("release manifest missing platform entry: {platform_key}"))?;
    let bundle_artifact =
        release_bundle_artifact_for_platform(platform_entry).ok_or_else(|| {
            anyhow!("release manifest missing Linux desktop artifact for {platform_key}")
        })?;
    let expected_sha = normalize_sha256_hex(&bundle_artifact.sha256)?;
    let final_path =
        managed_remote_bundle_appimage_download_path(app, channel, platform_key, &expected_sha)?;
    cache_release_artifact(
        &final_path,
        bundle_artifact,
        &base_url,
        "managed remote bundle",
        true,
    )
}
