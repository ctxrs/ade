use super::*;
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ReleaseArtifact {
    pub(crate) url_path: String,
    pub(crate) sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedRemoteReleaseArtifact {
    pub(crate) url: String,
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

fn validate_remote_artifact_url(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return Ok(trimmed.to_string());
    }
    anyhow::bail!("managed remote artifact URL must use http or https");
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

pub(crate) fn remote_bundle_dir_for_data_dir(remote_data_dir: &str) -> String {
    format!("{}/bundles", remote_data_dir.trim_end_matches('/'))
}

pub(crate) fn remote_bundle_backup_dir_for_data_dir(remote_data_dir: &str) -> String {
    format!(
        "{}.pre-update-backup",
        remote_bundle_dir_for_data_dir(remote_data_dir)
    )
}

pub(crate) fn release_bundle_artifact_for_platform(
    platform_entry: &ReleasePlatformEntry,
) -> Option<&ReleaseArtifact> {
    platform_entry
        .appimage
        .as_ref()
        .or(platform_entry.desktop.as_ref())
}

fn resolved_release_artifact(
    base_url: &str,
    artifact: &ReleaseArtifact,
) -> Result<ResolvedRemoteReleaseArtifact> {
    let url = validate_remote_artifact_url(&join_url(base_url, &artifact.url_path))?;
    let sha256 = normalize_sha256_hex(&artifact.sha256)?;
    Ok(ResolvedRemoteReleaseArtifact { url, sha256 })
}

pub(crate) fn resolve_managed_remote_daemon_artifact(
    remote_arch: &str,
    channel: &str,
) -> Result<ResolvedRemoteReleaseArtifact> {
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
    resolved_release_artifact(&base_url, daemon_artifact)
}

pub(crate) fn resolve_managed_remote_bundle_appimage_artifact(
    remote_arch: &str,
    channel: &str,
) -> Result<ResolvedRemoteReleaseArtifact> {
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
    resolved_release_artifact(&base_url, bundle_artifact)
}
