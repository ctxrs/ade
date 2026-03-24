use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

const BUNDLE_ENV_DIR: &str = "CTX_BUNDLE_DIR";
const BUNDLE_ENV_MANIFEST: &str = "CTX_BUNDLE_MANIFEST";
const MANIFEST_FILENAME: &str = "manifest.json";
const MANIFEST_VERSION: u32 = 1;
const RUNTIME_LOCK_FILENAME: &str = "runtime_lock.v2.json";
const RUNTIME_LOCK_VERSION: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundledAssetsManifest {
    pub version: u32,
    #[serde(default)]
    pub generated_at: Option<String>,
    #[serde(default)]
    pub providers: Vec<BundledProvider>,
    #[serde(default)]
    pub runtimes: Vec<BundledRuntime>,
    // Container images are Linux artifacts even when bundled on macOS/Windows.
    // They are keyed by the Linux arch token matching the host arch (e.g. aarch64, x86_64).
    #[serde(default)]
    pub images: Vec<BundledImage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundledProvider {
    pub id: String,
    pub protocol: String,
    pub version: String,
    pub os: String,
    pub arch: String,
    pub sha256: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundledRuntime {
    pub id: String,
    pub version: String,
    pub os: String,
    pub arch: String,
    pub sha256: String,
    pub root: String,
    pub bin: String,
    #[serde(default)]
    pub npm_cli: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundledImage {
    pub id: String,
    pub version: String,
    pub os: String,
    pub arch: String,
    pub sha256: String,
    // Path to the image tar, relative to CTX_BUNDLE_DIR.
    pub tar: String,
    // The image reference/tag the tar loads as (e.g. ghcr.io/ctxrs/ctx-harness:ubuntu-24.04).
    pub image: String,
}

#[derive(Debug, Clone)]
pub struct BundledCommand {
    pub command: String,
    pub args: Vec<String>,
    pub version: String,
}

#[derive(Debug, Clone)]
pub struct BundledRuntimePaths {
    pub root: PathBuf,
    pub bin: PathBuf,
    pub npm_cli: Option<PathBuf>,
    pub version: String,
}

#[derive(Debug, Clone)]
pub struct ManagedArtifactSource {
    pub uri: String,
    pub sha256: String,
}

#[derive(Debug, Clone)]
pub struct ManagedRuntimeSource {
    pub uri: String,
    pub sha256: String,
    pub version: String,
    pub bin: String,
    pub helpers: HashMap<String, ManagedArtifactSource>,
}

#[derive(Debug, Clone, Deserialize)]
struct RuntimeLockProfile {
    #[serde(default)]
    allowed_source_types: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct RuntimeLockSource {
    source_type: String,
    #[serde(default)]
    uri: Option<String>,
    #[serde(default)]
    sha256: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct RuntimeLockHelperSource {
    #[serde(default)]
    uri: Option<String>,
    #[serde(default)]
    sha256: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct RuntimeLockComponent {
    kind: String,
    id: String,
    os: String,
    arch: String,
    #[serde(default)]
    variant: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    bin: Option<String>,
    #[serde(default)]
    helpers: HashMap<String, RuntimeLockHelperSource>,
    #[serde(default)]
    sources: Vec<RuntimeLockSource>,
}

#[derive(Debug, Clone, Deserialize)]
struct RuntimeLockV2 {
    version: u32,
    #[serde(default)]
    profiles: HashMap<String, RuntimeLockProfile>,
    #[serde(default)]
    components: Vec<RuntimeLockComponent>,
}

fn bundle_dir() -> Option<PathBuf> {
    #[cfg(test)]
    if let Some((root, _)) = test_manifest_override()
        .lock()
        .expect("test bundled assets manifest override lock poisoned")
        .clone()
    {
        return Some(root);
    }

    let raw = std::env::var(BUNDLE_ENV_DIR).ok()?;
    let path = PathBuf::from(raw.trim());
    if path.as_os_str().is_empty() || !path.exists() {
        return None;
    }
    Some(path)
}

fn manifest_path(root: &Path) -> PathBuf {
    if let Ok(raw) = std::env::var(BUNDLE_ENV_MANIFEST) {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            let candidate = PathBuf::from(trimmed);
            if candidate.is_absolute() {
                return candidate;
            }
            return root.join(candidate);
        }
    }
    root.join(MANIFEST_FILENAME)
}

fn runtime_lock_path(root: &Path) -> PathBuf {
    let manifest = manifest_path(root);
    let sibling = manifest
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| root.to_path_buf())
        .join(RUNTIME_LOCK_FILENAME);
    if sibling.exists() {
        return sibling;
    }
    root.join(RUNTIME_LOCK_FILENAME)
}

fn current_platform() -> (&'static str, &'static str) {
    (std::env::consts::OS, std::env::consts::ARCH)
}

fn current_arch() -> &'static str {
    std::env::consts::ARCH
}

fn resolve_bundle_path(root: &Path, value: &str) -> Option<PathBuf> {
    let candidate = PathBuf::from(value);
    if candidate.is_absolute() {
        return if candidate.exists() {
            Some(candidate)
        } else {
            None
        };
    }

    let direct = root.join(value);
    if direct.exists() {
        return Some(direct);
    }

    let bin = root.join("bin").join(value);
    if bin.exists() {
        return Some(bin);
    }

    None
}

fn resolve_bundle_args(root: &Path, args: &[String]) -> Vec<String> {
    args.iter()
        .map(|arg| {
            if arg.starts_with('-') {
                return arg.clone();
            }
            if Path::new(arg).is_absolute() {
                return arg.clone();
            }
            if let Some(resolved) = resolve_bundle_path(root, arg) {
                return resolved.to_string_lossy().to_string();
            }
            arg.clone()
        })
        .collect()
}

fn load_manifest() -> Option<BundledAssetsManifest> {
    #[cfg(test)]
    if let Some((_, manifest)) = test_manifest_override()
        .lock()
        .expect("test bundled assets manifest override lock poisoned")
        .clone()
    {
        return Some(manifest);
    }

    static MANIFEST: OnceLock<Option<BundledAssetsManifest>> = OnceLock::new();
    let res = MANIFEST.get_or_init(|| {
        let root = bundle_dir()?;
        let path = manifest_path(&root);
        let raw = std::fs::read_to_string(&path).ok()?;
        let parsed: BundledAssetsManifest = match serde_json::from_str(&raw) {
            Ok(parsed) => parsed,
            Err(err) => {
                tracing::warn!(
                    "failed to parse bundled assets manifest {}: {err}",
                    path.display()
                );
                return None;
            }
        };
        if parsed.version != MANIFEST_VERSION {
            tracing::warn!(
                "unsupported bundled assets manifest version {} (expected {})",
                parsed.version,
                MANIFEST_VERSION
            );
            return None;
        }
        Some(parsed)
    });
    res.clone()
}

fn load_runtime_lock() -> Option<RuntimeLockV2> {
    static LOCK: OnceLock<Option<RuntimeLockV2>> = OnceLock::new();
    let res = LOCK.get_or_init(|| {
        let root = bundle_dir()?;
        let path = runtime_lock_path(&root);
        let raw = std::fs::read_to_string(&path).ok()?;
        let parsed: RuntimeLockV2 = match serde_json::from_str(&raw) {
            Ok(parsed) => parsed,
            Err(err) => {
                tracing::warn!("failed to parse runtime lock {}: {err}", path.display());
                return None;
            }
        };
        if parsed.version != RUNTIME_LOCK_VERSION {
            tracing::warn!(
                "unsupported runtime lock version {} (expected {})",
                parsed.version,
                RUNTIME_LOCK_VERSION
            );
            return None;
        }
        Some(parsed)
    });
    res.clone()
}

fn active_runtime_profile() -> &'static str {
    let raw = std::env::var("CTX_RUNTIME_PROFILE").unwrap_or_default();
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "parity" => "parity",
        "override" => "override",
        "source-all" => "source-all",
        _ => "parity",
    }
}

fn allowed_source_types_for_profile(lock: &RuntimeLockV2) -> HashSet<String> {
    let mut out = HashSet::<String>::new();
    let profile = active_runtime_profile();
    let cfg = lock
        .profiles
        .get(profile)
        .or_else(|| lock.profiles.get("parity"));
    if let Some(cfg) = cfg {
        for source_type in &cfg.allowed_source_types {
            let trimmed = source_type.trim();
            if trimmed.is_empty() {
                continue;
            }
            out.insert(trimmed.to_ascii_lowercase());
        }
    }
    out
}

fn select_managed_source(
    component: &RuntimeLockComponent,
    allowed_source_types: &HashSet<String>,
) -> Option<ManagedArtifactSource> {
    component.sources.iter().find_map(|source| {
        let source_type = source.source_type.trim();
        if source_type.is_empty() || source_type.eq_ignore_ascii_case("local") {
            return None;
        }
        if !allowed_source_types.is_empty()
            && !allowed_source_types.contains(&source_type.to_ascii_lowercase())
        {
            return None;
        }
        let uri = source.uri.as_ref()?.trim();
        let sha256 = source.sha256.as_ref()?.trim();
        if uri.is_empty() || sha256.is_empty() {
            return None;
        }
        Some(ManagedArtifactSource {
            uri: uri.to_string(),
            sha256: sha256.to_string(),
        })
    })
}

fn select_managed_runtime_source(
    component: &RuntimeLockComponent,
    allowed_source_types: &HashSet<String>,
) -> Option<ManagedRuntimeSource> {
    let source = select_managed_source(component, allowed_source_types)?;
    let version = component.version.as_deref()?.trim();
    if version.is_empty() {
        return None;
    }
    let bin = component.bin.as_deref()?.trim();
    if bin.is_empty() {
        return None;
    }
    let mut helpers = HashMap::new();
    for (name, helper) in &component.helpers {
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        let uri = helper.uri.as_deref().unwrap_or("").trim();
        let sha256 = helper.sha256.as_deref().unwrap_or("").trim();
        if uri.is_empty() || sha256.is_empty() {
            continue;
        }
        helpers.insert(
            name.to_string(),
            ManagedArtifactSource {
                uri: uri.to_string(),
                sha256: sha256.to_string(),
            },
        );
    }
    Some(ManagedRuntimeSource {
        uri: source.uri,
        sha256: source.sha256,
        version: version.to_string(),
        bin: bin.to_string(),
        helpers,
    })
}

pub fn bundled_provider_command(provider_id: &str) -> Option<BundledCommand> {
    let root = bundle_dir()?;
    let manifest = load_manifest()?;
    let (os, arch) = current_platform();
    let entry = manifest
        .providers
        .iter()
        .find(|p| p.id == provider_id && p.os == os && p.arch == arch)?;
    let command = resolve_bundle_path(&root, &entry.command)?;
    let args = resolve_bundle_args(&root, &entry.args);
    Some(BundledCommand {
        command: command.to_string_lossy().to_string(),
        args,
        version: entry.version.clone(),
    })
}

fn bundled_runtime_from_manifest(
    root: &Path,
    manifest: &BundledAssetsManifest,
    id: &str,
    version: Option<&str>,
) -> Option<BundledRuntimePaths> {
    let (os, arch) = current_platform();
    let entry = manifest.runtimes.iter().find(|r| {
        r.id == id
            && r.os == os
            && r.arch == arch
            && version.is_none_or(|expected| r.version == expected)
    })?;
    let runtime_root =
        resolve_bundle_path(root, &entry.root).unwrap_or_else(|| root.join(&entry.root));
    if !runtime_root.exists() {
        return None;
    }
    let bin = if Path::new(&entry.bin).is_absolute() {
        PathBuf::from(&entry.bin)
    } else {
        runtime_root.join(&entry.bin)
    };
    if !bin.exists() {
        return None;
    }
    let npm_cli = entry.npm_cli.as_ref().map(|raw| {
        if Path::new(raw).is_absolute() {
            PathBuf::from(raw)
        } else {
            runtime_root.join(raw)
        }
    });
    if let Some(path) = npm_cli.as_ref() {
        if !path.exists() {
            return None;
        }
    }
    Some(BundledRuntimePaths {
        root: runtime_root,
        bin,
        npm_cli,
        version: entry.version.clone(),
    })
}

fn bundled_runtime(id: &str) -> Option<BundledRuntimePaths> {
    let root = bundle_dir()?;
    let manifest = load_manifest()?;
    bundled_runtime_from_manifest(&root, &manifest, id, None)
}

pub fn bundled_node_runtime() -> Option<BundledRuntimePaths> {
    bundled_runtime("node")
}

pub fn bundled_python_runtime() -> Option<BundledRuntimePaths> {
    bundled_runtime("python")
}

pub fn bundled_python_runtime_version(version: &str) -> Option<BundledRuntimePaths> {
    let root = bundle_dir()?;
    let manifest = load_manifest()?;
    bundled_runtime_from_manifest(&root, &manifest, "python", Some(version))
}

pub fn bundled_podman_runtime() -> Option<BundledRuntimePaths> {
    bundled_runtime("podman")
}

pub fn bundled_avf_linux_guest_runtime() -> Option<BundledRuntimePaths> {
    bundled_runtime("avf-linux-guest")
}

pub fn bundled_image_tar(id: &str, os: &str, arch: &str) -> Option<PathBuf> {
    let root = bundle_dir()?;
    let manifest = load_manifest()?;
    let entry = manifest
        .images
        .iter()
        .find(|img| img.id == id && img.os == os && img.arch == arch)?;
    resolve_bundle_path(&root, &entry.tar).or_else(|| {
        let direct = root.join(&entry.tar);
        direct.exists().then_some(direct)
    })
}

pub fn bundled_ctx_harness_image_tar(expected_image: &str) -> Option<PathBuf> {
    let manifest = load_manifest()?;
    let arch = current_arch();
    let entry = manifest.images.iter().find(|img| {
        img.id == "ctx-harness"
            && img.os == "linux"
            && img.arch == arch
            && img.image == expected_image
    })?;
    let root = bundle_dir()?;
    resolve_bundle_path(&root, &entry.tar).or_else(|| {
        let direct = root.join(&entry.tar);
        direct.exists().then_some(direct)
    })
}

pub fn managed_image_source(id: &str, os: &str, arch: &str) -> Option<ManagedArtifactSource> {
    let lock = load_runtime_lock()?;
    let allowed_source_types = allowed_source_types_for_profile(&lock);
    let component = lock.components.iter().find(|component| {
        let variant = component
            .variant
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("default");
        component.kind == "image"
            && component.id == id
            && component.os == os
            && component.arch == arch
            && variant == "default"
    })?;
    select_managed_source(component, &allowed_source_types)
}

pub fn managed_machine_cache_source(
    id: &str,
    os: &str,
    arch: &str,
) -> Option<ManagedArtifactSource> {
    let lock = load_runtime_lock()?;
    let allowed_source_types = allowed_source_types_for_profile(&lock);
    let component = lock.components.iter().find(|component| {
        let variant = component
            .variant
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("default");
        component.kind == "machine_cache"
            && component.id == id
            && component.os == os
            && component.arch == arch
            && variant == "default"
    })?;
    select_managed_source(component, &allowed_source_types)
}

pub fn managed_runtime_source(id: &str, os: &str, arch: &str) -> Option<ManagedRuntimeSource> {
    let lock = load_runtime_lock()?;
    let allowed_source_types = allowed_source_types_for_profile(&lock);
    let component = lock.components.iter().find(|component| {
        let variant = component
            .variant
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("default");
        component.kind == "runtime"
            && component.id == id
            && component.os == os
            && component.arch == arch
            && variant == "default"
    })?;
    select_managed_runtime_source(component, &allowed_source_types)
}

pub fn managed_ctx_harness_image_source(_expected_image: &str) -> Option<ManagedArtifactSource> {
    #[cfg(test)]
    if let Some(source) = test_managed_ctx_harness_image_source_override()
        .lock()
        .expect("test managed harness image override lock poisoned")
        .clone()
    {
        return Some(source);
    }

    managed_image_source("ctx-harness", "linux", current_arch())
}

pub fn managed_podman_machine_cache_source() -> Option<ManagedArtifactSource> {
    #[cfg(test)]
    if let Some(source) = test_managed_podman_machine_cache_source_override()
        .lock()
        .expect("test managed machine cache override lock poisoned")
        .clone()
    {
        return Some(source);
    }

    managed_machine_cache_source("podman-machine", current_platform().0, current_platform().1)
}

#[cfg(test)]
fn test_managed_podman_machine_cache_source_override(
) -> &'static std::sync::Mutex<Option<ManagedArtifactSource>> {
    static OVERRIDE: std::sync::OnceLock<std::sync::Mutex<Option<ManagedArtifactSource>>> =
        std::sync::OnceLock::new();
    OVERRIDE.get_or_init(|| std::sync::Mutex::new(None))
}

#[cfg(test)]
fn test_managed_ctx_harness_image_source_override(
) -> &'static std::sync::Mutex<Option<ManagedArtifactSource>> {
    static OVERRIDE: std::sync::OnceLock<std::sync::Mutex<Option<ManagedArtifactSource>>> =
        std::sync::OnceLock::new();
    OVERRIDE.get_or_init(|| std::sync::Mutex::new(None))
}

#[cfg(test)]
fn test_manifest_override() -> &'static std::sync::Mutex<Option<(PathBuf, BundledAssetsManifest)>> {
    static OVERRIDE: std::sync::OnceLock<
        std::sync::Mutex<Option<(PathBuf, BundledAssetsManifest)>>,
    > = std::sync::OnceLock::new();
    OVERRIDE.get_or_init(|| std::sync::Mutex::new(None))
}

#[cfg(test)]
pub(crate) struct TestManagedPodmanMachineCacheSourceGuard {
    previous: Option<ManagedArtifactSource>,
}

#[cfg(test)]
impl Drop for TestManagedPodmanMachineCacheSourceGuard {
    fn drop(&mut self) {
        let mut guard = test_managed_podman_machine_cache_source_override()
            .lock()
            .expect("test managed machine cache override lock poisoned");
        *guard = self.previous.take();
    }
}

#[cfg(test)]
pub(crate) struct TestManagedCtxHarnessImageSourceGuard {
    previous: Option<ManagedArtifactSource>,
}

#[cfg(test)]
impl Drop for TestManagedCtxHarnessImageSourceGuard {
    fn drop(&mut self) {
        let mut guard = test_managed_ctx_harness_image_source_override()
            .lock()
            .expect("test managed harness image override lock poisoned");
        *guard = self.previous.take();
    }
}

#[cfg(test)]
#[allow(dead_code)]
pub(crate) struct TestBundledAssetsManifestGuard {
    previous: Option<(PathBuf, BundledAssetsManifest)>,
}

#[cfg(test)]
impl Drop for TestBundledAssetsManifestGuard {
    fn drop(&mut self) {
        let mut guard = test_manifest_override()
            .lock()
            .expect("test bundled assets manifest override lock poisoned");
        *guard = self.previous.take();
    }
}

#[cfg(test)]
pub(crate) fn override_managed_podman_machine_cache_source_for_test(
    source: ManagedArtifactSource,
) -> TestManagedPodmanMachineCacheSourceGuard {
    let mut guard = test_managed_podman_machine_cache_source_override()
        .lock()
        .expect("test managed machine cache override lock poisoned");
    let previous = guard.clone();
    *guard = Some(source);
    TestManagedPodmanMachineCacheSourceGuard { previous }
}

#[cfg(test)]
pub(crate) fn override_managed_ctx_harness_image_source_for_test(
    source: ManagedArtifactSource,
) -> TestManagedCtxHarnessImageSourceGuard {
    let mut guard = test_managed_ctx_harness_image_source_override()
        .lock()
        .expect("test managed harness image override lock poisoned");
    let previous = guard.clone();
    *guard = Some(source);
    TestManagedCtxHarnessImageSourceGuard { previous }
}

#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn override_bundled_assets_manifest_for_test(
    root: PathBuf,
    manifest: BundledAssetsManifest,
) -> TestBundledAssetsManifestGuard {
    let mut guard = test_manifest_override()
        .lock()
        .expect("test bundled assets manifest override lock poisoned");
    let previous = guard.clone();
    *guard = Some((root, manifest));
    TestBundledAssetsManifestGuard { previous }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn manifest_path_uses_explicit_absolute_override() {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let root = PathBuf::from("/tmp/ctx-bundles-root");
        let absolute = PathBuf::from("/tmp/ctx-manifest-absolute.json");
        std::env::set_var(BUNDLE_ENV_MANIFEST, absolute.to_string_lossy().to_string());
        let resolved = manifest_path(&root);
        std::env::remove_var(BUNDLE_ENV_MANIFEST);
        assert_eq!(resolved, absolute);
    }

    #[test]
    fn manifest_path_uses_relative_override_with_bundle_root() {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let root = PathBuf::from("/tmp/ctx-bundles-root");
        std::env::set_var(BUNDLE_ENV_MANIFEST, "runtime_manifest.effective.json");
        let resolved = manifest_path(&root);
        std::env::remove_var(BUNDLE_ENV_MANIFEST);
        assert_eq!(resolved, root.join("runtime_manifest.effective.json"));
    }

    #[test]
    fn manifest_path_defaults_to_bundle_manifest() {
        let _guard = env_lock().lock().expect("env lock poisoned");
        std::env::remove_var(BUNDLE_ENV_MANIFEST);
        let root = PathBuf::from("/tmp/ctx-bundles-root");
        let resolved = manifest_path(&root);
        assert_eq!(resolved, root.join(MANIFEST_FILENAME));
    }

    #[test]
    fn bundled_runtime_from_manifest_can_select_python_by_version() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let (os, arch) = current_platform();
        let runtime_313_root = root.join("runtimes/python/runtime-313");
        let runtime_312_root = root.join("runtimes/python/runtime-312");
        std::fs::create_dir_all(runtime_313_root.join("bin")).expect("mkdir runtime 313");
        std::fs::create_dir_all(runtime_312_root.join("bin")).expect("mkdir runtime 312");
        std::fs::write(runtime_313_root.join("bin/python3"), b"python313")
            .expect("write runtime 313");
        std::fs::write(runtime_312_root.join("bin/python3"), b"python312")
            .expect("write runtime 312");

        let manifest = BundledAssetsManifest {
            version: MANIFEST_VERSION,
            generated_at: None,
            providers: vec![],
            runtimes: vec![
                BundledRuntime {
                    id: "python".to_string(),
                    version: "3.13.12".to_string(),
                    os: os.to_string(),
                    arch: arch.to_string(),
                    sha256: "sha313".to_string(),
                    root: "runtimes/python/runtime-313".to_string(),
                    bin: "bin/python3".to_string(),
                    npm_cli: None,
                },
                BundledRuntime {
                    id: "python".to_string(),
                    version: "3.12.13".to_string(),
                    os: os.to_string(),
                    arch: arch.to_string(),
                    sha256: "sha312".to_string(),
                    root: "runtimes/python/runtime-312".to_string(),
                    bin: "bin/python3".to_string(),
                    npm_cli: None,
                },
            ],
            images: vec![],
        };

        let bundled = bundled_runtime_from_manifest(root, &manifest, "python", Some("3.12.13"))
            .expect("bundled runtime");
        assert_eq!(bundled.version, "3.12.13");
        assert_eq!(bundled.bin, runtime_312_root.join("bin/python3"));
    }

    #[test]
    fn select_managed_source_ignores_local_entries() {
        let component = RuntimeLockComponent {
            kind: "image".to_string(),
            id: "ctx-harness".to_string(),
            os: "linux".to_string(),
            arch: "aarch64".to_string(),
            variant: Some("default".to_string()),
            version: None,
            bin: None,
            helpers: HashMap::new(),
            sources: vec![
                RuntimeLockSource {
                    source_type: "local".to_string(),
                    uri: None,
                    sha256: None,
                },
                RuntimeLockSource {
                    source_type: "ci".to_string(),
                    uri: Some("https://example.test/image.tar".to_string()),
                    sha256: Some("abcd".to_string()),
                },
            ],
        };
        let mut allowed = HashSet::new();
        allowed.insert("ci".to_string());
        let source = select_managed_source(&component, &allowed).expect("managed source");
        assert_eq!(source.uri, "https://example.test/image.tar");
        assert_eq!(source.sha256, "abcd");
    }

    #[test]
    fn select_managed_source_respects_allowed_source_types() {
        let component = RuntimeLockComponent {
            kind: "image".to_string(),
            id: "ctx-harness".to_string(),
            os: "linux".to_string(),
            arch: "x86_64".to_string(),
            variant: Some("default".to_string()),
            version: None,
            bin: None,
            helpers: HashMap::new(),
            sources: vec![RuntimeLockSource {
                source_type: "vendor".to_string(),
                uri: Some("https://example.test/image.tar".to_string()),
                sha256: Some("abcd".to_string()),
            }],
        };
        let mut allowed = HashSet::new();
        allowed.insert("ci".to_string());
        assert!(select_managed_source(&component, &allowed).is_none());
    }

    #[test]
    fn select_managed_runtime_source_extracts_version_bin_and_helpers() {
        let component = RuntimeLockComponent {
            kind: "runtime".to_string(),
            id: "podman".to_string(),
            os: "macos".to_string(),
            arch: "aarch64".to_string(),
            variant: Some("default".to_string()),
            version: Some("5.8.0".to_string()),
            bin: Some("usr/bin/podman".to_string()),
            helpers: HashMap::from([(
                "gvproxy".to_string(),
                RuntimeLockHelperSource {
                    uri: Some("https://example.test/gvproxy".to_string()),
                    sha256: Some("1234".to_string()),
                },
            )]),
            sources: vec![RuntimeLockSource {
                source_type: "vendor".to_string(),
                uri: Some("https://example.test/podman.zip".to_string()),
                sha256: Some("abcd".to_string()),
            }],
        };
        let mut allowed = HashSet::new();
        allowed.insert("vendor".to_string());
        let source = select_managed_runtime_source(&component, &allowed).expect("runtime source");
        assert_eq!(source.uri, "https://example.test/podman.zip");
        assert_eq!(source.sha256, "abcd");
        assert_eq!(source.version, "5.8.0");
        assert_eq!(source.bin, "usr/bin/podman");
        let helper = source
            .helpers
            .get("gvproxy")
            .expect("gvproxy helper should be present");
        assert_eq!(helper.uri, "https://example.test/gvproxy");
        assert_eq!(helper.sha256, "1234");
    }

    #[test]
    fn select_managed_runtime_source_supports_avf_guest_helper_payloads() {
        let component = RuntimeLockComponent {
            kind: "runtime".to_string(),
            id: "avf-linux-guest".to_string(),
            os: "macos".to_string(),
            arch: "aarch64".to_string(),
            variant: Some("default".to_string()),
            version: Some("locked".to_string()),
            bin: Some("rootfs.raw".to_string()),
            helpers: HashMap::from([
                (
                    "kernel".to_string(),
                    RuntimeLockHelperSource {
                        uri: Some("https://example.test/kernel".to_string()),
                        sha256: Some("1111".to_string()),
                    },
                ),
                (
                    "initrd".to_string(),
                    RuntimeLockHelperSource {
                        uri: Some("https://example.test/initrd".to_string()),
                        sha256: Some("2222".to_string()),
                    },
                ),
                (
                    "egress-proxy".to_string(),
                    RuntimeLockHelperSource {
                        uri: Some("https://example.test/egress-proxy".to_string()),
                        sha256: Some("3333".to_string()),
                    },
                ),
            ]),
            sources: vec![RuntimeLockSource {
                source_type: "ci".to_string(),
                uri: Some("https://example.test/rootfs.raw.zst".to_string()),
                sha256: Some("abcd".to_string()),
            }],
        };
        let mut allowed = HashSet::new();
        allowed.insert("ci".to_string());
        let source = select_managed_runtime_source(&component, &allowed).expect("runtime source");
        assert_eq!(source.uri, "https://example.test/rootfs.raw.zst");
        assert_eq!(source.sha256, "abcd");
        assert_eq!(source.version, "locked");
        assert_eq!(source.bin, "rootfs.raw");
        assert_eq!(
            source
                .helpers
                .get("kernel")
                .map(|helper| helper.uri.as_str()),
            Some("https://example.test/kernel")
        );
        assert_eq!(
            source
                .helpers
                .get("initrd")
                .map(|helper| helper.uri.as_str()),
            Some("https://example.test/initrd")
        );
        assert_eq!(
            source
                .helpers
                .get("egress-proxy")
                .map(|helper| helper.uri.as_str()),
            Some("https://example.test/egress-proxy")
        );
    }

    #[test]
    fn managed_podman_machine_cache_source_can_be_overridden_for_tests() {
        let override_source = ManagedArtifactSource {
            uri: "https://example.test/podman-machine.raw.zst".to_string(),
            sha256: "cafebabe".to_string(),
        };
        let guard = override_managed_podman_machine_cache_source_for_test(override_source.clone());
        let resolved = managed_podman_machine_cache_source().expect("override should resolve");
        assert_eq!(resolved.uri, override_source.uri);
        assert_eq!(resolved.sha256, override_source.sha256);
        drop(guard);
        if let Some(restored) = managed_podman_machine_cache_source() {
            assert!(
                restored.uri != override_source.uri || restored.sha256 != override_source.sha256,
                "dropping the guard should restore the prior source"
            );
        }
    }
}
