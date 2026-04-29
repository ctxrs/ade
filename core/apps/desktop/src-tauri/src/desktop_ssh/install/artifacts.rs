use super::*;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use tauri_plugin_updater::verify_signature;
use url::Url;

const RELEASE_MANIFEST_PUBKEY_OVERRIDE_ENV: &str = "CTX_RELEASE_MANIFEST_PUBKEY";
const EMBEDDED_RELEASE_MANIFEST_PUBKEY: &str = include_str!("../../../config/updater_pubkey.txt");
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
    let decoded_bytes = BASE64_STANDARD.decode(compact.as_bytes()).ok()?;
    let decoded_text = String::from_utf8(decoded_bytes).ok()?;
    normalize_minisign_pubkey_text(&decoded_text)
        .map(|decoded_plain| BASE64_STANDARD.encode(decoded_plain.as_bytes()))
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
    if let Some(pubkey) = std::env::var(RELEASE_MANIFEST_PUBKEY_OVERRIDE_ENV)
        .ok()
        .as_deref()
        .and_then(normalize_updater_pubkey)
    {
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

fn contains_dot_segment(path: &str) -> bool {
    path.split('/').any(|segment| matches!(segment, "." | ".."))
}

fn resolve_base_path_relative_artifact_ref(base: &Url, artifact_ref: &str) -> Url {
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
    resolved
}

fn validate_remote_artifact_url(base: &Url, raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        anyhow::bail!("managed remote artifact url_path is empty");
    }
    if trimmed.contains('\\') {
        anyhow::bail!("managed remote artifact url_path must not contain backslashes: {trimmed}");
    }
    if trimmed.contains('?') || trimmed.contains('#') {
        anyhow::bail!(
            "managed remote artifact url_path must not contain query or fragment: {trimmed}"
        );
    }

    let decoded = urlencoding::decode(trimmed)
        .with_context(|| format!("decoding managed remote artifact url_path: {trimmed}"))?;
    if decoded.contains('\\') || contains_dot_segment(&decoded) {
        anyhow::bail!(
            "managed remote artifact url_path contains an unsafe path segment: {trimmed}"
        );
    }

    let resolved = match Url::parse(trimmed) {
        Ok(absolute) => absolute,
        Err(url::ParseError::RelativeUrlWithoutBase) => {
            if trimmed.starts_with("//") {
                anyhow::bail!(
                    "managed remote artifact url_path must not be scheme-relative: {trimmed}"
                );
            }
            if !trimmed.starts_with('/') {
                anyhow::bail!(
                    "managed remote artifact url_path must be root-relative or same-origin absolute: {trimmed}"
                );
            }
            resolve_base_path_relative_artifact_ref(base, trimmed)
        }
        Err(err) => anyhow::bail!("invalid managed remote artifact url_path '{trimmed}': {err}"),
    };

    if !matches!(resolved.scheme(), "http" | "https") {
        anyhow::bail!(
            "managed remote artifact URL must use http or https: {}",
            resolved
        );
    }
    if resolved.scheme() != base.scheme()
        || resolved.host_str() != base.host_str()
        || resolved.port_or_known_default() != base.port_or_known_default()
    {
        anyhow::bail!(
            "managed remote artifact URL must stay on release base origin {}: {}",
            base.origin().ascii_serialization(),
            resolved
        );
    }
    Ok(resolved.to_string())
}

fn fetch_release_manifest_for_channel(channel: &str, base_url: &str) -> Result<ReleaseManifest> {
    let url = format!(
        "{}/releases/{}/latest.json",
        base_url.trim_end_matches('/'),
        channel
    );
    let signature_url = signature_url_for_manifest(&url)?;
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
    let manifest_bytes = resp.bytes().context("reading release manifest body")?;
    let signature = client
        .get(&signature_url)
        .send()
        .with_context(|| format!("fetching release manifest signature: {signature_url}"))?
        .error_for_status()
        .with_context(|| format!("release manifest signature http error: {signature_url}"))?
        .text()
        .context("reading release manifest signature body")?;
    let pubkey = resolve_release_manifest_pubkey()?;
    verify_signature(&manifest_bytes, signature.trim(), &pubkey)
        .with_context(|| format!("verifying release manifest signature: {url}"))?;
    serde_json::from_slice::<ReleaseManifest>(&manifest_bytes)
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
    let base = Url::parse(base_url)
        .with_context(|| format!("invalid release download base URL: {base_url}"))?;
    let url = validate_remote_artifact_url(&base, &artifact.url_path)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    fn test_artifact(url_path: &str) -> ReleaseArtifact {
        ReleaseArtifact {
            url_path: url_path.to_string(),
            sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        }
    }

    fn spawn_unsigned_manifest_server(manifest_body: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let addr = listener.local_addr().expect("test server addr");
        thread::spawn(move || {
            for stream in listener.incoming().take(2) {
                let Ok(mut stream) = stream else {
                    continue;
                };
                let mut buf = [0u8; 2048];
                let n = stream.read(&mut buf).unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/");
                if path == "/releases/stable/latest.json" {
                    let response = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                        manifest_body.len(),
                        manifest_body
                    );
                    let _ = stream.write_all(response.as_bytes());
                } else {
                    let body = "missing signature";
                    let response = format!(
                        "HTTP/1.1 404 Not Found\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes());
                }
            }
        });
        format!("http://{addr}")
    }

    #[test]
    fn remote_release_artifact_rejects_third_party_absolute_url() {
        let err = resolved_release_artifact(
            "https://api.ctx.rs/functions/v1",
            &test_artifact("https://downloads.example.test/ctx"),
        )
        .expect_err("third-party artifact URL should fail");
        assert!(
            err.to_string().contains("origin"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn remote_release_artifact_preserves_base_path_for_root_relative_url_path() {
        let artifact = resolved_release_artifact(
            "https://api.ctx.rs/functions/v1",
            &test_artifact("/download/stable/1.2.3/ctx.AppImage"),
        )
        .expect("artifact");
        assert_eq!(
            artifact.url,
            "https://api.ctx.rs/functions/v1/download/stable/1.2.3/ctx.AppImage"
        );
    }

    #[test]
    fn remote_release_artifact_rejects_path_traversal() {
        let err = resolved_release_artifact(
            "https://api.ctx.rs/functions/v1",
            &test_artifact("/download/stable/1.2.3/../ctx"),
        )
        .expect_err("traversal artifact URL should fail");
        assert!(
            err.to_string().contains("unsafe path segment"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn remote_release_manifest_requires_signature() {
        let manifest = r#"{"platforms":{"linux-x64":{"daemon":{"url_path":"/download/stable/9.9.9/ctx","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}}}"#;
        let base_url = spawn_unsigned_manifest_server(manifest);
        let err = fetch_release_manifest_for_channel("stable", &base_url)
            .expect_err("unsigned manifest should fail");
        assert!(
            err.to_string().contains("signature"),
            "unexpected error: {err:#}"
        );
    }
}
