use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::Utc;
use sha2::Digest;
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::time::timeout;

use crate::bundled_assets;
use crate::container_builder;
use crate::daemon::{self, AppState};
use crate::installs::{
    truncate_for_storage, InstallErrorCode, InstallEventLevel, InstallId, InstallProgressEvent,
    InstallStateKind, InstallTarget,
};
use crate::lsp_catalog::{LspCatalogArchive, LspCatalogInstall};
use crate::provider_install_contract;
use crate::provider_matrix;
use crate::title_generation_local;
use crate::updates;
use ctx_providers::crp::Tier1CrpAdapter;

mod artifacts;
mod config;
mod dependencies;
mod lsp;
mod provider_install;
mod title_generation;
mod toolchains;

pub(crate) use self::artifacts::{
    download_to_file, ensure_executable, extract_zip_to_dir, find_unique_path_ending_with,
    install_agent_server_url_binary, install_url_binary, resolve_command_path,
    resolve_download_resume, run_command_with_timeout,
};
use self::dependencies::{
    install_managed_archive_dependency, install_managed_npm_dependency, map_archive_kind,
    resolve_install_args,
};
use self::provider_install::{
    classify_install_error, emit_install, emit_install_with_code, ensure_install_not_cancelled,
    install_provider_impl, repair_install_dir, run_tracked_provider_install,
};

pub use config::{
    agent_server_config_path, apply_managed_install_details,
    apply_managed_install_details_for_target, apply_managed_lsp_server_config,
    apply_user_lsp_server_config, load_agent_server_config, load_lsp_server_config,
    load_user_lsp_config, managed_install_metadata_for_target, managed_provider_command_for_target,
    mutate_agent_server_config, resolve_provider_command, resolve_provider_login_command,
    resolve_runtime_provider_command, resolve_runtime_provider_command_for_target,
    save_agent_server_config, save_lsp_server_config, AgentServerCommand, AgentServerConfigFile,
    LspServerConfigFile, ManagedInstallError, ManagedInstallMetadata, ProviderRuntimeCommand,
    ProviderRuntimeCommandSource, UserLspConfigFile, UserLspServerSpec,
};
pub use lsp::{install_lsp_catalog_server_with_progress, install_lsp_server_with_progress};
pub use provider_install::refresh_provider_statuses;
pub use title_generation::install_title_generation_local_with_progress;
pub(crate) use toolchains::{
    archive_bin_requires_node_runtime, ensure_node_runtime, ensure_python_pip,
    ensure_python_runtime_versioned, install_dir_for_provider, install_dir_rel,
    node_runtime_dependency_id, node_runtime_dependency_metadata,
    node_runtime_dependency_targets_for_install_target, npm_dependency_matches, npm_install,
    npm_install_one, resolve_node_package_bin, sanitize_npm_package_for_path, venv_exe,
    NodeRuntime,
};

const NODE_VERSION: &str = "24.14.0";
const PYTHON_VERSION: &str = "3.13.12";
const PYTHON_BUILD_TAG: &str = "20260303";

const TYPESCRIPT_LS_VERSION: &str = "5.1.3";
const TYPESCRIPT_VERSION: &str = "5.9.3";
const PYRIGHT_VERSION: &str = "1.1.407";
const VSCODE_LANGSERVERS_EXTRACTED_VERSION: &str = "4.10.0";
const YAML_LANGUAGE_SERVER_VERSION: &str = "1.19.2";
const BASH_LANGUAGE_SERVER_VERSION: &str = "5.6.0";
const DOCKERFILE_LANGUAGE_SERVER_VERSION: &str = "0.15.0";

const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const NPM_INSTALL_TIMEOUT: Duration = Duration::from_secs(12 * 60);
const PIP_INSTALL_TIMEOUT: Duration = Duration::from_secs(12 * 60);
const RETRY_COUNT: u32 = 2;
const RETRY_BACKOFF_BASE_MS: u64 = 750;
const LAST_ERROR_MAX_LEN: usize = 8000;
const INSTALL_EVENT_ERROR_MAX_LEN: usize = 6000;
const INSTALL_REGISTRY_POLL_INTERVAL: Duration = Duration::from_millis(100);

static NODE_RUNTIME_INSTALL_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static PYTHON_RUNTIME_INSTALL_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static PROVIDER_INSTALL_LOCKS: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();
const TITLE_GENERATION_LOCAL_INSTALL_KEY: &str = "title_generation_local";
const MANAGED_PROVIDER_INSTALLS_ENABLED: bool = true;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ManagedPythonRuntimeSpec {
    version: String,
    build_tag: String,
}

pub(crate) fn expected_managed_dependency_version(dependency_id: &str) -> Option<&'static str> {
    let normalized = dependency_id.trim().to_ascii_lowercase();
    if normalized.starts_with("runtime-node-") {
        return Some(NODE_VERSION);
    }
    if normalized.starts_with("runtime-python-") {
        return Some(PYTHON_VERSION);
    }
    None
}

fn node_runtime_install_lock() -> &'static Mutex<()> {
    NODE_RUNTIME_INSTALL_LOCK.get_or_init(|| Mutex::new(()))
}

fn python_runtime_install_lock() -> &'static Mutex<()> {
    PYTHON_RUNTIME_INSTALL_LOCK.get_or_init(|| Mutex::new(()))
}

fn managed_python_runtime_spec(
    python_version: Option<&str>,
    python_build_tag: Option<&str>,
) -> ManagedPythonRuntimeSpec {
    let version = python_version
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(PYTHON_VERSION)
        .to_string();
    let build_tag = python_build_tag
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(PYTHON_BUILD_TAG)
        .to_string();
    ManagedPythonRuntimeSpec { version, build_tag }
}

fn provider_install_locks() -> &'static Mutex<HashMap<String, Arc<Mutex<()>>>> {
    PROVIDER_INSTALL_LOCKS.get_or_init(|| Mutex::new(HashMap::new()))
}

async fn acquire_provider_install_lock(
    provider_id: &str,
    target: InstallTarget,
) -> tokio::sync::OwnedMutexGuard<()> {
    let key = format!("{provider_id}@{}", target.as_str());
    let lock = {
        let mut locks = provider_install_locks().lock().await;
        locks
            .entry(key)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    };
    lock.lock_owned().await
}

fn managed_provider_runtime_command(
    data_root: &Path,
    provider_id: &str,
    managed_cmd: AgentServerCommand,
    bridge_cmd: Option<&AgentServerCommand>,
) -> Result<AgentServerCommand> {
    if !daemon::is_acp_provider_id(provider_id) {
        return Ok(managed_cmd);
    }

    let bridge_cmd = bridge_cmd.ok_or_else(|| {
        anyhow::anyhow!(
            "ACP bridge runtime is not configured or invalid for provider '{provider_id}'"
        )
    })?;
    let acp_cmd = daemon::normalize_acp_provider_command(data_root, provider_id, managed_cmd)?;
    Ok(daemon::acp_bridge_command(bridge_cmd, acp_cmd))
}

pub async fn install_provider(state: &AppState, provider_id: &str) -> Result<()> {
    install_provider_impl(state, provider_id, InstallTarget::Host, None).await
}

pub fn is_supported_managed_provider(
    matrix: &provider_matrix::ProviderMatrix,
    provider_id: &str,
) -> bool {
    if !MANAGED_PROVIDER_INSTALLS_ENABLED {
        return false;
    }
    provider_matrix::is_managed_supported(matrix, provider_id)
}

pub fn parse_install_target(raw: Option<&str>) -> Result<InstallTarget> {
    let Some(raw) = raw else {
        return Ok(InstallTarget::Host);
    };
    let normalized = raw.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "" | "host" => Ok(InstallTarget::Host),
        "container" => Ok(InstallTarget::Container),
        "linux-aarch64" => Ok(InstallTarget::LinuxAarch64),
        "linux-x86_64" => Ok(InstallTarget::LinuxX8664),
        other => anyhow::bail!(
            "invalid install target '{}'; expected host, container, linux-aarch64, or linux-x86_64",
            other
        ),
    }
}

fn dependency_target_compatible_with_context(
    dependency_target: Option<InstallTarget>,
    container_exec: bool,
    host_os: &str,
    host_arch: &str,
) -> bool {
    match dependency_target.unwrap_or(InstallTarget::Host) {
        InstallTarget::Host => !container_exec,
        InstallTarget::Container => {
            if container_exec {
                matches!(host_arch, "x86_64" | "aarch64")
            } else {
                host_os == "linux" && matches!(host_arch, "x86_64" | "aarch64")
            }
        }
        InstallTarget::LinuxAarch64 => {
            host_arch == "aarch64" && (container_exec || host_os == "linux")
        }
        InstallTarget::LinuxX8664 => {
            host_arch == "x86_64" && (container_exec || host_os == "linux")
        }
    }
}

fn provider_env_targets_linux_sandbox(provider_env: &HashMap<String, String>) -> bool {
    provider_env
        .get(crate::harness_runtime::CTX_HARNESS_LINUX_SANDBOX_ENV)
        .is_some_and(|value| value == "1")
        || provider_env.contains_key("CTX_HARNESS_CONTAINER_ID")
}

fn prepend_bundled_seed_node_bin_dir(
    bin_dirs: &mut Vec<PathBuf>,
    runtime_cmd: &ProviderRuntimeCommand,
    bundled_node_runtime: Option<bundled_assets::BundledRuntimePaths>,
) {
    if runtime_cmd.source != ProviderRuntimeCommandSource::BundledSeed {
        return;
    }
    let runtime_cmd_path = Path::new(&runtime_cmd.command_abs_path);
    if !archive_bin_requires_node_runtime(&runtime_cmd.command_abs_path, runtime_cmd_path) {
        return;
    }
    let Some(node_bin_dir) = bundled_node_runtime
        .as_ref()
        .and_then(|runtime| runtime.bin.parent())
        .map(Path::to_path_buf)
    else {
        return;
    };
    if !bin_dirs.contains(&node_bin_dir) {
        bin_dirs.push(node_bin_dir);
    }
}

pub(crate) fn prepend_runtime_bin_dirs_to_provider_path_for_target(
    provider_env: &mut HashMap<String, String>,
    cfg: &AgentServerConfigFile,
    runtime_provider_id: &str,
    data_root: &Path,
    requested_target: Option<InstallTarget>,
) {
    let mut bin_dirs: Vec<PathBuf> = Vec::new();
    let container_exec = provider_env_targets_linux_sandbox(provider_env);
    if let Ok(Some(runtime_cmd)) =
        resolve_runtime_provider_command_for_target(cfg, runtime_provider_id, requested_target)
    {
        let runtime_cmd_path = Path::new(&runtime_cmd.command_abs_path);
        if let Some(parent) = runtime_cmd_path.parent() {
            let parent_dir = parent.to_path_buf();
            if !bin_dirs.contains(&parent_dir) {
                bin_dirs.push(parent_dir);
            }
        }
        for dep in &runtime_cmd.dependencies {
            if let Some(meta) = cfg.managed_installs.get(dep) {
                if !dependency_target_compatible_with_context(
                    meta.target,
                    container_exec,
                    std::env::consts::OS,
                    std::env::consts::ARCH,
                ) {
                    continue;
                }
                if let Some(rel) = meta.bin_dir_rel.as_ref() {
                    let dep_dir = data_root.join(rel);
                    if !bin_dirs.contains(&dep_dir) {
                        bin_dirs.push(dep_dir);
                    }
                }
            }
        }
        prepend_bundled_seed_node_bin_dir(
            &mut bin_dirs,
            &runtime_cmd,
            bundled_assets::bundled_node_runtime(),
        );
    }
    if bin_dirs.is_empty() {
        return;
    }

    let mut path_parts: Vec<PathBuf> = bin_dirs;
    if let Some(current) = provider_env
        .get("PATH")
        .cloned()
        .or_else(|| std::env::var("PATH").ok())
    {
        path_parts.extend(std::env::split_paths(std::ffi::OsStr::new(&current)));
    }
    if let Ok(joined) = std::env::join_paths(path_parts) {
        provider_env.insert("PATH".to_string(), joined.to_string_lossy().to_string());
    }
}

#[cfg(test)]
pub(crate) fn prepend_runtime_bin_dirs_to_provider_path(
    provider_env: &mut HashMap<String, String>,
    cfg: &AgentServerConfigFile,
    runtime_provider_id: &str,
    data_root: &Path,
) {
    prepend_runtime_bin_dirs_to_provider_path_for_target(
        provider_env,
        cfg,
        runtime_provider_id,
        data_root,
        None,
    )
}

pub fn resolve_matrix_target_key(target: InstallTarget) -> Result<&'static str> {
    match target {
        InstallTarget::Host => host_target_key(),
        InstallTarget::Container => container_target_key(),
        InstallTarget::LinuxAarch64 => Ok("linux-aarch64"),
        InstallTarget::LinuxX8664 => Ok("linux-x86_64"),
    }
}

pub fn is_supported_managed_provider_for_target(
    matrix: &provider_matrix::ProviderMatrix,
    provider_id: &str,
    target: InstallTarget,
) -> bool {
    if !is_supported_managed_provider(matrix, provider_id) {
        return false;
    }
    let Some(entry) = provider_matrix::get_entry(matrix, provider_id) else {
        return false;
    };
    let Some(install) = entry.managed_install.as_ref() else {
        return false;
    };
    let target_key = match resolve_matrix_target_key(target) {
        Ok(key) => key,
        Err(_) => return false,
    };

    // Archive installs are target-specific. npm/python installs are currently
    // supported for host and container targets.
    match install {
        provider_matrix::ProviderInstall::Archive { targets, .. } => {
            targets.contains_key(target_key)
        }
        provider_matrix::ProviderInstall::Npm { .. }
        | provider_matrix::ProviderInstall::Python { .. } => {
            matches!(target, InstallTarget::Host | InstallTarget::Container)
        }
    }
}

pub fn managed_install_download_size_bytes(
    matrix: &provider_matrix::ProviderMatrix,
    provider_id: &str,
    target: InstallTarget,
) -> Option<u64> {
    let entry = provider_matrix::get_entry(matrix, provider_id)?;
    let install = entry.managed_install.as_ref()?;
    let target_key = resolve_matrix_target_key(target).ok()?;

    let mut total: u64 = 0;
    let mut any = false;

    match install {
        provider_matrix::ProviderInstall::Archive { targets, .. } => {
            let target_entry = targets.get(target_key)?;
            let size = target_entry.size_bytes?;
            total = total.saturating_add(size);
            any = true;
        }
        provider_matrix::ProviderInstall::Npm { .. }
        | provider_matrix::ProviderInstall::Python { .. } => {}
    }

    for dependency in &entry.dependencies {
        match &dependency.install {
            provider_matrix::DependencyInstall::Archive { targets, .. } => {
                let target_entry = targets.get(target_key)?;
                let size = target_entry.size_bytes?;
                total = total.saturating_add(size);
                any = true;
            }
            provider_matrix::DependencyInstall::Npm { .. } => {}
        }
    }

    if any {
        Some(total)
    } else {
        None
    }
}

pub(crate) fn apply_install_target_status(
    status: &mut ctx_providers::adapters::ProviderStatus,
    target: InstallTarget,
) {
    let requested_target = target.as_str();
    if matches!(target, InstallTarget::Host) {
        let Some(managed_target) = status.details.get("managed_target").cloned() else {
            return;
        };
        if managed_target == requested_target {
            status.details.remove("target_mismatch");
            status.details.remove("target_unverified");
            status.details.remove("target_mismatch_reason");
            return;
        }

        status.installed = false;
        status.health = ctx_providers::adapters::ProviderHealth::Missing;
        status
            .details
            .insert("target_mismatch".to_string(), "true".to_string());
        status.details.remove("target_unverified");
        status.details.insert(
            "target_mismatch_reason".to_string(),
            format!(
                "provider managed install target is '{managed_target}', requested '{requested_target}'"
            ),
        );
        let diagnostic = format!(
            "provider is installed for target '{managed_target}', not '{requested_target}'"
        );
        if !status.diagnostics.iter().any(|msg| msg == &diagnostic) {
            status.diagnostics.insert(0, diagnostic);
        }
        return;
    }

    let Some(managed_target) = status.details.get("managed_target").cloned() else {
        if !status.installed {
            return;
        }
        status.installed = false;
        status.health = ctx_providers::adapters::ProviderHealth::Missing;
        status.details.remove("target_mismatch");
        status
            .details
            .insert("target_unverified".to_string(), "true".to_string());
        status.details.insert(
            "target_mismatch_reason".to_string(),
            format!(
                "provider status was detected from the host environment and cannot verify target '{requested_target}'"
            ),
        );
        let diagnostic = format!(
            "provider status reflects the host environment and does not verify target '{requested_target}'"
        );
        if !status.diagnostics.iter().any(|msg| msg == &diagnostic) {
            status.diagnostics.insert(0, diagnostic);
        }
        return;
    };
    if managed_target == requested_target {
        status.details.remove("target_mismatch");
        status.details.remove("target_unverified");
        status.details.remove("target_mismatch_reason");
        return;
    }

    status.installed = false;
    status.health = ctx_providers::adapters::ProviderHealth::Missing;
    status
        .details
        .insert("target_mismatch".to_string(), "true".to_string());
    status.details.remove("target_unverified");
    status.details.insert(
        "target_mismatch_reason".to_string(),
        format!(
            "provider managed install target is '{managed_target}', requested '{requested_target}'"
        ),
    );
    let diagnostic =
        format!("provider is installed for target '{managed_target}', not '{requested_target}'");
    if !status.diagnostics.iter().any(|msg| msg == &diagnostic) {
        status.diagnostics.insert(0, diagnostic);
    }
}

fn validate_post_install_status(
    status: &ctx_providers::adapters::ProviderStatus,
    provider_id: &str,
    target: InstallTarget,
) -> Result<()> {
    if let Some(managed_target) = status.details.get("managed_target") {
        if managed_target != target.as_str() {
            anyhow::bail!(
                "install completed but provider '{}' resolved to managed target '{}' (expected '{}')",
                provider_id,
                managed_target,
                target.as_str()
            );
        }
    }
    if !status.installed || !matches!(status.health, ctx_providers::adapters::ProviderHealth::Ok) {
        anyhow::bail!(
            "install completed but provider is not healthy: {}",
            status.diagnostics.join("; ")
        );
    }
    Ok(())
}

pub fn is_supported_managed_lsp_server(server_id: &str) -> bool {
    matches!(
        server_id,
        "typescript" | "python" | "html" | "css" | "json" | "yaml" | "bash" | "dockerfile"
    )
}

pub async fn install_provider_with_progress(
    state: std::sync::Arc<AppState>,
    install_id: InstallId,
    provider_id: String,
    target: InstallTarget,
) -> Result<()> {
    run_tracked_provider_install(state.as_ref(), install_id, &provider_id, target).await
}

fn catalog_target_key() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "linux-x64",
        ("linux", "aarch64") => "linux-arm64",
        ("macos", "x86_64") => "darwin-x64",
        ("macos", "aarch64") => "darwin-arm64",
        _ => "unknown",
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum AgentServerArchive {
    None,
    TarGz,
    TarBz2,
    Zip,
    Dmg,
}

fn host_target_key() -> Result<&'static str> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    match (os, arch) {
        ("linux", "x86_64") => Ok("linux-x86_64"),
        ("linux", "aarch64") => Ok("linux-aarch64"),
        ("macos", "x86_64") => Ok("darwin-x86_64"),
        ("macos", "aarch64") => Ok("darwin-aarch64"),
        ("windows", "x86_64") => Ok("windows-x86_64"),
        ("windows", "aarch64") => Ok("windows-aarch64"),
        _ => anyhow::bail!("unsupported platform: {os}/{arch}"),
    }
}

fn container_target_key() -> Result<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Ok("linux-x86_64"),
        "aarch64" => Ok("linux-aarch64"),
        other => anyhow::bail!("unsupported container architecture: {other}"),
    }
}

struct ManagedProviderInstall {
    command: String,
    args: Vec<String>,
    meta: ManagedInstallMetadata,
}

struct ManagedDependencyInstall {
    meta: ManagedInstallMetadata,
}

#[allow(clippy::too_many_arguments)]
async fn install_managed_npm_provider(
    state: &AppState,
    install_id: Option<InstallId>,
    provider_id: &str,
    package: &str,
    version: &str,
    script_rel: &str,
    extra_args: Vec<String>,
    target: InstallTarget,
    stage: &mut &'static str,
) -> Result<ManagedProviderInstall> {
    let data_root = state.core.data_root.clone();
    let install_dir = install_dir_for_provider(&data_root, provider_id, version, target);
    let install_dir_rel = install_dir_rel(&data_root, &install_dir);

    *stage = "node";
    let node = ensure_node_runtime(state, install_id, provider_id, &data_root, target)
        .await
        .context("ensuring managed Node runtime")?;

    *stage = "prepare";
    repair_install_dir(install_id, state, provider_id, &install_dir, script_rel)
        .await
        .context("preparing install directory")?;

    let package_spec = format!("{package}@{version}");
    *stage = "npm_install";
    npm_install(
        state,
        install_id,
        provider_id,
        &node,
        &install_dir,
        &package_spec,
        target,
    )
    .await
    .context("running package install")?;

    *stage = "entrypoint";
    let script_path = install_dir.join(script_rel);
    if !script_path.exists() {
        tokio::fs::remove_dir_all(&install_dir).await.ok();
        anyhow::bail!(
            "install completed but entrypoint missing: {}",
            script_path.display()
        );
    }

    let mut args = vec![script_path.to_string_lossy().to_string()];
    args.extend(extra_args);

    let meta = ManagedInstallMetadata {
        package: Some(package.to_string()),
        version: Some(version.to_string()),
        archive_sha256: None,
        target: Some(target),
        install_dir_rel: Some(install_dir_rel),
        bin_dir_rel: None,
        last_success_at: Some(Utc::now().to_rfc3339()),
        last_error: None,
    };

    Ok(ManagedProviderInstall {
        command: node.node_bin.to_string_lossy().to_string(),
        args,
        meta,
    })
}

#[allow(clippy::too_many_arguments)]
async fn install_managed_archive_provider(
    state: &AppState,
    install_id: Option<InstallId>,
    provider_id: &str,
    version: &str,
    url: &str,
    expected_sha256: Option<&str>,
    archive: AgentServerArchive,
    bin_path: &str,
    args: Vec<String>,
    target: InstallTarget,
    stage: &mut &'static str,
) -> Result<ManagedProviderInstall> {
    let bin = install_agent_server_url_binary(
        state,
        install_id,
        provider_id,
        provider_id,
        version,
        url,
        expected_sha256,
        archive,
        bin_path,
        target,
        stage,
    )
    .await
    .context("installing agent server binary")?;

    let install_dir = install_dir_for_provider(&state.core.data_root, provider_id, version, target);
    let meta = ManagedInstallMetadata {
        package: Some(url.to_string()),
        version: Some(version.to_string()),
        archive_sha256: expected_sha256.map(str::to_string),
        target: Some(target),
        install_dir_rel: Some(install_dir_rel(&state.core.data_root, &install_dir)),
        bin_dir_rel: None,
        last_success_at: Some(Utc::now().to_rfc3339()),
        last_error: None,
    };

    Ok(ManagedProviderInstall {
        command: bin.to_string_lossy().to_string(),
        args,
        meta,
    })
}

#[allow(clippy::too_many_arguments)]
async fn install_managed_python_provider(
    state: &AppState,
    install_id: Option<InstallId>,
    provider_id: &str,
    package: &str,
    version: &str,
    entrypoint: &str,
    python_version: Option<&str>,
    python_build_tag: Option<&str>,
    args: Vec<String>,
    target: InstallTarget,
    stage: &mut &'static str,
) -> Result<ManagedProviderInstall> {
    *stage = "python";
    let python_runtime = managed_python_runtime_spec(python_version, python_build_tag);
    let python = ensure_python_runtime_versioned(
        state,
        install_id,
        provider_id,
        &state.core.data_root,
        target,
        &python_runtime.version,
        &python_runtime.build_tag,
    )
    .await
    .context("ensuring managed Python runtime")?
    .python_bin;
    let data_root = state.core.data_root.clone();
    let install_dir = install_dir_for_provider(&data_root, provider_id, version, target);
    let install_dir_rel = install_dir_rel(&data_root, &install_dir);
    let venv_dir = install_dir.join("venv");

    *stage = "prepare";
    emit_install(
        state,
        install_id,
        provider_id,
        InstallEventLevel::Info,
        "prepare",
        format!("Preparing install dir: {}", install_dir.display()),
        None,
        None,
        None,
    )
    .await;

    if install_dir.exists() {
        let expected = venv_exe(&venv_dir, entrypoint, target);
        if !expected.exists() {
            tokio::fs::remove_dir_all(&install_dir).await.ok();
        }
    }
    tokio::fs::create_dir_all(&install_dir)
        .await
        .with_context(|| format!("creating install dir: {}", install_dir.display()))?;

    *stage = "venv";
    emit_install(
        state,
        install_id,
        provider_id,
        InstallEventLevel::Info,
        "venv",
        "Creating virtualenv…".to_string(),
        None,
        None,
        None,
    )
    .await;

    if matches!(target, InstallTarget::Container) {
        container_builder::ensure_builder_ready(&state.core.data_root)
            .await
            .context("ensuring container builder readiness")?;
        let argv = vec![
            python.to_string_lossy().to_string(),
            "-m".to_string(),
            "venv".to_string(),
            venv_dir.to_string_lossy().to_string(),
        ];
        let out = container_builder::run_command(
            &state.core.data_root,
            &install_dir,
            &[],
            &argv,
            Duration::from_secs(5 * 60),
        )
        .await
        .context("creating virtualenv")?;
        if !out.status.success() {
            anyhow::bail!(
                "creating virtualenv failed status={}\nstdout:\n{}\nstderr:\n{}",
                out.status,
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
        }
    } else {
        let mut venv_cmd = Command::new(&python);
        venv_cmd
            .arg("-m")
            .arg("venv")
            .arg(&venv_dir)
            .kill_on_drop(true);
        run_command_with_timeout(venv_cmd, Duration::from_secs(5 * 60))
            .await
            .context("creating virtualenv")?;
    }

    let venv_python = venv_exe(&venv_dir, "python", target);

    if matches!(target, InstallTarget::Container) {
        let argv = vec![
            venv_python.to_string_lossy().to_string(),
            "-m".to_string(),
            "ensurepip".to_string(),
            "--upgrade".to_string(),
        ];
        let out = container_builder::run_command(
            &state.core.data_root,
            &install_dir,
            &[],
            &argv,
            Duration::from_secs(5 * 60),
        )
        .await
        .context("ensuring pip in virtualenv")?;
        if !out.status.success() {
            anyhow::bail!(
                "ensurepip failed status={}\nstdout:\n{}\nstderr:\n{}",
                out.status,
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
        }
    } else {
        ensure_python_pip(&venv_python)
            .await
            .context("ensuring pip in virtualenv")?;
    }

    let package_spec = if package.starts_with("https://") || package.starts_with("http://") {
        package.to_string()
    } else {
        format!("{package}=={version}")
    };

    *stage = "pip_install";
    emit_install(
        state,
        install_id,
        provider_id,
        InstallEventLevel::Info,
        "pip_install",
        format!("Installing {package_spec}…"),
        None,
        None,
        None,
    )
    .await;

    let out = if matches!(target, InstallTarget::Container) {
        let argv = vec![
            venv_python.to_string_lossy().to_string(),
            "-m".to_string(),
            "pip".to_string(),
            "install".to_string(),
            "--disable-pip-version-check".to_string(),
            "--no-input".to_string(),
            package_spec.clone(),
        ];
        let env = vec![("PIP_DISABLE_PIP_VERSION_CHECK".to_string(), "1".to_string())];
        container_builder::run_command(
            &state.core.data_root,
            &install_dir,
            &env,
            &argv,
            PIP_INSTALL_TIMEOUT,
        )
        .await
    } else {
        let mut pip_cmd = Command::new(&venv_python);
        pip_cmd
            .arg("-m")
            .arg("pip")
            .arg("install")
            .arg("--disable-pip-version-check")
            .arg("--no-input")
            .arg(&package_spec)
            .env("PIP_DISABLE_PIP_VERSION_CHECK", "1")
            .kill_on_drop(true);
        run_command_with_timeout(pip_cmd, PIP_INSTALL_TIMEOUT).await
    }
    .context("running pip install")?;
    if !out.status.success() {
        anyhow::bail!(
            "pip install failed ({}) status={}\nstdout:\n{}\nstderr:\n{}",
            package_spec,
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    let exe = venv_exe(&venv_dir, entrypoint, target);
    if !exe.exists() {
        tokio::fs::remove_dir_all(&install_dir).await.ok();
        anyhow::bail!(
            "pip install completed but entrypoint missing: {}",
            exe.display()
        );
    }

    let meta = ManagedInstallMetadata {
        package: Some(package.to_string()),
        version: Some(version.to_string()),
        archive_sha256: None,
        target: Some(target),
        install_dir_rel: Some(install_dir_rel),
        bin_dir_rel: None,
        last_success_at: Some(Utc::now().to_rfc3339()),
        last_error: None,
    };

    Ok(ManagedProviderInstall {
        command: exe.to_string_lossy().to_string(),
        args,
        meta,
    })
}

#[cfg(test)]
mod tests;
