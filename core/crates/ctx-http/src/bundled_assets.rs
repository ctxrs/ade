use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

const BUNDLE_ENV_DIR: &str = "CTX_BUNDLE_DIR";
const MANIFEST_FILENAME: &str = "manifest.json";
const MANIFEST_VERSION: u32 = 1;

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

fn bundle_dir() -> Option<PathBuf> {
    let raw = std::env::var(BUNDLE_ENV_DIR).ok()?;
    let path = PathBuf::from(raw.trim());
    if path.as_os_str().is_empty() || !path.exists() {
        return None;
    }
    Some(path)
}

fn manifest_path(root: &Path) -> PathBuf {
    root.join(MANIFEST_FILENAME)
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

fn bundled_runtime(id: &str) -> Option<BundledRuntimePaths> {
    let root = bundle_dir()?;
    let manifest = load_manifest()?;
    let (os, arch) = current_platform();
    let entry = manifest
        .runtimes
        .iter()
        .find(|r| r.id == id && r.os == os && r.arch == arch)?;
    let runtime_root =
        resolve_bundle_path(&root, &entry.root).unwrap_or_else(|| root.join(&entry.root));
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

pub fn bundled_node_runtime() -> Option<BundledRuntimePaths> {
    bundled_runtime("node")
}

pub fn bundled_python_runtime() -> Option<BundledRuntimePaths> {
    bundled_runtime("python")
}

pub fn bundled_podman_runtime() -> Option<BundledRuntimePaths> {
    bundled_runtime("podman")
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
