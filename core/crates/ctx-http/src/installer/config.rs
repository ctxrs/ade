use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use ctx_lsp::LspManagerConfig;

use super::expected_managed_dependency_version;
use crate::bundled_assets;
use crate::installs::{truncate_for_storage, InstallErrorCode, InstallTarget};

static AGENT_SERVER_CONFIG_MUTATION_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn agent_server_config_mutation_lock() -> &'static Mutex<()> {
    AGENT_SERVER_CONFIG_MUTATION_LOCK.get_or_init(|| Mutex::new(()))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedInstallError {
    pub at: String,
    pub stage: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<InstallErrorCode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedInstallMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<InstallTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_dir_rel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bin_dir_rel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_success_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<ManagedInstallError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentServerCommand {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed: Option<ManagedInstallMetadata>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct AgentServerConfigFile {
    #[serde(default)]
    pub providers: HashMap<String, AgentServerCommand>,
    #[serde(default)]
    pub managed_installs: HashMap<String, ManagedInstallMetadata>,
    #[serde(default)]
    pub managed_provider_targets: HashMap<String, HashMap<String, AgentServerCommand>>,
    #[serde(default)]
    pub managed_install_targets: HashMap<String, HashMap<String, ManagedInstallMetadata>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderRuntimeCommandSource {
    UserOverride,
    ManagedInstall,
    BundledSeed,
}

impl ProviderRuntimeCommandSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UserOverride => "user_override",
            Self::ManagedInstall => "managed_install",
            Self::BundledSeed => "bundled_seed",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProviderRuntimeCommand {
    pub provider_id: String,
    pub command_abs_path: String,
    pub args: Vec<String>,
    pub dependencies: Vec<String>,
    pub source: ProviderRuntimeCommandSource,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct LspServerConfigFile {
    #[serde(default)]
    pub servers: HashMap<String, AgentServerCommand>,
    /// Extra language servers (including non-builtin language ids) plus extension/filename mappings.
    #[serde(default)]
    pub extra_servers: Vec<UserLspServerSpec>,
    #[serde(default)]
    pub managed_installs: HashMap<String, ManagedInstallMetadata>,
}

fn install_target_bucket_key(target: InstallTarget) -> &'static str {
    target.as_str()
}

fn requested_target_or_host(target: Option<InstallTarget>) -> InstallTarget {
    target.unwrap_or(InstallTarget::Host)
}

fn legacy_managed_metadata_matches_target(
    meta: &ManagedInstallMetadata,
    requested_target: Option<InstallTarget>,
) -> bool {
    meta.target.unwrap_or(InstallTarget::Host) == requested_target_or_host(requested_target)
}

fn managed_dependency_target_from_id(entry_id: &str) -> Option<InstallTarget> {
    expected_managed_dependency_version(entry_id)?;
    let suffix = entry_id
        .trim()
        .strip_prefix("runtime-node-")
        .or_else(|| entry_id.trim().strip_prefix("runtime-python-"))?;
    match suffix {
        "host" => Some(InstallTarget::Host),
        "container" => Some(InstallTarget::Container),
        "linux-aarch64" => Some(InstallTarget::LinuxAarch64),
        "linux-x86_64" => Some(InstallTarget::LinuxX8664),
        _ => None,
    }
}

fn infer_legacy_managed_target(entry_id: &str, target: Option<InstallTarget>) -> InstallTarget {
    target
        .or_else(|| managed_dependency_target_from_id(entry_id))
        .unwrap_or(InstallTarget::Host)
}

fn target_bucket_lookup<'a, T>(
    buckets: &'a HashMap<String, HashMap<String, T>>,
    provider_id: &str,
    requested_target: Option<InstallTarget>,
) -> Option<&'a T> {
    let target_key = install_target_bucket_key(requested_target_or_host(requested_target));
    buckets.get(provider_id)?.get(target_key)
}

fn user_override_provider_command(
    cfg: &AgentServerConfigFile,
    provider_id: &str,
) -> Option<AgentServerCommand> {
    let configured = cfg.providers.get(provider_id)?;
    configured.managed.is_none().then(|| configured.clone())
}

pub fn managed_install_metadata_for_target<'a>(
    cfg: &'a AgentServerConfigFile,
    provider_id: &str,
    requested_target: Option<InstallTarget>,
) -> Option<&'a ManagedInstallMetadata> {
    target_bucket_lookup(&cfg.managed_install_targets, provider_id, requested_target)
        .or_else(|| {
            cfg.providers
                .get(provider_id)
                .and_then(|e| e.managed.as_ref())
                .filter(|meta| legacy_managed_metadata_matches_target(meta, requested_target))
        })
        .or_else(|| {
            cfg.managed_installs
                .get(provider_id)
                .filter(|meta| legacy_managed_metadata_matches_target(meta, requested_target))
        })
}

pub fn managed_provider_command_for_target(
    cfg: &AgentServerConfigFile,
    provider_id: &str,
    requested_target: Option<InstallTarget>,
) -> Option<AgentServerCommand> {
    target_bucket_lookup(&cfg.managed_provider_targets, provider_id, requested_target)
        .cloned()
        .or_else(|| {
            cfg.providers
                .get(provider_id)
                .filter(|entry| {
                    entry.managed.as_ref().is_some_and(|meta| {
                        legacy_managed_metadata_matches_target(meta, requested_target)
                    })
                })
                .cloned()
        })
}

pub fn apply_managed_install_details_for_target(
    status: &mut ctx_providers::adapters::ProviderStatus,
    cfg: &AgentServerConfigFile,
    requested_target: Option<InstallTarget>,
) {
    let Some(meta) =
        managed_install_metadata_for_target(cfg, &status.provider_id, requested_target)
    else {
        return;
    };

    if let Some(v) = &meta.version {
        status
            .details
            .insert("managed_version".to_string(), v.clone());
    }
    if let Some(target) = meta.target {
        status
            .details
            .insert("managed_target".to_string(), target.as_str().to_string());
    }
    if let Some(p) = &meta.package {
        status
            .details
            .insert("managed_package".to_string(), p.clone());
    }
    if let Some(d) = &meta.install_dir_rel {
        status
            .details
            .insert("managed_install_dir".to_string(), d.clone());
    }
    if let Some(d) = &meta.bin_dir_rel {
        status
            .details
            .insert("managed_bin_dir".to_string(), d.clone());
    }
    if let Some(ts) = &meta.last_success_at {
        status
            .details
            .insert("managed_last_success_at".to_string(), ts.clone());
    }
    if let Some(err) = &meta.last_error {
        status.details.insert(
            "managed_last_error".to_string(),
            truncate_for_storage(&err.message, 1200),
        );
        status
            .details
            .insert("managed_last_error_at".to_string(), err.at.clone());
        status
            .details
            .insert("managed_last_error_stage".to_string(), err.stage.clone());
    }
}

pub fn apply_managed_install_details(
    status: &mut ctx_providers::adapters::ProviderStatus,
    cfg: &AgentServerConfigFile,
) {
    apply_managed_install_details_for_target(status, cfg, None);
}

pub fn resolve_provider_command(
    cfg: &AgentServerConfigFile,
    provider_id: &str,
) -> Option<AgentServerCommand> {
    if let Some(configured) = user_override_provider_command(cfg, provider_id) {
        return Some(configured.clone());
    }
    if let Some(configured) = managed_provider_command_for_target(cfg, provider_id, None) {
        return Some(configured);
    }
    if let Some(bundled) = bundled_assets::bundled_provider_command(provider_id) {
        return Some(AgentServerCommand {
            command: bundled.command,
            args: bundled.args,
            dependencies: Vec::new(),
            managed: None,
        });
    }
    None
}

fn runtime_command_candidate(
    cfg: &AgentServerConfigFile,
    provider_id: &str,
    requested_target: Option<InstallTarget>,
) -> Result<Option<(AgentServerCommand, ProviderRuntimeCommandSource)>> {
    let allow_bundled_seed = matches!(
        requested_target_or_host(requested_target),
        InstallTarget::Host
    );
    if bundled_only_mode_applies_to_provider(provider_id) {
        if allow_bundled_seed {
            if let Some(bundled) = bundled_assets::bundled_provider_command(provider_id) {
                return Ok(Some((
                    AgentServerCommand {
                        command: bundled.command,
                        args: bundled.args,
                        dependencies: Vec::new(),
                        managed: None,
                    },
                    ProviderRuntimeCommandSource::BundledSeed,
                )));
            }
        } else {
            anyhow::bail!(
                "runtime_command_missing_bundled_target: provider={} target={}",
                provider_id,
                requested_target_or_host(requested_target).as_str()
            );
        }
        anyhow::bail!(
            "runtime_command_missing_bundled: provider={} (set CTX_BUNDLE_DIR and ensure bundled manifest includes provider)",
            provider_id
        );
    }

    if let Some(configured) = user_override_provider_command(cfg, provider_id) {
        return Ok(Some((
            configured,
            ProviderRuntimeCommandSource::UserOverride,
        )));
    }

    if let Some(configured) =
        managed_provider_command_for_target(cfg, provider_id, requested_target)
    {
        return Ok(Some((
            configured,
            ProviderRuntimeCommandSource::ManagedInstall,
        )));
    }
    if allow_bundled_seed {
        if let Some(bundled) = bundled_assets::bundled_provider_command(provider_id) {
            return Ok(Some((
                AgentServerCommand {
                    command: bundled.command,
                    args: bundled.args,
                    dependencies: Vec::new(),
                    managed: None,
                },
                ProviderRuntimeCommandSource::BundledSeed,
            )));
        }
    }
    Ok(None)
}

fn preserve_raw_bundle_command_path(path: &Path) -> Option<PathBuf> {
    let raw_bundle_dir = std::env::var("CTX_BUNDLE_DIR").ok()?;
    let raw_bundle_dir = PathBuf::from(raw_bundle_dir.trim());
    if raw_bundle_dir.as_os_str().is_empty() || !raw_bundle_dir.is_absolute() {
        return None;
    }
    path.starts_with(&raw_bundle_dir)
        .then(|| path.to_path_buf())
}

pub fn resolve_runtime_provider_command_for_target(
    cfg: &AgentServerConfigFile,
    provider_id: &str,
    requested_target: Option<InstallTarget>,
) -> Result<Option<ProviderRuntimeCommand>> {
    let Some((candidate, source)) = runtime_command_candidate(cfg, provider_id, requested_target)?
    else {
        return Ok(None);
    };

    let raw = candidate.command.trim();
    if raw.is_empty() {
        anyhow::bail!(
            "runtime_command_missing: provider={} source={}",
            provider_id,
            source.as_str()
        );
    }
    let path = Path::new(raw);
    if !path.is_absolute() {
        anyhow::bail!(
            "runtime_command_not_absolute: provider={} source={} command={}",
            provider_id,
            source.as_str(),
            raw
        );
    }
    if !path.exists() {
        anyhow::bail!(
            "runtime_command_not_found: provider={} source={} command={}",
            provider_id,
            source.as_str(),
            raw
        );
    }
    let command_abs_path = preserve_raw_bundle_command_path(path)
        .unwrap_or_else(|| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()))
        .to_string_lossy()
        .to_string();

    Ok(Some(ProviderRuntimeCommand {
        provider_id: provider_id.to_string(),
        command_abs_path,
        args: candidate.args,
        dependencies: candidate.dependencies,
        source,
    }))
}

pub fn resolve_runtime_provider_command(
    cfg: &AgentServerConfigFile,
    provider_id: &str,
) -> Result<Option<ProviderRuntimeCommand>> {
    resolve_runtime_provider_command_for_target(cfg, provider_id, None)
}

fn env_flag_truthy(var_name: &str) -> bool {
    match std::env::var(var_name) {
        Ok(value) => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => false,
    }
}

fn bundled_only_mode_applies_to_provider(provider_id: &str) -> bool {
    if !env_flag_truthy("CTX_E2E_BUNDLED_ONLY") {
        return false;
    }
    let raw = match std::env::var("CTX_E2E_BUNDLED_ONLY_PROVIDERS") {
        Ok(value) => value,
        Err(_) => return true,
    };
    let providers: Vec<&str> = raw
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .collect();
    if providers.is_empty() {
        return true;
    }
    providers.contains(&provider_id)
}

fn is_legacy_bundle_path(path: &str) -> bool {
    let normalized = path.trim().replace('\\', "/").to_ascii_lowercase();
    normalized.contains("/contents/resources/bundles/")
        || normalized.contains("/src-tauri/bundles/")
        || normalized.contains("/bundles/providers/")
}

fn is_legacy_bundle_rel(path: &str) -> bool {
    let normalized = path.trim().replace('\\', "/").to_ascii_lowercase();
    normalized.starts_with("bundles/")
        || normalized.starts_with("./bundles/")
        || normalized.contains("/bundles/")
}

fn migrate_agent_server_config(cfg: &mut AgentServerConfigFile) -> bool {
    let mut changed = false;
    let mut drop_provider_entries = Vec::new();
    let mut drop_managed_entries = Vec::new();

    for (provider_id, command) in cfg.providers.iter_mut() {
        let Some(existing) = command.managed.as_ref() else {
            continue;
        };
        let target = infer_legacy_managed_target(provider_id, existing.target);
        if command.managed.as_ref().and_then(|managed| managed.target) != Some(target) {
            if let Some(managed) = command.managed.as_mut() {
                managed.target = Some(target);
            }
            changed = true;
        }
        let managed = command.managed.clone().expect("managed metadata");
        let has_legacy_rel = managed
            .install_dir_rel
            .as_deref()
            .map(is_legacy_bundle_rel)
            .unwrap_or(false)
            || managed
                .bin_dir_rel
                .as_deref()
                .map(is_legacy_bundle_rel)
                .unwrap_or(false);

        if is_legacy_bundle_path(&command.command) || has_legacy_rel {
            drop_provider_entries.push(provider_id.clone());
            drop_managed_entries.push(provider_id.clone());
            continue;
        }
        let mut command_clone = command.clone();
        command_clone.managed = Some(managed.clone());

        let target_key = install_target_bucket_key(target);
        let provider_targets = cfg
            .managed_provider_targets
            .entry(provider_id.clone())
            .or_default();
        if !provider_targets.contains_key(target_key) {
            provider_targets.insert(target_key.to_string(), command_clone);
        }
        let install_targets = cfg
            .managed_install_targets
            .entry(provider_id.clone())
            .or_default();
        if !install_targets.contains_key(target_key) {
            install_targets.insert(target_key.to_string(), managed.clone());
        }
        drop_provider_entries.push(provider_id.clone());
        changed = true;
    }

    for (provider_id, managed) in cfg.managed_installs.iter_mut() {
        let target = infer_legacy_managed_target(provider_id, managed.target);
        if managed.target != Some(target) {
            managed.target = Some(target);
            changed = true;
        }
        if expected_managed_dependency_version(provider_id).is_some() {
            continue;
        }
        let has_legacy_rel = managed
            .install_dir_rel
            .as_deref()
            .map(is_legacy_bundle_rel)
            .unwrap_or(false)
            || managed
                .bin_dir_rel
                .as_deref()
                .map(is_legacy_bundle_rel)
                .unwrap_or(false);
        if has_legacy_rel {
            drop_managed_entries.push(provider_id.clone());
            continue;
        }

        let target_key = install_target_bucket_key(target);
        let install_targets = cfg
            .managed_install_targets
            .entry(provider_id.clone())
            .or_default();
        if !install_targets.contains_key(target_key) {
            install_targets.insert(target_key.to_string(), managed.clone());
        }
        drop_managed_entries.push(provider_id.clone());
        changed = true;
    }

    if !drop_provider_entries.is_empty() {
        for provider_id in drop_provider_entries {
            cfg.providers.remove(&provider_id);
        }
        changed = true;
    }
    if !drop_managed_entries.is_empty() {
        drop_managed_entries.sort();
        drop_managed_entries.dedup();
        for provider_id in drop_managed_entries {
            cfg.managed_installs.remove(&provider_id);
        }
        changed = true;
    }

    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};
    use tempfile::tempdir;

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn env_lock() -> &'static Mutex<()> {
        ENV_LOCK.get_or_init(|| Mutex::new(()))
    }

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<String>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var(key).ok();
            // SAFETY: Guarded by ENV_LOCK so tests mutate process env serially.
            unsafe { std::env::set_var(key, value) };
            Self { key, previous }
        }

        fn unset(key: &'static str) -> Self {
            let previous = std::env::var(key).ok();
            // SAFETY: Guarded by ENV_LOCK so tests mutate process env serially.
            unsafe { std::env::remove_var(key) };
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => {
                    // SAFETY: Guarded by ENV_LOCK so tests mutate process env serially.
                    unsafe { std::env::set_var(self.key, value) };
                }
                None => {
                    // SAFETY: Guarded by ENV_LOCK so tests mutate process env serially.
                    unsafe { std::env::remove_var(self.key) };
                }
            }
        }
    }

    #[test]
    fn bundled_only_mode_errors_when_bundled_command_missing() {
        let _guard = env_lock().lock().expect("lock env");
        let temp = tempdir().expect("tempdir");
        let _bundle_dir = EnvVarGuard::set("CTX_BUNDLE_DIR", &temp.path().to_string_lossy());
        let _strict = EnvVarGuard::set("CTX_E2E_BUNDLED_ONLY", "1");
        let _providers = EnvVarGuard::set("CTX_E2E_BUNDLED_ONLY_PROVIDERS", "qwen");

        let mut cfg = AgentServerConfigFile::default();
        cfg.providers.insert(
            "qwen".to_string(),
            AgentServerCommand {
                command: "/tmp/non-bundled-qwen".to_string(),
                args: Vec::new(),
                dependencies: Vec::new(),
                managed: Some(ManagedInstallMetadata {
                    package: Some("qwen-managed".to_string()),
                    version: Some("1.0.0".to_string()),
                    target: None,
                    install_dir_rel: None,
                    bin_dir_rel: None,
                    last_success_at: None,
                    last_error: None,
                }),
            },
        );

        let err = resolve_runtime_provider_command(&cfg, "qwen").expect_err("should fail");
        assert!(err
            .to_string()
            .contains("runtime_command_missing_bundled: provider=qwen"));
    }

    #[test]
    fn bundled_only_provider_scope_defaults_to_all_when_empty() {
        let _guard = env_lock().lock().expect("lock env");
        let _bundle_dir = EnvVarGuard::unset("CTX_BUNDLE_DIR");
        let _strict = EnvVarGuard::set("CTX_E2E_BUNDLED_ONLY", "1");
        let _providers = EnvVarGuard::set("CTX_E2E_BUNDLED_ONLY_PROVIDERS", " , ");
        assert!(bundled_only_mode_applies_to_provider("codex"));
        assert!(bundled_only_mode_applies_to_provider("acp-crp-bridge"));
    }

    #[test]
    fn bundled_only_mode_can_be_disabled() {
        let _guard = env_lock().lock().expect("lock env");
        let _bundle_dir = EnvVarGuard::unset("CTX_BUNDLE_DIR");
        let _strict = EnvVarGuard::unset("CTX_E2E_BUNDLED_ONLY");
        let _providers = EnvVarGuard::unset("CTX_E2E_BUNDLED_ONLY_PROVIDERS");
        assert!(!bundled_only_mode_applies_to_provider("codex"));
    }

    #[test]
    fn bundle_dir_alone_does_not_force_bundled_only_mode() {
        let _guard = env_lock().lock().expect("lock env");
        let temp = tempdir().expect("tempdir");
        let _bundle_dir = EnvVarGuard::set("CTX_BUNDLE_DIR", &temp.path().to_string_lossy());
        let _strict = EnvVarGuard::unset("CTX_E2E_BUNDLED_ONLY");
        let _providers = EnvVarGuard::unset("CTX_E2E_BUNDLED_ONLY_PROVIDERS");
        assert!(!bundled_only_mode_applies_to_provider("codex"));
    }

    #[cfg(unix)]
    #[test]
    fn resolve_runtime_provider_command_preserves_raw_bundle_symlink_paths() {
        use std::os::unix::fs::symlink;

        let _guard = env_lock().lock().expect("lock env");
        let temp = tempdir().expect("tempdir");
        let bundle_target = temp.path().join("bundle-target");
        let bundle_link = temp.path().join("bundle-link");
        let target_command = bundle_target.join("providers/codex/macos/aarch64/codex-crp");
        let raw_command = bundle_link.join("providers/codex/macos/aarch64/codex-crp");
        std::fs::create_dir_all(target_command.parent().expect("parent")).expect("mkdir");
        std::fs::write(&target_command, b"ok").expect("write command");
        symlink(&bundle_target, &bundle_link).expect("symlink bundle");

        let _bundle_dir = EnvVarGuard::set("CTX_BUNDLE_DIR", &bundle_link.to_string_lossy());
        let _strict = EnvVarGuard::unset("CTX_E2E_BUNDLED_ONLY");
        let _providers = EnvVarGuard::unset("CTX_E2E_BUNDLED_ONLY_PROVIDERS");

        let mut cfg = AgentServerConfigFile::default();
        cfg.providers.insert(
            "codex".to_string(),
            AgentServerCommand {
                command: raw_command.to_string_lossy().to_string(),
                args: Vec::new(),
                dependencies: Vec::new(),
                managed: Some(ManagedInstallMetadata {
                    package: Some("@openai/codex".to_string()),
                    version: Some("1.0.0".to_string()),
                    target: Some(InstallTarget::Container),
                    install_dir_rel: None,
                    bin_dir_rel: None,
                    last_success_at: None,
                    last_error: None,
                }),
            },
        );

        let resolved = resolve_runtime_provider_command_for_target(
            &cfg,
            "codex",
            Some(InstallTarget::Container),
        )
        .expect("resolve runtime command")
        .expect("runtime command");
        assert_eq!(resolved.command_abs_path, raw_command.to_string_lossy());
        assert_ne!(
            std::fs::canonicalize(&raw_command)
                .expect("canonicalize raw command")
                .to_string_lossy(),
            resolved.command_abs_path
        );
    }

    #[test]
    fn migration_moves_legacy_managed_provider_entries_into_target_buckets() {
        let mut cfg = AgentServerConfigFile::default();
        cfg.providers.insert(
            "codex".to_string(),
            AgentServerCommand {
                command: "/tmp/codex-host".to_string(),
                args: Vec::new(),
                dependencies: vec!["runtime-node-host".to_string()],
                managed: Some(ManagedInstallMetadata {
                    package: Some("@openai/codex".to_string()),
                    version: Some("0.2.54".to_string()),
                    target: None,
                    install_dir_rel: Some("providers/agent-servers/codex/0.2.54".to_string()),
                    bin_dir_rel: Some("providers/agent-servers/codex/0.2.54/bin".to_string()),
                    last_success_at: None,
                    last_error: None,
                }),
            },
        );

        assert!(migrate_agent_server_config(&mut cfg));
        assert!(!cfg.providers.contains_key("codex"));
        assert!(!cfg.managed_installs.contains_key("codex"));
        assert_eq!(
            cfg.managed_install_targets
                .get("codex")
                .and_then(|targets| targets.get("host"))
                .and_then(|meta| meta.target),
            Some(InstallTarget::Host)
        );
        assert_eq!(
            cfg.managed_provider_targets
                .get("codex")
                .and_then(|targets| targets.get("host"))
                .map(|command| command.command.as_str()),
            Some("/tmp/codex-host")
        );
    }

    #[test]
    fn migration_preserves_runtime_dependency_entries_and_infers_target_from_id() {
        let mut cfg = AgentServerConfigFile::default();
        cfg.managed_installs.insert(
            "runtime-node-container".to_string(),
            ManagedInstallMetadata {
                package: Some("node-runtime".to_string()),
                version: Some("24.14.0".to_string()),
                target: None,
                install_dir_rel: Some("providers/runtimes/node/container".to_string()),
                bin_dir_rel: Some("providers/runtimes/node/container/bin".to_string()),
                last_success_at: None,
                last_error: None,
            },
        );

        assert!(migrate_agent_server_config(&mut cfg));
        assert_eq!(
            cfg.managed_installs
                .get("runtime-node-container")
                .and_then(|meta| meta.target),
            Some(InstallTarget::Container)
        );
        assert!(!cfg
            .managed_install_targets
            .contains_key("runtime-node-container"));
    }

    #[test]
    fn resolve_runtime_provider_command_for_target_prefers_target_bucket() {
        let temp = tempdir().expect("tempdir");
        let host = temp.path().join("codex-host");
        let container = temp.path().join("codex-container");
        std::fs::write(&host, b"host").expect("write host runtime");
        std::fs::write(&container, b"container").expect("write container runtime");

        let mut cfg = AgentServerConfigFile::default();
        cfg.managed_provider_targets.insert(
            "codex".to_string(),
            HashMap::from([
                (
                    "host".to_string(),
                    AgentServerCommand {
                        command: host.to_string_lossy().to_string(),
                        args: vec!["--host".to_string()],
                        dependencies: vec!["runtime-node-host".to_string()],
                        managed: Some(ManagedInstallMetadata {
                            package: Some("@openai/codex".to_string()),
                            version: Some("1.0.0".to_string()),
                            target: Some(InstallTarget::Host),
                            install_dir_rel: None,
                            bin_dir_rel: None,
                            last_success_at: None,
                            last_error: None,
                        }),
                    },
                ),
                (
                    "container".to_string(),
                    AgentServerCommand {
                        command: container.to_string_lossy().to_string(),
                        args: vec!["--container".to_string()],
                        dependencies: vec!["runtime-node-container".to_string()],
                        managed: Some(ManagedInstallMetadata {
                            package: Some("@openai/codex".to_string()),
                            version: Some("1.0.0".to_string()),
                            target: Some(InstallTarget::Container),
                            install_dir_rel: None,
                            bin_dir_rel: None,
                            last_success_at: None,
                            last_error: None,
                        }),
                    },
                ),
            ]),
        );

        let host_resolved =
            resolve_runtime_provider_command_for_target(&cfg, "codex", Some(InstallTarget::Host))
                .expect("resolve host")
                .expect("host runtime");
        assert_eq!(
            host_resolved.command_abs_path,
            std::fs::canonicalize(&host)
                .expect("canonicalize host runtime")
                .to_string_lossy()
        );
        assert_eq!(host_resolved.args, vec!["--host".to_string()]);

        let container_resolved = resolve_runtime_provider_command_for_target(
            &cfg,
            "codex",
            Some(InstallTarget::Container),
        )
        .expect("resolve container")
        .expect("container runtime");
        assert_eq!(
            container_resolved.command_abs_path,
            std::fs::canonicalize(&container)
                .expect("canonicalize container runtime")
                .to_string_lossy()
        );
        assert_eq!(container_resolved.args, vec!["--container".to_string()]);
    }

    #[test]
    fn resolve_runtime_provider_command_for_target_does_not_use_bundled_seed_for_container() {
        let _guard = env_lock().lock().expect("lock env");
        let temp = tempdir().expect("tempdir");
        let bundle_dir = temp.path().join("bundle");
        let bundle_bin = bundle_dir.join("bin");
        let bundle_cmd = bundle_bin.join("acp-crp-bridge");
        std::fs::create_dir_all(&bundle_bin).expect("mkdir bundle bin");
        std::fs::write(&bundle_cmd, b"bridge").expect("write bundle bridge");
        std::fs::write(
            bundle_dir.join("manifest.json"),
            format!(
                r#"{{
  "version": 1,
  "providers": [
    {{
      "id": "acp-crp-bridge",
      "protocol": "crp",
      "version": "0.1.0",
      "os": "{}",
      "arch": "{}",
      "command": "acp-crp-bridge",
      "args": [],
      "sha256": "deadbeef"
    }}
  ]
}}"#,
                std::env::consts::OS,
                std::env::consts::ARCH
            ),
        )
        .expect("write bundle manifest");
        let _bundle_dir = EnvVarGuard::set("CTX_BUNDLE_DIR", &bundle_dir.to_string_lossy());
        let _bundle_manifest = EnvVarGuard::unset("CTX_BUNDLE_MANIFEST");
        let _strict = EnvVarGuard::unset("CTX_E2E_BUNDLED_ONLY");
        let _providers = EnvVarGuard::unset("CTX_E2E_BUNDLED_ONLY_PROVIDERS");

        let resolved = resolve_runtime_provider_command_for_target(
            &AgentServerConfigFile::default(),
            "acp-crp-bridge",
            Some(InstallTarget::Container),
        )
        .expect("resolve container target");

        assert!(
            resolved.is_none(),
            "container target must not reuse host bundled provider commands"
        );
    }

    #[test]
    fn migration_drops_legacy_bundled_provider_commands() {
        let mut cfg = AgentServerConfigFile::default();
        cfg.providers.insert(
            "codex".to_string(),
            AgentServerCommand {
                command:
                    "/Applications/ctx.app/Contents/Resources/bundles/providers/codex/macos/aarch64/codex"
                        .to_string(),
                args: Vec::new(),
                dependencies: Vec::new(),
                managed: Some(ManagedInstallMetadata {
                    package: Some("@openai/codex".to_string()),
                    version: Some("0.2.54".to_string()),
                    target: None,
                    install_dir_rel: Some("bundles/providers/codex/macos/aarch64".to_string()),
                    bin_dir_rel: None,
                    last_success_at: None,
                    last_error: None,
                }),
            },
        );
        cfg.managed_installs.insert(
            "codex".to_string(),
            ManagedInstallMetadata {
                package: Some("@openai/codex".to_string()),
                version: Some("0.2.54".to_string()),
                target: None,
                install_dir_rel: Some("bundles/providers/codex/macos/aarch64".to_string()),
                bin_dir_rel: None,
                last_success_at: None,
                last_error: None,
            },
        );

        assert!(migrate_agent_server_config(&mut cfg));
        assert!(!cfg.providers.contains_key("codex"));
        assert!(!cfg.managed_installs.contains_key("codex"));
    }
}

pub fn agent_server_config_path(data_root: &Path) -> PathBuf {
    data_root
        .join("providers")
        .join("agent-servers")
        .join("agent_servers.json")
}

pub async fn load_agent_server_config(data_root: &Path) -> Result<AgentServerConfigFile> {
    let path = agent_server_config_path(data_root);
    if !path.exists() {
        return Ok(AgentServerConfigFile::default());
    }
    let txt = tokio::fs::read_to_string(&path).await?;
    if txt.trim().is_empty() {
        return Ok(AgentServerConfigFile::default());
    }
    let mut cfg: AgentServerConfigFile =
        serde_json::from_str(&txt).context("parsing agent server config")?;
    if migrate_agent_server_config(&mut cfg) {
        if let Err(error) = save_agent_server_config(data_root, &cfg).await {
            tracing::warn!(
                "failed to persist migrated agent server config at {}: {error:#}",
                path.display()
            );
        }
    }
    Ok(cfg)
}

pub async fn mutate_agent_server_config<T, F>(data_root: &Path, mutate: F) -> Result<T>
where
    F: FnOnce(&mut AgentServerConfigFile) -> T,
{
    let _guard = agent_server_config_mutation_lock().lock().await;
    let mut cfg = load_agent_server_config(data_root).await?;
    let result = mutate(&mut cfg);
    save_agent_server_config(data_root, &cfg).await?;
    Ok(result)
}

pub async fn save_agent_server_config(data_root: &Path, cfg: &AgentServerConfigFile) -> Result<()> {
    let path = agent_server_config_path(data_root);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let tmp_path = path.with_file_name(format!(
        "{}.tmp-{}",
        path.file_name().unwrap_or_default().to_string_lossy(),
        nanos
    ));
    tokio::fs::write(&tmp_path, serde_json::to_string_pretty(cfg)?).await?;
    if let Err(err) = tokio::fs::rename(&tmp_path, &path).await {
        let _ = tokio::fs::remove_file(&path).await;
        tokio::fs::rename(&tmp_path, &path).await?;
        if !matches!(err.kind(), std::io::ErrorKind::AlreadyExists) {
            return Err(err.into());
        }
    }
    Ok(())
}

fn lsp_server_config_path(data_root: &Path) -> PathBuf {
    data_root.join("lsp").join("lsp_servers.json")
}

pub async fn load_lsp_server_config(data_root: &Path) -> Result<LspServerConfigFile> {
    let path = lsp_server_config_path(data_root);
    if !path.exists() {
        return Ok(LspServerConfigFile::default());
    }
    let txt = tokio::fs::read_to_string(&path).await?;
    if txt.trim().is_empty() {
        return Ok(LspServerConfigFile::default());
    }
    serde_json::from_str(&txt).context("parsing lsp server config")
}

pub async fn save_lsp_server_config(data_root: &Path, cfg: &LspServerConfigFile) -> Result<()> {
    let path = lsp_server_config_path(data_root);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let tmp_path = path.with_file_name(format!(
        "{}.tmp-{}",
        path.file_name().unwrap_or_default().to_string_lossy(),
        nanos
    ));
    tokio::fs::write(&tmp_path, serde_json::to_string_pretty(cfg)?).await?;
    if let Err(err) = tokio::fs::rename(&tmp_path, &path).await {
        let _ = tokio::fs::remove_file(&path).await;
        tokio::fs::rename(&tmp_path, &path).await?;
        if !matches!(err.kind(), std::io::ErrorKind::AlreadyExists) {
            return Err(err.into());
        }
    }
    Ok(())
}

pub async fn apply_managed_lsp_server_config(
    data_root: &Path,
    cfg: &mut LspManagerConfig,
) -> Result<()> {
    let installed = load_lsp_server_config(data_root).await.unwrap_or_default();
    if let Some(cmd) = installed.servers.get("rust") {
        cfg.rust_command = cmd.command.clone();
        cfg.rust_args = cmd.args.clone();
    }
    if let Some(cmd) = installed.servers.get("typescript") {
        cfg.ts_command = cmd.command.clone();
        cfg.ts_args = cmd.args.clone();
    }
    if let Some(cmd) = installed.servers.get("python") {
        cfg.py_command = cmd.command.clone();
        cfg.py_args = cmd.args.clone();
    }
    if let Some(cmd) = installed.servers.get("go") {
        cfg.go_command = cmd.command.clone();
        cfg.go_args = cmd.args.clone();
    }
    if let Some(cmd) = installed.servers.get("html") {
        cfg.html_command = cmd.command.clone();
        cfg.html_args = cmd.args.clone();
    }
    if let Some(cmd) = installed.servers.get("css") {
        cfg.css_command = cmd.command.clone();
        cfg.css_args = cmd.args.clone();
    }
    if let Some(cmd) = installed.servers.get("json") {
        cfg.json_command = cmd.command.clone();
        cfg.json_args = cmd.args.clone();
    }
    if let Some(cmd) = installed.servers.get("yaml") {
        cfg.yaml_command = cmd.command.clone();
        cfg.yaml_args = cmd.args.clone();
    }
    if let Some(cmd) = installed.servers.get("bash") {
        cfg.bash_command = cmd.command.clone();
        cfg.bash_args = cmd.args.clone();
    }
    if let Some(cmd) = installed.servers.get("dockerfile") {
        cfg.dockerfile_command = cmd.command.clone();
        cfg.dockerfile_args = cmd.args.clone();
    }
    if let Some(cmd) = installed.servers.get("cpp") {
        cfg.clangd_command = cmd.command.clone();
        cfg.clangd_args = cmd.args.clone();
    }
    if let Some(cmd) = installed.servers.get("lua") {
        cfg.lua_command = cmd.command.clone();
        cfg.lua_args = cmd.args.clone();
    }
    if let Some(cmd) = installed.servers.get("toml") {
        cfg.toml_command = cmd.command.clone();
        cfg.toml_args = cmd.args.clone();
    }
    if let Some(cmd) = installed.servers.get("markdown") {
        cfg.markdown_command = cmd.command.clone();
        cfg.markdown_args = cmd.args.clone();
    }

    // Apply extra managed servers with extension/filename mappings.
    for server in installed.extra_servers {
        let language_id = server.language_id.trim().to_string();
        if language_id.is_empty() || server.command.trim().is_empty() {
            continue;
        }

        match language_id.as_str() {
            "rust" => {
                cfg.rust_command = server.command;
                cfg.rust_args = server.args;
            }
            "typescript" | "javascript" => {
                cfg.ts_command = server.command;
                cfg.ts_args = server.args;
            }
            "python" => {
                cfg.py_command = server.command;
                cfg.py_args = server.args;
            }
            "go" => {
                cfg.go_command = server.command;
                cfg.go_args = server.args;
            }
            "html" => {
                cfg.html_command = server.command;
                cfg.html_args = server.args;
            }
            "css" => {
                cfg.css_command = server.command;
                cfg.css_args = server.args;
            }
            "json" => {
                cfg.json_command = server.command;
                cfg.json_args = server.args;
            }
            "yaml" => {
                cfg.yaml_command = server.command;
                cfg.yaml_args = server.args;
            }
            "bash" => {
                cfg.bash_command = server.command;
                cfg.bash_args = server.args;
            }
            "dockerfile" => {
                cfg.dockerfile_command = server.command;
                cfg.dockerfile_args = server.args;
            }
            "cpp" | "c" => {
                cfg.clangd_command = server.command;
                cfg.clangd_args = server.args;
            }
            "lua" => {
                cfg.lua_command = server.command;
                cfg.lua_args = server.args;
            }
            "toml" => {
                cfg.toml_command = server.command;
                cfg.toml_args = server.args;
            }
            "markdown" => {
                cfg.markdown_command = server.command;
                cfg.markdown_args = server.args;
            }
            other => {
                cfg.custom_servers
                    .insert(other.to_string(), (server.command, server.args));
            }
        }

        for ext in server.extensions {
            let ext = ext.trim().trim_start_matches('.').to_ascii_lowercase();
            if !ext.is_empty() {
                cfg.custom_extension_map.insert(ext, language_id.clone());
            }
        }
        for name in server.filenames {
            let name = name.trim().to_string();
            if !name.is_empty() {
                cfg.custom_filename_map.insert(name, language_id.clone());
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserLspServerSpec {
    /// Optional stable identifier (used by managed installs / catalog entries).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Language id used for `textDocument/didOpen` (and to key the server).
    pub language_id: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// File extensions (no leading dot) that map to this language server.
    #[serde(default)]
    pub extensions: Vec<String>,
    /// Exact filenames (e.g. "Dockerfile") that map to this language server.
    #[serde(default)]
    pub filenames: Vec<String>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct UserLspConfigFile {
    #[serde(default)]
    pub servers: Vec<UserLspServerSpec>,
}

fn user_lsp_config_path(data_root: &Path) -> PathBuf {
    data_root.join("lsp").join("user_servers.json")
}

pub async fn load_user_lsp_config(data_root: &Path) -> Result<UserLspConfigFile> {
    let path = user_lsp_config_path(data_root);
    if !path.exists() {
        return Ok(UserLspConfigFile::default());
    }
    let txt = tokio::fs::read_to_string(&path).await?;
    serde_json::from_str(&txt).context("parsing user lsp server config")
}

pub async fn apply_user_lsp_server_config(
    data_root: &Path,
    cfg: &mut LspManagerConfig,
) -> Result<()> {
    let user = load_user_lsp_config(data_root).await.unwrap_or_default();
    for server in user.servers {
        let language_id = server.language_id.trim().to_string();
        if language_id.is_empty() || server.command.trim().is_empty() {
            continue;
        }

        // Allow overriding known servers by language id.
        match language_id.as_str() {
            "rust" => {
                cfg.rust_command = server.command;
                cfg.rust_args = server.args;
            }
            "typescript" | "javascript" => {
                cfg.ts_command = server.command;
                cfg.ts_args = server.args;
            }
            "python" => {
                cfg.py_command = server.command;
                cfg.py_args = server.args;
            }
            "go" => {
                cfg.go_command = server.command;
                cfg.go_args = server.args;
            }
            "html" => {
                cfg.html_command = server.command;
                cfg.html_args = server.args;
            }
            "css" => {
                cfg.css_command = server.command;
                cfg.css_args = server.args;
            }
            "json" => {
                cfg.json_command = server.command;
                cfg.json_args = server.args;
            }
            "yaml" => {
                cfg.yaml_command = server.command;
                cfg.yaml_args = server.args;
            }
            "bash" => {
                cfg.bash_command = server.command;
                cfg.bash_args = server.args;
            }
            "dockerfile" => {
                cfg.dockerfile_command = server.command;
                cfg.dockerfile_args = server.args;
            }
            "cpp" | "c" => {
                cfg.clangd_command = server.command;
                cfg.clangd_args = server.args;
            }
            "lua" => {
                cfg.lua_command = server.command;
                cfg.lua_args = server.args;
            }
            "toml" => {
                cfg.toml_command = server.command;
                cfg.toml_args = server.args;
            }
            "markdown" => {
                cfg.markdown_command = server.command;
                cfg.markdown_args = server.args;
            }
            other => {
                cfg.custom_servers
                    .insert(other.to_string(), (server.command, server.args));
            }
        }

        for ext in server.extensions {
            let ext = ext.trim().trim_start_matches('.').to_ascii_lowercase();
            if !ext.is_empty() {
                cfg.custom_extension_map.insert(ext, language_id.clone());
            }
        }
        for name in server.filenames {
            let name = name.trim().to_string();
            if !name.is_empty() {
                cfg.custom_filename_map.insert(name, language_id.clone());
            }
        }
    }
    Ok(())
}
