use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result};
use semver::Version;
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tokio::time::timeout;

use crate::installer::AgentServerConfigFile;
use crate::updates;

const MATRIX_CACHE_TTL: Duration = Duration::from_secs(6 * 60 * 60);
const MATRIX_CACHE_FILENAME: &str = "provider_matrix.json";
const MATRIX_SCHEMA_VERSION: u32 = 2;
const VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(4);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderMatrix {
    pub version: u32,
    #[serde(default)]
    pub generated_at: Option<String>,
    #[serde(default)]
    pub providers: Vec<ProviderMatrixEntry>,
}

impl Default for ProviderMatrix {
    fn default() -> Self {
        serde_json::from_str(include_str!("provider_matrix.json")).unwrap_or(ProviderMatrix {
            version: MATRIX_SCHEMA_VERSION,
            generated_at: None,
            providers: vec![],
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderMatrixEntry {
    pub id: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub tier: Option<String>,
    #[serde(default)]
    pub command: Option<ProviderCommand>,
    #[serde(default, rename = "managed_install")]
    pub managed_install: Option<ProviderInstall>,
    #[serde(default)]
    pub dependencies: Vec<ProviderDependency>,
    #[serde(default)]
    pub version_probe: Option<VersionProbe>,
    #[serde(default)]
    pub releases: Vec<ProviderRelease>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCommand {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProviderInstall {
    Npm {
        package: String,
        entrypoint: String,
        #[serde(default)]
        args: Vec<String>,
    },
    Archive {
        version: String,
        #[serde(default)]
        args: Vec<String>,
        targets: HashMap<String, ProviderArchiveTarget>,
    },
    Python {
        package: String,
        version: String,
        entrypoint: String,
        #[serde(default)]
        args: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderDependency {
    pub id: String,
    #[serde(rename = "install")]
    pub install: DependencyInstall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DependencyInstall {
    Npm {
        package: String,
        version: String,
    },
    Archive {
        version: String,
        targets: HashMap<String, ProviderArchiveTarget>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderArchiveTarget {
    pub url: String,
    pub archive: ProviderArchiveKind,
    pub bin_path: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderArchiveKind {
    None,
    TarGz,
    TarBz2,
    Zip,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VersionProbe {
    Command { args: Vec<String> },
    NodePackage { package: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderRelease {
    pub version: String,
    #[serde(default)]
    pub status: ProviderReleaseStatus,
    #[serde(default)]
    pub context_min: Option<String>,
    #[serde(default)]
    pub context_max: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderReleaseStatus {
    #[default]
    Supported,
    Blocked,
    Deprecated,
}

#[derive(Debug, Default)]
pub struct ProviderMatrixCache {
    pub cached_at: Option<Instant>,
    pub matrix: Option<ProviderMatrix>,
}

pub fn default_matrix_base_url() -> String {
    std::env::var("CTX_PROVIDER_MATRIX_BASE_URL")
        .or_else(|_| std::env::var("CONTEXT_PROVIDER_MATRIX_BASE_URL"))
        .unwrap_or_else(|_| updates::default_download_base_url())
}

pub fn default_matrix_channel() -> String {
    std::env::var("CTX_PROVIDER_MATRIX_CHANNEL")
        .or_else(|_| std::env::var("CONTEXT_PROVIDER_MATRIX_CHANNEL"))
        .unwrap_or_else(|_| "stable".to_string())
}

pub fn matrix_cache_path(data_root: &Path) -> PathBuf {
    data_root.join("providers").join(MATRIX_CACHE_FILENAME)
}

pub fn builtin_matrix() -> ProviderMatrix {
    ProviderMatrix::default()
}

pub async fn load_matrix(data_root: &Path) -> ProviderMatrix {
    let cached = load_cached_matrix(data_root);
    let refresh = cached
        .as_ref()
        .and_then(|_| cached_age_ok(data_root).ok())
        .map(|ok| !ok)
        .unwrap_or(true);

    if !refresh {
        if let Some(matrix) = cached {
            return matrix;
        }
    }

    let base_url = default_matrix_base_url();
    let channel = default_matrix_channel();
    match fetch_remote_matrix(&base_url, &channel).await {
        Ok(matrix) => {
            let _ = save_cached_matrix(data_root, &matrix).await;
            return matrix;
        }
        Err(err) => {
            tracing::warn!("failed to fetch provider matrix: {err:#}");
        }
    }

    if let Some(matrix) = cached {
        return matrix;
    }

    builtin_matrix()
}

pub async fn load_matrix_cached(
    data_root: &Path,
    cache: &tokio::sync::Mutex<ProviderMatrixCache>,
) -> ProviderMatrix {
    let cached = {
        let guard = cache.lock().await;
        if let Some(at) = guard.cached_at {
            if at.elapsed() < MATRIX_CACHE_TTL {
                guard.matrix.clone()
            } else {
                None
            }
        } else {
            None
        }
    };

    if let Some(matrix) = cached {
        return matrix;
    }

    let matrix = load_matrix(data_root).await;
    let mut guard = cache.lock().await;
    guard.cached_at = Some(Instant::now());
    guard.matrix = Some(matrix.clone());
    matrix
}

pub fn get_entry<'a>(
    matrix: &'a ProviderMatrix,
    provider_id: &str,
) -> Option<&'a ProviderMatrixEntry> {
    matrix.providers.iter().find(|p| p.id == provider_id)
}

pub fn is_managed_supported(matrix: &ProviderMatrix, provider_id: &str) -> bool {
    get_entry(matrix, provider_id)
        .and_then(|p| p.managed_install.as_ref())
        .is_some()
}

pub fn recommended_release<'a>(
    entry: &'a ProviderMatrixEntry,
    context_version: Option<&Version>,
) -> Option<&'a ProviderRelease> {
    let candidates: Vec<&ProviderRelease> = entry
        .releases
        .iter()
        .filter(|r| r.status == ProviderReleaseStatus::Supported)
        .filter(|r| release_matches_context(r, context_version))
        .collect();

    select_latest_release(&candidates)
}

pub fn latest_release(entry: &ProviderMatrixEntry) -> Option<&ProviderRelease> {
    let candidates: Vec<&ProviderRelease> = entry
        .releases
        .iter()
        .filter(|r| r.status == ProviderReleaseStatus::Supported)
        .collect();
    select_latest_release(&candidates)
}

pub fn release_for_version<'a>(
    entry: &'a ProviderMatrixEntry,
    version: &str,
) -> Option<&'a ProviderRelease> {
    entry
        .releases
        .iter()
        .find(|r| version_matches(&r.version, version))
}

pub async fn apply_matrix_to_status(
    data_root: &Path,
    cfg: &AgentServerConfigFile,
    entry: &ProviderMatrixEntry,
    status: &mut ctx_providers::adapters::ProviderStatus,
) {
    let context_version = updates::normalize_version_str(env!("CARGO_PKG_VERSION"));
    let context_version = context_version.as_ref();

    let detected_version = detect_provider_version(data_root, cfg, entry, status).await;
    if let Some(version) = detected_version.clone() {
        status.version = Some(version);
    }

    if let Some(rec) = recommended_release(entry, context_version) {
        status.details.insert(
            "matrix_recommended_version".to_string(),
            rec.version.clone(),
        );
    }
    if let Some(latest) = latest_release(entry) {
        status
            .details
            .insert("matrix_latest_version".to_string(), latest.version.clone());
    }

    let mut supported = true;
    let mut diagnostics = Vec::new();

    if status.installed {
        if let Some(version) = detected_version.as_deref() {
            match release_for_version(entry, version) {
                Some(release) => {
                    if release.status != ProviderReleaseStatus::Supported {
                        supported = false;
                        diagnostics.push(format!(
                            "Provider version {} is blocked by the support matrix",
                            release.version
                        ));
                    } else if !release_matches_context(release, context_version) {
                        supported = false;
                        let mut msg = "Provider version requires a newer ctx build".to_string();
                        if let Some(min) = release.context_min.as_ref() {
                            msg = format!("Provider version requires ctx >= {min}");
                        }
                        diagnostics.push(msg);
                    }
                }
                None => {
                    supported = false;
                    diagnostics.push(format!(
                        "Provider version {} is not in the support matrix",
                        version
                    ));
                }
            }
        } else {
            supported = false;
            diagnostics.push("Unable to determine provider version".to_string());
        }
    }

    if status.installed
        && matches!(status.health, ctx_providers::adapters::ProviderHealth::Ok)
        && !supported
    {
        status.health = ctx_providers::adapters::ProviderHealth::UnsupportedVersion;
    }

    if !diagnostics.is_empty() {
        status.diagnostics.extend(diagnostics);
    }

    let update_available = match (
        detected_version.as_deref(),
        status.details.get("matrix_recommended_version"),
    ) {
        (Some(installed), Some(recommended)) => {
            normalize_version(installed) != normalize_version(recommended)
        }
        _ => false,
    };
    if update_available {
        status
            .details
            .insert("matrix_update_available".to_string(), "true".to_string());
    }

    let update_requires_context = match (
        status.details.get("matrix_recommended_version"),
        status.details.get("matrix_latest_version"),
    ) {
        (Some(recommended), Some(latest)) => {
            normalize_version(recommended) != normalize_version(latest)
        }
        _ => false,
    };
    if update_requires_context {
        status.details.insert(
            "matrix_update_requires_context".to_string(),
            "true".to_string(),
        );
    }
}

async fn detect_provider_version(
    data_root: &Path,
    cfg: &AgentServerConfigFile,
    entry: &ProviderMatrixEntry,
    status: &ctx_providers::adapters::ProviderStatus,
) -> Option<String> {
    if !status.installed {
        return None;
    }
    if let Some(meta) = cfg
        .providers
        .get(&status.provider_id)
        .and_then(|c| c.managed.as_ref())
    {
        if let Some(version) = meta.version.clone() {
            return Some(version);
        }
    }
    if let Some(meta) = cfg.managed_installs.get(&status.provider_id) {
        if let Some(version) = meta.version.clone() {
            return Some(version);
        }
    }

    let probe = entry.version_probe.as_ref()?;
    let command = cfg
        .providers
        .get(&status.provider_id)
        .map(|c| ProviderCommand {
            command: c.command.clone(),
            args: c.args.clone(),
        })
        .or_else(|| entry.command.clone());
    let command = command?;

    match probe {
        VersionProbe::Command { args } => probe_command_version(&command.command, args).await,
        VersionProbe::NodePackage { package } => {
            probe_node_package_version(&command, package, data_root)
        }
    }
}

async fn probe_command_version(command: &str, args: &[String]) -> Option<String> {
    let mut cmd = Command::new(command);
    cmd.args(args)
        .kill_on_drop(true)
        .env("NO_COLOR", "1")
        .env("CLICOLOR", "0");

    let output = timeout(VERSION_PROBE_TIMEOUT, cmd.output())
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    extract_version(&format!("{}\n{}", stdout, stderr))
}

fn probe_node_package_version(
    command: &ProviderCommand,
    package: &str,
    data_root: &Path,
) -> Option<String> {
    let script_path = if command.command.ends_with("node") || command.command.ends_with("node.exe")
    {
        command.args.first().map(PathBuf::from)
    } else {
        resolve_command_path(&command.command).or_else(|| {
            if command.command.contains(std::path::MAIN_SEPARATOR) {
                Some(PathBuf::from(&command.command))
            } else {
                None
            }
        })
    };

    let script_path = script_path?;
    let resolved = std::fs::canonicalize(&script_path).unwrap_or(script_path);

    // Check for managed installs first.
    if resolved.starts_with(data_root) {
        if let Some(version) = find_package_version(&resolved, package) {
            return Some(version);
        }
    }

    find_package_version(&resolved, package)
}

fn find_package_version(path: &Path, package: &str) -> Option<String> {
    for ancestor in path.ancestors() {
        let direct = ancestor.join("package.json");
        if let Some(version) = read_package_version(&direct, package) {
            return Some(version);
        }
        let nested = ancestor
            .join("node_modules")
            .join(package)
            .join("package.json");
        if let Some(version) = read_package_version(&nested, package) {
            return Some(version);
        }
    }
    None
}

fn read_package_version(path: &Path, package: &str) -> Option<String> {
    let raw = std::fs::read_to_string(path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let name = json.get("name")?.as_str()?;
    if name != package {
        return None;
    }
    json.get("version")?.as_str().map(|s| s.to_string())
}

fn resolve_command_path(command: &str) -> Option<PathBuf> {
    if command.contains(std::path::MAIN_SEPARATOR)
        || command.contains('/')
        || command.contains('\\')
    {
        let p = PathBuf::from(command);
        return if p.exists() { Some(p) } else { None };
    }
    which::which(command).ok()
}

fn load_cached_matrix(data_root: &Path) -> Option<ProviderMatrix> {
    let path = matrix_cache_path(data_root);
    let txt = std::fs::read_to_string(&path).ok()?;
    let parsed: ProviderMatrix = serde_json::from_str(&txt).ok()?;
    if parsed.version != MATRIX_SCHEMA_VERSION {
        return None;
    }
    Some(parsed)
}

async fn save_cached_matrix(data_root: &Path, matrix: &ProviderMatrix) -> Result<()> {
    let path = matrix_cache_path(data_root);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let txt = serde_json::to_string_pretty(matrix).context("serializing provider matrix")?;
    tokio::fs::write(&path, txt)
        .await
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

fn cached_age_ok(data_root: &Path) -> Result<bool> {
    let path = matrix_cache_path(data_root);
    if !path.exists() {
        return Ok(false);
    }
    let meta = std::fs::metadata(&path).context("reading matrix metadata")?;
    let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    let age = SystemTime::now()
        .duration_since(modified)
        .unwrap_or(Duration::from_secs(0));
    Ok(age < MATRIX_CACHE_TTL)
}

async fn fetch_remote_matrix(base_url: &str, channel: &str) -> Result<ProviderMatrix> {
    let mut url = format!(
        "{}/provider-matrix/{}/latest.json",
        base_url.trim_end_matches('/'),
        channel
    );

    let mut params = Vec::new();
    params.push(("context_version", env!("CARGO_PKG_VERSION").to_string()));
    if let Some(platform) = updates::platform_key() {
        params.push(("platform", platform.to_string()));
    }
    if !params.is_empty() {
        let qs = params
            .iter()
            .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&");
        url.push('?');
        url.push_str(&qs);
    }

    let txt = reqwest::get(&url)
        .await
        .with_context(|| format!("fetching provider matrix: {url}"))?
        .error_for_status()
        .with_context(|| format!("provider matrix http error: {url}"))?
        .text()
        .await
        .context("reading provider matrix body")?;
    let parsed: ProviderMatrix =
        serde_json::from_str(&txt).context("parsing provider matrix JSON")?;
    if parsed.version != MATRIX_SCHEMA_VERSION {
        anyhow::bail!(
            "unsupported provider matrix version {} (expected {})",
            parsed.version,
            MATRIX_SCHEMA_VERSION
        );
    }
    Ok(parsed)
}

fn release_matches_context(release: &ProviderRelease, context_version: Option<&Version>) -> bool {
    let Some(ctx) = context_version else {
        return true;
    };
    if let Some(min) = release.context_min.as_deref() {
        if let Some(min_v) = parse_version_loose(min) {
            if ctx < &min_v {
                return false;
            }
        }
    }
    if let Some(max) = release.context_max.as_deref() {
        if let Some(max_v) = parse_version_loose(max) {
            if ctx > &max_v {
                return false;
            }
        }
    }
    true
}

fn select_latest_release<'a>(candidates: &[&'a ProviderRelease]) -> Option<&'a ProviderRelease> {
    let mut best: Option<(&ProviderRelease, Version)> = None;
    for release in candidates {
        if let Some(parsed) = parse_version_loose(&release.version) {
            match &best {
                Some((_, best_v)) if parsed <= *best_v => {}
                _ => best = Some((release, parsed)),
            }
        }
    }
    if let Some((release, _)) = best {
        return Some(release);
    }
    candidates.last().copied()
}

fn parse_version_loose(raw: &str) -> Option<Version> {
    let trimmed = raw.trim().trim_start_matches('v');
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(v) = Version::parse(trimmed) {
        return Some(v);
    }
    if trimmed.matches('.').count() == 1 {
        let candidate = format!("{}.0", trimmed);
        if let Ok(v) = Version::parse(&candidate) {
            return Some(v);
        }
    }
    None
}

pub fn normalize_version(raw: &str) -> String {
    raw.trim().trim_start_matches('v').to_string()
}

fn version_matches(release: &str, detected: &str) -> bool {
    let rel = normalize_version(release);
    let det = normalize_version(detected);
    if rel == det {
        return true;
    }
    if rel.starts_with(&format!("{det}-")) {
        return true;
    }
    if det.starts_with(&format!("{rel}-")) {
        return true;
    }
    false
}

fn extract_version(text: &str) -> Option<String> {
    let mut buf = String::new();
    let mut started = false;
    for ch in text.chars() {
        if !started {
            if ch.is_ascii_digit() {
                started = true;
                buf.push(ch);
            }
            continue;
        }
        if ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' || ch == '_' {
            buf.push(ch);
            continue;
        }
        break;
    }
    if started {
        Some(buf)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_version_loose_accepts_two_part_versions() {
        let v = parse_version_loose("0.62").expect("expected version");
        assert_eq!(v.to_string(), "0.62.0");
    }

    #[test]
    fn select_latest_release_prefers_semver() {
        let releases = [
            ProviderRelease {
                version: "0.7.1".to_string(),
                status: ProviderReleaseStatus::Supported,
                context_min: None,
                context_max: None,
                notes: None,
            },
            ProviderRelease {
                version: "0.7.3".to_string(),
                status: ProviderReleaseStatus::Supported,
                context_min: None,
                context_max: None,
                notes: None,
            },
        ];
        let refs = releases.iter().collect::<Vec<_>>();
        let latest = select_latest_release(&refs).expect("latest");
        assert_eq!(latest.version, "0.7.3");
    }

    #[test]
    fn release_matches_context_min() {
        let release = ProviderRelease {
            version: "1.0.0".to_string(),
            status: ProviderReleaseStatus::Supported,
            context_min: Some("1.2.0".to_string()),
            context_max: None,
            notes: None,
        };
        let ctx = Version::parse("1.1.0").ok();
        assert!(!release_matches_context(&release, ctx.as_ref()));
    }

    #[test]
    fn version_matches_suffix_release() {
        assert!(version_matches("1.0.1-cli", "1.0.1"));
        assert!(version_matches("1.0.1", "1.0.1-cli"));
    }
}
