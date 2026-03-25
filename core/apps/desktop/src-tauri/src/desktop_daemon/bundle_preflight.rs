use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
struct DesktopBundledAssetsManifest {
    #[allow(dead_code)]
    version: u32,
    #[serde(default)]
    providers: Vec<DesktopBundledProvider>,
    #[serde(default)]
    runtimes: Vec<DesktopBundledRuntime>,
    #[serde(default)]
    images: Vec<DesktopBundledImage>,
}

#[derive(Debug, Clone, Deserialize)]
struct DesktopBundledProvider {
    id: String,
    os: String,
    arch: String,
    command: String,
}

#[derive(Debug, Clone, Deserialize)]
struct DesktopBundledRuntime {
    id: String,
    os: String,
    arch: String,
    root: String,
    bin: String,
}

#[derive(Debug, Clone, Deserialize)]
struct DesktopBundledImage {
    id: String,
    os: String,
    arch: String,
    tar: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RuntimeLockRequiredTargets {
    #[serde(default)]
    provider: Vec<String>,
    #[serde(default)]
    runtime: Vec<String>,
    #[serde(default)]
    image: Vec<String>,
    #[serde(default)]
    machine_cache: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RuntimeLockRequired {
    #[serde(default)]
    provider_ids: Vec<String>,
    #[serde(default)]
    runtime_ids: Vec<String>,
    #[serde(default)]
    image_ids: Vec<String>,
    #[serde(default)]
    machine_cache_ids: Vec<String>,
    #[serde(default)]
    targets: RuntimeLockRequiredTargets,
}

#[derive(Debug, Clone, Deserialize)]
struct RuntimeLockV2 {
    version: u32,
    #[serde(default)]
    profiles: HashMap<String, RuntimeLockProfile>,
    required: RuntimeLockRequired,
    #[serde(default)]
    components: Vec<RuntimeLockComponent>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RuntimeLockProfile {
    #[serde(default)]
    allowed_source_types: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RuntimeLockComponentSource {
    #[serde(default)]
    source_type: String,
    #[serde(default)]
    uri: Option<String>,
    #[serde(default)]
    sha256: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RuntimeLockComponentHelper {
    #[serde(default)]
    uri: Option<String>,
    #[serde(default)]
    sha256: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RuntimeLockComponentHelpers {
    #[serde(default)]
    kernel: Option<RuntimeLockComponentHelper>,
    #[serde(default)]
    initrd: Option<RuntimeLockComponentHelper>,
    #[serde(rename = "guest-agent", default)]
    guest_agent: Option<RuntimeLockComponentHelper>,
    #[serde(rename = "egress-proxy", default)]
    egress_proxy: Option<RuntimeLockComponentHelper>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RuntimeLockComponent {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    id: String,
    #[serde(default)]
    os: String,
    #[serde(default)]
    arch: String,
    #[serde(default)]
    variant: Option<String>,
    #[serde(default)]
    sources: Vec<RuntimeLockComponentSource>,
    #[serde(default)]
    helpers: RuntimeLockComponentHelpers,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RuntimeTarget {
    os: String,
    arch: String,
}

fn parity_profile_enabled() -> bool {
    matches!(
        std::env::var("CTX_RUNTIME_PROFILE")
            .ok()
            .map(|value| value.trim().to_ascii_lowercase())
            .as_deref(),
        None | Some("") | Some("parity")
    )
}

fn active_runtime_profile() -> &'static str {
    match std::env::var("CTX_RUNTIME_PROFILE")
        .ok()
        .map(|value| value.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("override") => "override",
        Some("source-all") => "source-all",
        _ => "parity",
    }
}

fn allowed_source_types_for_profile(lock: &RuntimeLockV2) -> HashSet<String> {
    let mut out = HashSet::new();
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
            out.insert(trimmed.to_string());
        }
    }
    out
}

fn lock_component_has_managed_source(
    component: &RuntimeLockComponent,
    allowed_sources: &HashSet<String>,
) -> bool {
    component.sources.iter().any(|source| {
        let source_type = source.source_type.trim();
        if source_type.is_empty() || source_type == "local" {
            return false;
        }
        if !allowed_sources.is_empty() && !allowed_sources.contains(source_type) {
            return false;
        }
        source
            .uri
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_some()
            && source
                .sha256
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_some()
    })
}

fn required_component_has_managed_source(
    lock: &RuntimeLockV2,
    kind: &str,
    id: &str,
    target: &RuntimeTarget,
    allowed_sources: &HashSet<String>,
) -> bool {
    find_required_component(lock, kind, id, target)
        .map(|component| lock_component_has_managed_source(component, allowed_sources))
        .unwrap_or(false)
}

fn find_required_component<'a>(
    lock: &'a RuntimeLockV2,
    kind: &str,
    id: &str,
    target: &RuntimeTarget,
) -> Option<&'a RuntimeLockComponent> {
    lock.components.iter().find(|component| {
        let variant = component
            .variant
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("default");
        component.kind == kind
            && component.id == id
            && component.os == target.os
            && component.arch == target.arch
            && variant == "default"
    })
}

fn helper_metadata_complete(helper: Option<&RuntimeLockComponentHelper>) -> bool {
    helper
        .and_then(|helper| helper.uri.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some()
        && helper
            .and_then(|helper| helper.sha256.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_some()
}

fn avf_helper_names_and_paths() -> [(&'static str, &'static str); 4] {
    [
        ("kernel", "helpers/kernel"),
        ("initrd", "helpers/initrd"),
        ("guest-agent", "helpers/guest-agent"),
        ("egress-proxy", "helpers/egress-proxy"),
    ]
}

fn avf_helper_metadata_complete(component: &RuntimeLockComponent) -> bool {
    helper_metadata_complete(component.helpers.kernel.as_ref())
        && helper_metadata_complete(component.helpers.initrd.as_ref())
        && helper_metadata_complete(component.helpers.guest_agent.as_ref())
        && helper_metadata_complete(component.helpers.egress_proxy.as_ref())
}

fn normalize_target_token(raw: &str, host_value: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.eq_ignore_ascii_case("host") {
        return Some(host_value.to_string());
    }
    Some(trimmed.to_string())
}

fn parse_target(raw: &str, host_os: &str, host_arch: &str) -> Option<RuntimeTarget> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (os, arch) = trimmed.split_once('/')?;
    let os = normalize_target_token(os, host_os)?;
    let arch = normalize_target_token(arch, host_arch)?;
    Some(RuntimeTarget { os, arch })
}

fn required_targets_or_default(
    configured: &[String],
    fallback: &[RuntimeTarget],
    host_os: &str,
    host_arch: &str,
) -> Vec<RuntimeTarget> {
    if configured.is_empty() {
        return fallback.to_vec();
    }
    let mut out = Vec::<RuntimeTarget>::new();
    for value in configured {
        if let Some(target) = parse_target(value, host_os, host_arch) {
            if !out.contains(&target) {
                out.push(target);
            }
        }
    }
    if out.is_empty() {
        return fallback.to_vec();
    }
    out
}

fn host_default_provider_targets() -> Vec<RuntimeTarget> {
    let host_os = std::env::consts::OS.to_string();
    let host_arch = std::env::consts::ARCH.to_string();
    if host_os == "macos" && host_arch == "aarch64" {
        return vec![
            RuntimeTarget {
                os: "macos".to_string(),
                arch: "aarch64".to_string(),
            },
            RuntimeTarget {
                os: "linux".to_string(),
                arch: "aarch64".to_string(),
            },
            RuntimeTarget {
                os: "linux".to_string(),
                arch: "x86_64".to_string(),
            },
        ];
    }
    let mut out = vec![RuntimeTarget {
        os: host_os.clone(),
        arch: host_arch.clone(),
    }];
    let linux_target = RuntimeTarget {
        os: "linux".to_string(),
        arch: host_arch,
    };
    if !out.contains(&linux_target) {
        out.push(linux_target);
    }
    out
}

fn host_default_runtime_targets() -> Vec<RuntimeTarget> {
    host_default_provider_targets()
}

fn host_default_image_targets() -> Vec<RuntimeTarget> {
    let host_arch = std::env::consts::ARCH.to_string();
    if std::env::consts::OS == "macos" && host_arch == "aarch64" {
        return vec![
            RuntimeTarget {
                os: "linux".to_string(),
                arch: "aarch64".to_string(),
            },
            RuntimeTarget {
                os: "linux".to_string(),
                arch: "x86_64".to_string(),
            },
        ];
    }
    vec![RuntimeTarget {
        os: "linux".to_string(),
        arch: host_arch,
    }]
}

fn host_default_machine_cache_targets() -> Vec<RuntimeTarget> {
    if std::env::consts::OS != "macos" {
        return Vec::new();
    }
    vec![RuntimeTarget {
        os: "macos".to_string(),
        arch: std::env::consts::ARCH.to_string(),
    }]
}

fn host_relevant_targets(
    all_targets: &[RuntimeTarget],
    fallback: &[RuntimeTarget],
) -> Vec<RuntimeTarget> {
    let mut out = Vec::<RuntimeTarget>::new();
    for target in all_targets {
        if fallback.contains(target) && !out.contains(target) {
            out.push(target.clone());
        }
    }
    if !out.is_empty() {
        return out;
    }
    fallback.to_vec()
}

fn bundle_manifest_path(bundle_dir: &Path) -> PathBuf {
    if let Ok(raw) = std::env::var("CTX_BUNDLE_MANIFEST") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            let candidate = PathBuf::from(trimmed);
            if candidate.is_absolute() {
                return candidate;
            }
            return bundle_dir.join(candidate);
        }
    }
    bundle_dir.join("manifest.json")
}

pub(super) fn enforce_desktop_parity_bundle_preflight(bundle_dir: Option<&Path>) -> Result<()> {
    if !parity_profile_enabled() {
        return Ok(());
    }
    let channel = std::env::var("CTX_DESKTOP_CHANNEL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "dev".to_string());
    let surface = std::env::var("CTX_LAUNCH_SURFACE")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "desktop".to_string());

    let bundle_dir = bundle_dir.ok_or_else(|| anyhow!("bundle dir not found"))?;
    let manifest_path = bundle_manifest_path(bundle_dir);
    let manifest_parent = manifest_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| bundle_dir.to_path_buf());
    let manifest_sibling_lock = manifest_parent.join("runtime_lock.v2.json");
    let lock_path = if manifest_sibling_lock.exists() {
        manifest_sibling_lock
    } else {
        bundle_dir.join("runtime_lock.v2.json")
    };
    let lock_raw = std::fs::read_to_string(&lock_path)
        .with_context(|| format!("reading {}", lock_path.display()))?;
    let lock: RuntimeLockV2 = serde_json::from_str(&lock_raw)
        .with_context(|| format!("parsing {}", lock_path.display()))?;
    if lock.version != 2 {
        anyhow::bail!(
            "unsupported runtime lock version {} at {}",
            lock.version,
            lock_path.display()
        );
    }

    let manifest_raw = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("reading {}", manifest_path.display()))?;
    let manifest: DesktopBundledAssetsManifest = serde_json::from_str(&manifest_raw)
        .with_context(|| format!("parsing {}", manifest_path.display()))?;

    let provider_default_targets = host_default_provider_targets();
    let runtime_default_targets = host_default_runtime_targets();
    let image_default_targets = host_default_image_targets();
    let machine_cache_default_targets = host_default_machine_cache_targets();
    let host_os = std::env::consts::OS;
    let host_arch = std::env::consts::ARCH;
    let provider_targets = host_relevant_targets(
        &required_targets_or_default(
            &lock.required.targets.provider,
            &provider_default_targets,
            host_os,
            host_arch,
        ),
        &provider_default_targets,
    );
    let runtime_targets = host_relevant_targets(
        &required_targets_or_default(
            &lock.required.targets.runtime,
            &runtime_default_targets,
            host_os,
            host_arch,
        ),
        &runtime_default_targets,
    );
    let image_targets = host_relevant_targets(
        &required_targets_or_default(
            &lock.required.targets.image,
            &image_default_targets,
            host_os,
            host_arch,
        ),
        &image_default_targets,
    );
    let machine_cache_targets = host_relevant_targets(
        &required_targets_or_default(
            &lock.required.targets.machine_cache,
            &machine_cache_default_targets,
            host_os,
            host_arch,
        ),
        &machine_cache_default_targets,
    );
    let allowed_managed_sources = allowed_source_types_for_profile(&lock);

    let mut failures = Vec::<String>::new();

    for provider_id in &lock.required.provider_ids {
        for target in &provider_targets {
            let Some(entry) = manifest.providers.iter().find(|entry| {
                entry.id == *provider_id && entry.os == target.os && entry.arch == target.arch
            }) else {
                failures.push(format!(
                    "missing provider entry: {} ({}/{})",
                    provider_id, target.os, target.arch
                ));
                continue;
            };
            let command_path = bundle_dir.join(&entry.command);
            if !command_path.exists() {
                failures.push(format!(
                    "missing provider command file: {} ({}/{}) at {}",
                    provider_id,
                    target.os,
                    target.arch,
                    command_path.display()
                ));
            }
        }
    }

    for runtime_id in &lock.required.runtime_ids {
        for target in &runtime_targets {
            let runtime_component = find_required_component(&lock, "runtime", runtime_id, target);
            let managed_source_available = required_component_has_managed_source(
                &lock,
                "runtime",
                runtime_id,
                target,
                &allowed_managed_sources,
            );
            if *runtime_id == "avf-linux-guest"
                && managed_source_available
                && runtime_component
                    .map(|component| !avf_helper_metadata_complete(component))
                    .unwrap_or(true)
            {
                failures.push(format!(
                    "runtime lock missing AVF helper metadata: {} ({}/{})",
                    runtime_id, target.os, target.arch
                ));
            }
            let Some(entry) = manifest.runtimes.iter().find(|entry| {
                entry.id == *runtime_id && entry.os == target.os && entry.arch == target.arch
            }) else {
                if !managed_source_available {
                    failures.push(format!(
                        "missing runtime entry: {} ({}/{})",
                        runtime_id, target.os, target.arch
                    ));
                }
                continue;
            };
            let root_path = bundle_dir.join(&entry.root);
            if !root_path.exists() {
                failures.push(format!(
                    "missing runtime root dir: {} ({}/{}) at {}",
                    runtime_id,
                    target.os,
                    target.arch,
                    root_path.display()
                ));
                continue;
            }
            let bin_path = root_path.join(&entry.bin);
            if !bin_path.exists() {
                failures.push(format!(
                    "missing runtime binary file: {} ({}/{}) at {}",
                    runtime_id,
                    target.os,
                    target.arch,
                    bin_path.display()
                ));
            }
            if *runtime_id == "avf-linux-guest" {
                for (helper_name, helper_rel) in avf_helper_names_and_paths() {
                    let helper_path = root_path.join(helper_rel);
                    if !helper_path.exists() {
                        failures.push(format!(
                            "missing AVF runtime helper file: {} {} ({}/{}) at {}",
                            runtime_id,
                            helper_name,
                            target.os,
                            target.arch,
                            helper_path.display()
                        ));
                    }
                }
            }
        }
    }

    for image_id in &lock.required.image_ids {
        for target in &image_targets {
            let managed_source_available = required_component_has_managed_source(
                &lock,
                "image",
                image_id,
                target,
                &allowed_managed_sources,
            );
            let Some(entry) = manifest.images.iter().find(|entry| {
                entry.id == *image_id && entry.os == target.os && entry.arch == target.arch
            }) else {
                if !managed_source_available {
                    failures.push(format!(
                        "missing image entry: {} ({}/{})",
                        image_id, target.os, target.arch
                    ));
                }
                continue;
            };
            let tar_path = bundle_dir.join(&entry.tar);
            if !tar_path.exists() && !managed_source_available {
                failures.push(format!(
                    "missing image tar file: {} ({}/{}) at {}",
                    image_id,
                    target.os,
                    target.arch,
                    tar_path.display()
                ));
            }
        }
    }

    for machine_cache_id in &lock.required.machine_cache_ids {
        for target in &machine_cache_targets {
            if !required_component_has_managed_source(
                &lock,
                "machine_cache",
                machine_cache_id,
                target,
                &allowed_managed_sources,
            ) {
                failures.push(format!(
                    "missing machine-cache managed source: {} ({}/{})",
                    machine_cache_id, target.os, target.arch
                ));
            }
        }
    }

    if !failures.is_empty() {
        anyhow::bail!(
            "desktop parity preflight failed (channel={channel} profile=parity surface={surface}): {}",
            failures.join("; ")
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests;
