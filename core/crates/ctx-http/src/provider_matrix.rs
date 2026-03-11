use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

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
    pub kind: ProviderMatrixEntryKind,
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProviderMatrixEntryKind {
    #[default]
    Harness,
    Dependency,
}

impl ProviderMatrixEntryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Harness => "harness",
            Self::Dependency => "dependency",
        }
    }
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
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub size_bytes: Option<u64>,
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
    Dmg,
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
    pub upstream_version: Option<String>,
    #[serde(default)]
    pub provenance: Option<ProviderReleaseProvenance>,
    #[serde(default)]
    pub context_min: Option<String>,
    #[serde(default)]
    pub context_max: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderReleaseProvenance {
    #[serde(default)]
    pub upstream_repo: Option<String>,
    #[serde(default)]
    pub upstream_release_tag: Option<String>,
    #[serde(default)]
    pub upstream_commit_sha: Option<String>,
    #[serde(default)]
    pub ctx_repo: Option<String>,
    #[serde(default)]
    pub ctx_release_tag: Option<String>,
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

pub fn matrix_cache_path(data_root: &Path) -> PathBuf {
    data_root.join("providers").join(MATRIX_CACHE_FILENAME)
}

pub fn builtin_matrix() -> ProviderMatrix {
    ProviderMatrix::default()
}

pub async fn load_matrix(data_root: &Path) -> ProviderMatrix {
    // Desktop/runtime invariant: provider matrix is bundled (or local cached), never
    // dynamically refreshed from remote endpoints at runtime.
    if let Some(matrix) = load_cached_matrix(data_root) {
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

pub async fn invalidate_matrix_cache(cache: &tokio::sync::Mutex<ProviderMatrixCache>) {
    let mut guard = cache.lock().await;
    guard.cached_at = None;
    guard.matrix = None;
}

pub fn get_entry<'a>(
    matrix: &'a ProviderMatrix,
    provider_id: &str,
) -> Option<&'a ProviderMatrixEntry> {
    matrix.providers.iter().find(|p| p.id == provider_id)
}

pub fn is_user_facing_harness_id(matrix: &ProviderMatrix, provider_id: &str) -> bool {
    get_entry(matrix, provider_id)
        .map(|entry| entry.kind == ProviderMatrixEntryKind::Harness)
        .unwrap_or(true)
}

pub fn is_managed_supported(matrix: &ProviderMatrix, provider_id: &str) -> bool {
    let Some(entry) = get_entry(matrix, provider_id) else {
        return false;
    };
    if entry.managed_install.is_none() {
        return false;
    }
    let context_version = updates::normalize_version_str(env!("CARGO_PKG_VERSION"));
    recommended_release(entry, context_version.as_ref()).is_some()
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
    status.details.insert(
        "provider_kind".to_string(),
        entry.kind.as_str().to_string(),
    );
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
        if let Some(upstream) = rec.upstream_version.as_ref() {
            status.details.insert(
                "matrix_recommended_upstream_version".to_string(),
                upstream.clone(),
            );
        }
    }
    if let Some(latest) = latest_release(entry) {
        status
            .details
            .insert("matrix_latest_version".to_string(), latest.version.clone());
        if let Some(upstream) = latest.upstream_version.as_ref() {
            status.details.insert(
                "matrix_latest_upstream_version".to_string(),
                upstream.clone(),
            );
        }
    }

    let mut diagnostics = Vec::new();

    if status.installed {
        if let Some(version) = detected_version.as_deref() {
            match release_for_version(entry, version) {
                Some(release) => {
                    if let Some(upstream) = release.upstream_version.as_ref() {
                        status.details.insert(
                            "matrix_detected_upstream_version".to_string(),
                            upstream.clone(),
                        );
                    }
                    if release.status != ProviderReleaseStatus::Supported {
                        diagnostics.push(format!(
                            "Provider version {} is blocked by the support matrix",
                            release.version
                        ));
                    } else if !release_matches_context(release, context_version) {
                        let mut msg = "Provider version requires a newer ctx build".to_string();
                        if let Some(min) = release.context_min.as_ref() {
                            msg = format!("Provider version requires ctx >= {min}");
                        }
                        diagnostics.push(msg);
                    }
                }
                None => {
                    diagnostics.push(format!(
                        "Provider version {} is not in the support matrix",
                        version
                    ));
                }
            }
        } else {
            diagnostics.push("Unable to determine provider version".to_string());
        }
    }

    if !diagnostics.is_empty() {
        status.diagnostics.extend(diagnostics);
    }

    let release_update_available = match (
        detected_version.as_deref(),
        status.details.get("matrix_recommended_version"),
    ) {
        (Some(installed), Some(recommended)) => {
            normalize_version(installed) != normalize_version(recommended)
        }
        _ => false,
    };
    let dependency_update_available =
        status.installed && managed_dependency_update_available(cfg, status);
    if dependency_update_available {
        status.details.insert(
            "managed_dependency_update_available".to_string(),
            "true".to_string(),
        );
    }
    let update_available = release_update_available || dependency_update_available;
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

fn managed_dependency_update_available(
    cfg: &AgentServerConfigFile,
    status: &ctx_providers::adapters::ProviderStatus,
) -> bool {
    let requested_target = install_target_from_status(status);
    let command = crate::installer::managed_provider_command_for_target(
        cfg,
        &status.provider_id,
        requested_target,
    )
    .or_else(|| {
        cfg.providers
            .get(&status.provider_id)
            .filter(|command| command.managed.is_none())
            .cloned()
    });
    let Some(command) = command else {
        return false;
    };
    command.dependencies.iter().any(|dependency_id| {
        let Some(expected_version) =
            crate::installer::expected_managed_dependency_version(dependency_id)
        else {
            return false;
        };
        let installed_version = cfg
            .managed_installs
            .get(dependency_id)
            .and_then(|meta| meta.version.as_deref());
        match installed_version {
            Some(installed) => normalize_version(installed) != normalize_version(expected_version),
            None => true,
        }
    })
}

fn install_target_from_status(
    status: &ctx_providers::adapters::ProviderStatus,
) -> Option<crate::installs::InstallTarget> {
    status
        .details
        .get("install_target")
        .or_else(|| status.details.get("managed_target"))
        .and_then(|value| crate::installer::parse_install_target(Some(value.as_str())).ok())
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
    let requested_target = install_target_from_status(status);
    if let Some(meta) = crate::installer::managed_install_metadata_for_target(
        cfg,
        &status.provider_id,
        requested_target,
    ) {
        if let Some(version) = meta.version.clone() {
            return Some(version);
        }
    }

    let probe = entry.version_probe.as_ref()?;
    let command = match crate::installer::resolve_runtime_provider_command_for_target(
        cfg,
        &status.provider_id,
        requested_target,
    ) {
        Ok(Some(command)) => ProviderCommand {
            command: command.command_abs_path,
            args: command.args,
        },
        Ok(None) => return None,
        Err(err) => {
            tracing::debug!(
                provider_id = %status.provider_id,
                "skipping version probe: {err}"
            );
            return None;
        }
    };

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

#[cfg(test)]
async fn save_cached_matrix(data_root: &Path, matrix: &ProviderMatrix) -> anyhow::Result<()> {
    use anyhow::Context;

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

fn strip_cli_suffix(raw: &str) -> &str {
    raw.strip_suffix("-cli")
        .or_else(|| raw.strip_suffix("_cli"))
        .unwrap_or(raw)
}

fn version_matches(release: &str, detected: &str) -> bool {
    let a = normalize_version(release);
    let b = normalize_version(detected);
    if a == b {
        return true;
    }
    strip_cli_suffix(&a) == strip_cli_suffix(&b)
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
    use crate::installer::{AgentServerCommand, ManagedInstallMetadata};
    use tempfile::tempdir;

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
                upstream_version: None,
                context_min: None,
                context_max: None,
                notes: None,
                provenance: None,
            },
            ProviderRelease {
                version: "0.7.3".to_string(),
                status: ProviderReleaseStatus::Supported,
                upstream_version: None,
                context_min: None,
                context_max: None,
                notes: None,
                provenance: None,
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
            upstream_version: None,
            context_min: Some("1.2.0".to_string()),
            context_max: None,
            notes: None,
            provenance: None,
        };
        let ctx = Version::parse("1.1.0").ok();
        assert!(!release_matches_context(&release, ctx.as_ref()));
    }

    #[test]
    fn version_matches_suffix_release() {
        assert!(version_matches("1.0.1-cli", "1.0.1"));
        assert!(version_matches("1.0.1", "1.0.1-cli"));
    }

    #[tokio::test]
    async fn load_matrix_returns_cached_when_present() {
        let dir = tempdir().expect("tempdir");
        let data_root = dir.path();

        let cached = ProviderMatrix {
            version: MATRIX_SCHEMA_VERSION,
            generated_at: Some("2026-02-23T00:00:00Z".to_string()),
            providers: vec![ProviderMatrixEntry {
                id: "cached-provider".to_string(),
                kind: ProviderMatrixEntryKind::Harness,
                display_name: Some("Cached Provider".to_string()),
                tier: Some("tier3".to_string()),
                command: None,
                managed_install: None,
                dependencies: vec![],
                version_probe: None,
                releases: vec![],
            }],
        };
        save_cached_matrix(data_root, &cached)
            .await
            .expect("save cached matrix");

        let loaded = load_matrix(data_root).await;
        assert_eq!(loaded.version, MATRIX_SCHEMA_VERSION);
        assert_eq!(loaded.providers.len(), 1);
        assert_eq!(loaded.providers[0].id, "cached-provider");
    }

    #[tokio::test]
    async fn load_matrix_returns_builtin_when_cache_missing() {
        let dir = tempdir().expect("tempdir");
        let loaded = load_matrix(dir.path()).await;
        let builtin = builtin_matrix();
        assert_eq!(loaded.version, builtin.version);
        assert_eq!(loaded.providers.len(), builtin.providers.len());
    }

    #[test]
    fn provider_matrix_entry_kind_defaults_to_harness_when_missing_from_json() {
        let entry: ProviderMatrixEntry = serde_json::from_str(
            r#"{
              "id": "example-provider",
              "managed_install": {
                "kind": "npm",
                "package": "example",
                "entrypoint": "bin/example.js",
                "args": []
              },
              "releases": []
            }"#,
        )
        .expect("entry parses");

        assert_eq!(entry.kind, ProviderMatrixEntryKind::Harness);
    }

    #[test]
    fn builtin_matrix_marks_dependencies_and_omits_cagent() {
        let matrix = builtin_matrix();
        let bridge = get_entry(&matrix, "acp-crp-bridge").expect("bridge entry");
        let claude_cli = get_entry(&matrix, "claude-cli").expect("claude-cli entry");

        assert_eq!(bridge.kind, ProviderMatrixEntryKind::Dependency);
        assert_eq!(claude_cli.kind, ProviderMatrixEntryKind::Dependency);
        assert!(get_entry(&matrix, "cagent").is_none());
    }

    #[test]
    fn user_facing_harness_filter_excludes_known_dependencies_only() {
        let matrix = builtin_matrix();

        assert!(is_user_facing_harness_id(&matrix, "codex"));
        assert!(!is_user_facing_harness_id(&matrix, "acp-crp-bridge"));
        assert!(!is_user_facing_harness_id(&matrix, "claude-cli"));
        assert!(is_user_facing_harness_id(&matrix, "unknown-provider"));
    }

    #[test]
    fn managed_dependency_update_available_when_runtime_dependency_missing() {
        let mut cfg = AgentServerConfigFile::default();
        cfg.providers.insert(
            "codex".to_string(),
            AgentServerCommand {
                command: "/tmp/codex".to_string(),
                args: Vec::new(),
                dependencies: vec!["runtime-node-host".to_string()],
                managed: None,
            },
        );
        let status = ctx_providers::adapters::ProviderStatus {
            provider_id: "codex".to_string(),
            installed: true,
            detected_path: None,
            version: None,
            capabilities: None,
            health: ctx_providers::adapters::ProviderHealth::Ok,
            diagnostics: Vec::new(),
            details: HashMap::new(),
        };
        assert!(managed_dependency_update_available(&cfg, &status));
    }

    #[test]
    fn managed_dependency_update_available_when_runtime_dependency_version_mismatched() {
        let mut cfg = AgentServerConfigFile::default();
        cfg.providers.insert(
            "codex".to_string(),
            AgentServerCommand {
                command: "/tmp/codex".to_string(),
                args: Vec::new(),
                dependencies: vec!["runtime-node-host".to_string()],
                managed: None,
            },
        );
        cfg.managed_installs.insert(
            "runtime-node-host".to_string(),
            ManagedInstallMetadata {
                package: Some("node-runtime".to_string()),
                version: Some("0.0.1".to_string()),
                target: None,
                install_dir_rel: None,
                bin_dir_rel: None,
                last_success_at: None,
                last_error: None,
            },
        );
        let status = ctx_providers::adapters::ProviderStatus {
            provider_id: "codex".to_string(),
            installed: true,
            detected_path: None,
            version: None,
            capabilities: None,
            health: ctx_providers::adapters::ProviderHealth::Ok,
            diagnostics: Vec::new(),
            details: HashMap::new(),
        };
        assert!(managed_dependency_update_available(&cfg, &status));
    }

    #[test]
    fn managed_dependency_update_unavailable_when_runtime_dependency_matches_expected() {
        let mut cfg = AgentServerConfigFile::default();
        cfg.providers.insert(
            "codex".to_string(),
            AgentServerCommand {
                command: "/tmp/codex".to_string(),
                args: Vec::new(),
                dependencies: vec!["runtime-node-host".to_string()],
                managed: None,
            },
        );
        let expected = crate::installer::expected_managed_dependency_version("runtime-node-host")
            .expect("runtime node version");
        cfg.managed_installs.insert(
            "runtime-node-host".to_string(),
            ManagedInstallMetadata {
                package: Some("node-runtime".to_string()),
                version: Some(expected.to_string()),
                target: None,
                install_dir_rel: None,
                bin_dir_rel: None,
                last_success_at: None,
                last_error: None,
            },
        );
        let status = ctx_providers::adapters::ProviderStatus {
            provider_id: "codex".to_string(),
            installed: true,
            detected_path: None,
            version: None,
            capabilities: None,
            health: ctx_providers::adapters::ProviderHealth::Ok,
            diagnostics: Vec::new(),
            details: HashMap::new(),
        };
        assert!(!managed_dependency_update_available(&cfg, &status));
    }
}
