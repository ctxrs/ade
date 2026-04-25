use serde::Deserialize;
use std::collections::{HashMap, HashSet};

use super::RuntimeTarget;

#[derive(Debug, Clone, Deserialize)]
pub(super) struct DesktopBundledAssetsManifest {
    #[allow(dead_code)]
    version: u32,
    #[serde(default)]
    pub(super) providers: Vec<DesktopBundledProvider>,
    #[serde(default)]
    pub(super) runtimes: Vec<DesktopBundledRuntime>,
    #[serde(default)]
    pub(super) images: Vec<DesktopBundledImage>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct DesktopBundledProviderManifest {
    pub(super) version: u32,
    #[serde(default)]
    pub(super) providers: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct DesktopBundledProvider {
    pub(super) id: String,
    pub(super) os: String,
    pub(super) arch: String,
    pub(super) command: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct DesktopBundledRuntime {
    pub(super) id: String,
    pub(super) os: String,
    pub(super) arch: String,
    pub(super) root: String,
    pub(super) bin: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct DesktopBundledImage {
    pub(super) id: String,
    pub(super) os: String,
    pub(super) arch: String,
    pub(super) tar: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct RuntimeLockRequiredTargets {
    #[serde(default)]
    pub(super) provider: Vec<String>,
    #[serde(default)]
    pub(super) runtime: Vec<String>,
    #[serde(default)]
    pub(super) image: Vec<String>,
    #[serde(default)]
    pub(super) machine_cache: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct RuntimeLockRequired {
    #[serde(default)]
    pub(super) provider_ids: Vec<String>,
    #[serde(default)]
    pub(super) runtime_ids: Vec<String>,
    #[serde(default)]
    pub(super) image_ids: Vec<String>,
    #[serde(default)]
    pub(super) machine_cache_ids: Vec<String>,
    pub(super) targets: RuntimeLockRequiredTargets,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct RuntimeLockV2 {
    pub(super) version: u32,
    #[serde(default)]
    profiles: HashMap<String, RuntimeLockProfile>,
    pub(super) required: RuntimeLockRequired,
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
    #[serde(rename = "container-stack", default)]
    container_stack: Option<RuntimeLockComponentHelper>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct RuntimeLockComponent {
    #[serde(default)]
    pub(super) kind: String,
    #[serde(default)]
    pub(super) id: String,
    #[serde(default)]
    pub(super) os: String,
    #[serde(default)]
    pub(super) arch: String,
    #[serde(default)]
    pub(super) variant: Option<String>,
    #[serde(default)]
    sources: Vec<RuntimeLockComponentSource>,
    #[serde(default)]
    helpers: RuntimeLockComponentHelpers,
}

pub(super) fn allowed_source_types_for_profile(
    lock: &RuntimeLockV2,
    profile: &str,
) -> HashSet<String> {
    let mut out = HashSet::new();
    let cfg = lock.profiles.get(profile).or_else(|| lock.profiles.get("parity"));
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
        let Some(uri) = source
            .uri
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return false;
        };
        let Some(sha256) = source
            .sha256
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return false;
        };
        if component.kind == "runtime"
            && component.id == "avf-linux-guest"
            && (!managed_source_is_resolved(uri) || !sha256_is_resolved(sha256))
        {
            return false;
        }
        true
    })
}

pub(super) fn required_component_has_managed_source(
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

pub(super) fn find_required_component<'a>(
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

fn managed_source_is_resolved(uri: &str) -> bool {
    !uri.trim().starts_with("locked://")
}

fn sha256_is_resolved(sha256: &str) -> bool {
    let trimmed = sha256.trim();
    !trimmed.is_empty() && !(trimmed.len() == 64 && trimmed.bytes().all(|byte| byte == b'0'))
}

fn helper_metadata_complete(
    helper: Option<&RuntimeLockComponentHelper>,
    require_resolved: bool,
) -> bool {
    let Some(uri) = helper
        .and_then(|helper| helper.uri.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    let Some(sha256) = helper
        .and_then(|helper| helper.sha256.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    if require_resolved {
        managed_source_is_resolved(uri) && sha256_is_resolved(sha256)
    } else {
        true
    }
}

pub(super) fn avf_helper_names_and_paths() -> [(&'static str, &'static str); 5] {
    [
        ("kernel", "helpers/kernel"),
        ("initrd", "helpers/initrd"),
        ("guest-agent", "helpers/guest-agent"),
        ("egress-proxy", "helpers/egress-proxy"),
        ("container-stack", "helpers/container-stack.tar.gz"),
    ]
}

pub(super) fn avf_helper_metadata_complete(component: &RuntimeLockComponent) -> bool {
    helper_metadata_complete(component.helpers.kernel.as_ref(), true)
        && helper_metadata_complete(component.helpers.initrd.as_ref(), true)
        && helper_metadata_complete(component.helpers.guest_agent.as_ref(), true)
        && helper_metadata_complete(component.helpers.egress_proxy.as_ref(), true)
        && helper_metadata_complete(component.helpers.container_stack.as_ref(), true)
}
