use super::appimage::{appimage_path_env, in_place_update_capability_with_appimage_path};
use super::*;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use minisign_verify::{PublicKey, Signature};
use url::Url;

const RELEASE_MANIFEST_PUBKEY_OVERRIDE_ENV: &str = "CTX_RELEASE_MANIFEST_PUBKEY";
const EMBEDDED_RELEASE_MANIFEST_PUBKEY: &str =
    include_str!("../../../apps/desktop/src-tauri/config/updater_pubkey.txt");

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
    if matches!(trimmed, "." | "..") {
        anyhow::bail!("channel must not be '.' or '..'");
    }
    if !trimmed
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        anyhow::bail!("channel must use only ASCII letters, digits, '.', '_' or '-'");
    }
    Ok(trimmed.to_string())
}

pub fn normalize_release_artifact_sha256(raw: &str) -> Result<String> {
    let normalized = raw.trim().to_ascii_lowercase();
    if normalized.len() != 64 || !normalized.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        anyhow::bail!("release manifest artifact sha256 must be a 64-character hex digest");
    }
    Ok(normalized)
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

fn normalize_updater_pubkey(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(normalized_plain) = normalize_minisign_pubkey_text(trimmed) {
        return Some(BASE64_STANDARD.encode(normalized_plain.as_bytes()));
    }
    let compact: String = trimmed
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect();
    if let Some(decoded_plain) = decode_base64_minisign_pubkey(&compact) {
        return Some(BASE64_STANDARD.encode(decoded_plain.as_bytes()));
    }
    None
}

fn decode_base64_minisign_pubkey(encoded: &str) -> Option<String> {
    let decoded_bytes = BASE64_STANDARD.decode(encoded.as_bytes()).ok()?;
    let decoded_text = String::from_utf8(decoded_bytes).ok()?;
    normalize_minisign_pubkey_text(&decoded_text)
}

fn normalize_minisign_pubkey_text(raw: &str) -> Option<String> {
    let normalized = raw.replace("\r\n", "\n");
    let mut lines = normalized
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    let header = lines.next()?;
    if !header.starts_with("untrusted comment: minisign public key:") {
        return None;
    }
    let key_line = lines.next()?;
    if key_line.is_empty() || lines.next().is_some() {
        return None;
    }
    Some(format!("{header}\n{key_line}\n"))
}

fn resolve_release_manifest_pubkey() -> Result<String> {
    let runtime_override = std::env::var(RELEASE_MANIFEST_PUBKEY_OVERRIDE_ENV).ok();
    let runtime = runtime_override
        .as_deref()
        .and_then(normalize_updater_pubkey);
    if let Some(pubkey) = runtime {
        return Ok(pubkey);
    }
    normalize_updater_pubkey(EMBEDDED_RELEASE_MANIFEST_PUBKEY)
        .context("embedded release manifest updater public key is invalid")
}

fn signature_url_for_manifest(manifest_url: &str) -> Result<String> {
    let mut parsed = Url::parse(manifest_url)
        .with_context(|| format!("invalid release manifest url: {manifest_url}"))?;
    let next_path = format!("{}.sig", parsed.path());
    parsed.set_path(&next_path);
    Ok(parsed.to_string())
}

fn base64_to_utf8_text(label: &str, encoded: &str) -> Result<String> {
    let decoded = BASE64_STANDARD
        .decode(encoded.trim().as_bytes())
        .with_context(|| format!("decoding {label} from base64"))?;
    std::str::from_utf8(&decoded)
        .with_context(|| format!("{label} is not valid utf-8"))
        .map(|value| value.to_string())
}

fn verify_release_manifest_signature(
    manifest_body: &[u8],
    signature_b64: &str,
    pubkey_b64: &str,
) -> Result<()> {
    let pubkey_text = base64_to_utf8_text("release manifest public key", pubkey_b64)?;
    let signature_text = base64_to_utf8_text("release manifest signature", signature_b64)?;
    let public_key =
        PublicKey::decode(&pubkey_text).context("decoding release manifest public key")?;
    let signature =
        Signature::decode(&signature_text).context("decoding release manifest signature")?;
    public_key
        .verify(manifest_body, &signature, true)
        .context("verifying release manifest signature")?;
    Ok(())
}

fn release_artifacts_for_platform(
    platform: &ReleasePlatform,
) -> Vec<(&'static str, &ReleaseArtifact)> {
    let mut artifacts = Vec::new();
    if let Some(artifact) = platform.desktop.as_ref() {
        artifacts.push(("desktop", artifact));
    }
    if let Some(artifact) = platform.appimage.as_ref() {
        artifacts.push(("appimage", artifact));
    }
    if let Some(artifact) = platform.deb.as_ref() {
        artifacts.push(("deb", artifact));
    }
    if let Some(artifact) = platform.dmg.as_ref() {
        artifacts.push(("dmg", artifact));
    }
    if let Some(artifact) = platform.msi.as_ref() {
        artifacts.push(("msi", artifact));
    }
    if let Some(artifact) = platform.nsis.as_ref() {
        artifacts.push(("nsis", artifact));
    }
    if let Some(artifact) = platform.exe.as_ref() {
        artifacts.push(("exe", artifact));
    }
    if let Some(artifact) = platform.zip.as_ref() {
        artifacts.push(("zip", artifact));
    }
    if let Some(artifact) = platform.daemon.as_ref() {
        artifacts.push(("daemon", artifact));
    }
    artifacts
}

fn contains_dot_segment(path: &str) -> bool {
    path.split('/').any(|segment| matches!(segment, "." | ".."))
}

fn resolve_base_path_relative_artifact_ref(base: &Url, artifact_ref: &str) -> Result<Url> {
    let mut resolved = base.clone();
    resolved.set_query(None);
    resolved.set_fragment(None);

    let base_path = base.path().trim_end_matches('/');
    let artifact_path = artifact_ref.trim_start_matches('/');
    let joined_path = if base_path.is_empty() || base_path == "/" {
        format!("/{artifact_path}")
    } else {
        format!("{base_path}/{artifact_path}")
    };
    resolved.set_path(&joined_path);
    Ok(resolved)
}

fn validate_release_artifact_ref(base: &Url, raw_ref: &str) -> Result<Url> {
    let trimmed = raw_ref.trim();
    if trimmed.is_empty() {
        anyhow::bail!("release manifest artifact url_path is empty");
    }
    if trimmed.contains('\\') {
        anyhow::bail!("release manifest artifact url_path must not contain backslashes: {trimmed}");
    }
    if trimmed.contains('?') || trimmed.contains('#') {
        anyhow::bail!(
            "release manifest artifact url_path must not contain query or fragment: {trimmed}"
        );
    }

    let decoded = urlencoding::decode(trimmed)
        .with_context(|| format!("decoding release manifest artifact url_path: {trimmed}"))?;
    if decoded.contains('\\') || contains_dot_segment(&decoded) {
        anyhow::bail!(
            "release manifest artifact url_path contains an unsafe path segment: {trimmed}"
        );
    }

    let resolved = match Url::parse(trimmed) {
        Ok(absolute) => absolute,
        Err(url::ParseError::RelativeUrlWithoutBase) => {
            if trimmed.starts_with("//") {
                anyhow::bail!(
                    "release manifest artifact url_path must not be scheme-relative: {trimmed}"
                );
            }
            if !trimmed.starts_with('/') {
                anyhow::bail!(
                    "release manifest artifact url_path must be root-relative or same-origin absolute: {trimmed}"
                );
            }
            resolve_base_path_relative_artifact_ref(base, trimmed).with_context(|| {
                format!("resolving release manifest artifact url_path: {trimmed}")
            })?
        }
        Err(err) => {
            anyhow::bail!("invalid release manifest artifact url_path '{trimmed}': {err}");
        }
    };

    if !matches!(resolved.scheme(), "http" | "https") {
        anyhow::bail!(
            "release manifest artifact URL must use http or https: {}",
            resolved
        );
    }
    if resolved.scheme() != base.scheme()
        || resolved.host_str() != base.host_str()
        || resolved.port_or_known_default() != base.port_or_known_default()
    {
        anyhow::bail!(
            "release manifest artifact URL must stay on release base origin {}: {}",
            base.origin().ascii_serialization(),
            resolved
        );
    }
    let base_path = base.path().trim_end_matches('/');
    if !base_path.is_empty() && base_path != "/" {
        let resolved_path = resolved.path();
        if resolved_path != base_path
            && !resolved_path
                .strip_prefix(base_path)
                .is_some_and(|suffix| suffix.starts_with('/'))
        {
            anyhow::bail!(
                "release manifest artifact URL must stay under release base path {base_path}: {}",
                resolved
            );
        }
    }
    Ok(resolved)
}

pub fn resolve_release_artifact_url(base_url: &str, url_path: &str) -> Result<String> {
    let base = Url::parse(base_url)
        .with_context(|| format!("invalid release download base URL: {base_url}"))?;
    Ok(validate_release_artifact_ref(&base, url_path)?.to_string())
}

fn validate_release_manifest_artifact_references(
    manifest: &ReleaseManifest,
    base_url: &str,
) -> Result<()> {
    let base = Url::parse(base_url)
        .with_context(|| format!("invalid release download base URL: {base_url}"))?;
    for (platform_key, platform) in &manifest.platforms {
        for (artifact_key, artifact) in release_artifacts_for_platform(platform) {
            validate_release_artifact_ref(&base, &artifact.url_path).with_context(|| {
                format!("validating release manifest artifact {platform_key}/{artifact_key}")
            })?;
            normalize_release_artifact_sha256(&artifact.sha256).with_context(|| {
                format!("validating release manifest artifact {platform_key}/{artifact_key} sha256")
            })?;
        }
    }
    Ok(())
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
    let signature_url = signature_url_for_manifest(&url)?;
    let manifest_bytes = reqwest::get(&url)
        .await
        .with_context(|| format!("fetching release manifest: {url}"))?
        .error_for_status()
        .with_context(|| format!("release manifest http error: {url}"))?
        .bytes()
        .await
        .context("reading release manifest body")?;
    let signature_b64 = reqwest::get(&signature_url)
        .await
        .with_context(|| format!("fetching release manifest signature: {signature_url}"))?
        .error_for_status()
        .with_context(|| format!("release manifest signature http error: {signature_url}"))?
        .text()
        .await
        .context("reading release manifest signature body")?;
    let manifest_pubkey = resolve_release_manifest_pubkey()?;
    verify_release_manifest_signature(&manifest_bytes, &signature_b64, &manifest_pubkey)?;
    let txt =
        std::str::from_utf8(&manifest_bytes).context("release manifest body is not valid utf-8")?;
    let manifest: ReleaseManifest =
        serde_json::from_str(txt).context("parsing release manifest JSON")?;
    validate_release_manifest_artifact_references(&manifest, base_url)?;
    Ok(manifest)
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
