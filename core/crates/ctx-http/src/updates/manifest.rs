use super::appimage::{appimage_path_env, in_place_update_capability_with_appimage_path};
use super::*;

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

pub fn normalize_release_channel(channel: &str) -> Result<String> {
    let trimmed = channel.trim();
    if trimmed.is_empty() {
        anyhow::bail!("channel must be non-empty");
    }
    if trimmed.len() > 64 {
        anyhow::bail!("channel must be 64 characters or fewer");
    }
    if !trimmed
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        anyhow::bail!("channel must use only ASCII letters, digits, '.', '_' or '-'");
    }
    Ok(trimmed.to_string())
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
    let channel = normalize_release_channel(channel)?;
    let mut url = release_manifest_url(base_url, &channel);
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
