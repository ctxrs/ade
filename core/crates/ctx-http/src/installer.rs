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
mod lsp;
mod provider_install;
mod title_generation;
mod toolchains;

pub(crate) use self::artifacts::{
    download_to_file, ensure_executable, extract_zip_to_dir, find_unique_path_ending_with,
    install_agent_server_url_binary, install_url_binary, resolve_command_path,
    resolve_download_resume, run_command_with_timeout,
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
    resolve_provider_command, resolve_runtime_provider_command,
    resolve_runtime_provider_command_for_target, save_agent_server_config, save_lsp_server_config,
    AgentServerCommand, AgentServerConfigFile, LspServerConfigFile, ManagedInstallError,
    ManagedInstallMetadata, ProviderRuntimeCommand, ProviderRuntimeCommandSource,
    UserLspConfigFile, UserLspServerSpec,
};
pub use lsp::{install_lsp_catalog_server_with_progress, install_lsp_server_with_progress};
pub use provider_install::refresh_provider_statuses;
pub use title_generation::install_title_generation_local_with_progress;
pub(crate) use toolchains::{
    archive_bin_requires_node_runtime, ensure_node_runtime, ensure_python_pip,
    ensure_python_runtime, install_dir_for_provider, install_dir_rel, node_runtime_dependency_id,
    node_runtime_dependency_metadata, node_runtime_dependency_targets_for_install_target,
    npm_dependency_matches, npm_install, npm_install_one, resolve_node_package_bin,
    sanitize_npm_package_for_path, venv_exe, NodeRuntime,
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
    let acp_cmd = daemon::normalize_acp_provider_command(data_root, provider_id, managed_cmd);
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
    let container_exec = provider_env.contains_key("CTX_HARNESS_CONTAINER_ID");
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
    args: Vec<String>,
    target: InstallTarget,
    stage: &mut &'static str,
) -> Result<ManagedProviderInstall> {
    *stage = "python";
    let python = ensure_python_runtime(
        state,
        install_id,
        provider_id,
        &state.core.data_root,
        target,
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

async fn install_managed_npm_dependency(
    state: &AppState,
    install_id: Option<InstallId>,
    provider_id: &str,
    dependency_id: &str,
    package: &str,
    version: &str,
    stage: &mut &'static str,
) -> Result<ManagedDependencyInstall> {
    let data_root = state.core.data_root.clone();
    let install_dir = data_root
        .join("providers")
        .join("agent-servers")
        .join(dependency_id)
        .join(version);
    let install_dir_rel_value = install_dir_rel(&data_root, &install_dir);
    let bin_dir = install_dir.join("node_modules").join(".bin");
    let bin_dir_rel_value = install_dir_rel(&data_root, &bin_dir);

    if install_dir.exists() {
        if npm_dependency_matches(&install_dir, package, version)
            .await
            .unwrap_or(false)
            && bin_dir.exists()
        {
            let meta = ManagedInstallMetadata {
                package: Some(package.to_string()),
                version: Some(version.to_string()),
                target: Some(InstallTarget::Host),
                install_dir_rel: Some(install_dir_rel_value.clone()),
                bin_dir_rel: Some(bin_dir_rel_value.clone()),
                last_success_at: Some(Utc::now().to_rfc3339()),
                last_error: None,
            };
            return Ok(ManagedDependencyInstall { meta });
        }
        tokio::fs::remove_dir_all(&install_dir).await.ok();
    }

    *stage = "dependency_node";
    let node = ensure_node_runtime(
        state,
        install_id,
        provider_id,
        &data_root,
        InstallTarget::Host,
    )
    .await
    .context("ensuring managed Node runtime")?;

    *stage = "dependency_prepare";
    emit_install(
        state,
        install_id,
        provider_id,
        InstallEventLevel::Info,
        "dependency_prepare",
        format!("Preparing dependency {dependency_id}…"),
        None,
        None,
        None,
    )
    .await;
    tokio::fs::create_dir_all(&install_dir)
        .await
        .with_context(|| format!("creating install dir: {}", install_dir.display()))?;

    *stage = "dependency_npm_install";
    npm_install(
        state,
        install_id,
        provider_id,
        &node,
        &install_dir,
        &format!("{package}@{version}"),
        InstallTarget::Host,
    )
    .await
    .context("running package install for dependency")?;

    if !bin_dir.exists() {
        tokio::fs::remove_dir_all(&install_dir).await.ok();
        anyhow::bail!(
            "dependency install completed but bin dir missing: {}",
            bin_dir.display()
        );
    }

    let meta = ManagedInstallMetadata {
        package: Some(package.to_string()),
        version: Some(version.to_string()),
        target: Some(InstallTarget::Host),
        install_dir_rel: Some(install_dir_rel_value),
        bin_dir_rel: Some(bin_dir_rel_value),
        last_success_at: Some(Utc::now().to_rfc3339()),
        last_error: None,
    };

    Ok(ManagedDependencyInstall { meta })
}

#[allow(clippy::too_many_arguments)]
async fn install_managed_archive_dependency(
    state: &AppState,
    install_id: Option<InstallId>,
    provider_id: &str,
    dependency_id: &str,
    version: &str,
    url: &str,
    expected_sha256: Option<&str>,
    archive: AgentServerArchive,
    bin_path: &str,
    target: InstallTarget,
    stage: &mut &'static str,
) -> Result<ManagedDependencyInstall> {
    let data_root = state.core.data_root.clone();
    let install_dir = install_dir_for_provider(&data_root, dependency_id, version, target);

    let existing = if install_dir.exists() {
        let direct = install_dir.join(bin_path);
        if direct.exists() {
            Some(direct)
        } else {
            find_unique_path_ending_with(&install_dir, bin_path).ok()
        }
    } else {
        None
    };

    let bin = if let Some(bin) = existing {
        ensure_executable(&bin)?;
        bin
    } else {
        install_agent_server_url_binary(
            state,
            install_id,
            dependency_id,
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
        .context("installing dependency binary")?
    };

    let bin_dir = bin.parent().unwrap_or(&install_dir).to_path_buf();
    let meta = ManagedInstallMetadata {
        package: Some(url.to_string()),
        version: Some(version.to_string()),
        target: Some(target),
        install_dir_rel: Some(install_dir_rel(&data_root, &install_dir)),
        bin_dir_rel: Some(install_dir_rel(&data_root, &bin_dir)),
        last_success_at: Some(Utc::now().to_rfc3339()),
        last_error: None,
    };

    Ok(ManagedDependencyInstall { meta })
}

fn resolve_install_args(args: &[String]) -> Vec<String> {
    args.to_vec()
}

fn map_archive_kind(kind: provider_matrix::ProviderArchiveKind) -> AgentServerArchive {
    match kind {
        provider_matrix::ProviderArchiveKind::None => AgentServerArchive::None,
        provider_matrix::ProviderArchiveKind::TarGz => AgentServerArchive::TarGz,
        provider_matrix::ProviderArchiveKind::TarBz2 => AgentServerArchive::TarBz2,
        provider_matrix::ProviderArchiveKind::Zip => AgentServerArchive::Zip,
        provider_matrix::ProviderArchiveKind::Dmg => AgentServerArchive::Dmg,
    }
}

#[cfg(test)]
mod tests {
    use super::artifacts::{
        commit_atomic_install_dir, prepare_atomic_install_dir, resolve_download_resume,
        validate_sha256_digest,
    };
    use super::toolchains::{
        node_runtime_target_for_install_target, python_target_can_use_bundled_runtime,
        resolve_python_bin,
    };
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    fn status_with_managed_target(target: &str) -> ctx_providers::adapters::ProviderStatus {
        let mut details = HashMap::new();
        details.insert("managed_target".to_string(), target.to_string());
        ctx_providers::adapters::ProviderStatus {
            provider_id: "codex".to_string(),
            installed: true,
            detected_path: Some("/tmp/codex".to_string()),
            version: None,
            capabilities: None,
            health: ctx_providers::adapters::ProviderHealth::Ok,
            diagnostics: Vec::new(),
            details,
        }
    }

    fn host_detected_status() -> ctx_providers::adapters::ProviderStatus {
        ctx_providers::adapters::ProviderStatus {
            provider_id: "codex".to_string(),
            installed: true,
            detected_path: Some("/usr/local/bin/codex".to_string()),
            version: None,
            capabilities: None,
            health: ctx_providers::adapters::ProviderHealth::Ok,
            diagnostics: Vec::new(),
            details: HashMap::new(),
        }
    }

    #[test]
    fn managed_provider_installs_are_enabled_for_supported_entries() {
        let matrix = provider_matrix::builtin_matrix();
        assert!(is_supported_managed_provider(&matrix, "codex"));
        assert!(is_supported_managed_provider(&matrix, "opencode"));
    }

    #[test]
    fn parse_install_target_defaults_to_host() {
        assert_eq!(
            parse_install_target(None).expect("default install target"),
            InstallTarget::Host
        );
    }

    #[test]
    fn parse_install_target_rejects_unknown_values() {
        let err =
            parse_install_target(Some("not-a-target")).expect_err("invalid target should fail");
        assert!(err.to_string().contains("invalid install target"));
    }

    #[test]
    fn archive_bin_requires_node_runtime_detects_javascript_entrypoints() {
        let missing = Path::new("/__ctx_missing_entrypoint__");
        assert!(archive_bin_requires_node_runtime(
            "dist/bin/amp-acp.js",
            missing
        ));
        assert!(archive_bin_requires_node_runtime(
            "dist/bin/provider.mjs",
            missing
        ));
        assert!(archive_bin_requires_node_runtime(
            "dist/bin/provider.cjs",
            missing
        ));
        assert!(!archive_bin_requires_node_runtime(
            "dist/bin/provider",
            missing
        ));
        assert!(!archive_bin_requires_node_runtime(
            "dist/bin/provider.exe",
            missing
        ));
    }

    #[test]
    fn archive_bin_requires_node_runtime_detects_extensionless_node_shebang() {
        let temp = tempfile::tempdir().expect("tempdir");
        let launcher = temp.path().join("claude-crp");
        std::fs::write(&launcher, "#!/usr/bin/env node\nconsole.log('ctx');\n")
            .expect("write launcher");
        assert!(archive_bin_requires_node_runtime(
            "bin/claude-crp",
            &launcher
        ));
    }

    #[test]
    fn archive_bin_requires_node_runtime_detects_env_shebang_with_flags() {
        let temp = tempfile::tempdir().expect("tempdir");
        let launcher = temp.path().join("provider");
        std::fs::write(
            &launcher,
            "#!/usr/bin/env -S node --no-warnings\nconsole.log('ctx');\n",
        )
        .expect("write launcher");
        assert!(archive_bin_requires_node_runtime("bin/provider", &launcher));
    }

    #[test]
    fn archive_bin_requires_node_runtime_ignores_non_node_shebang() {
        let temp = tempfile::tempdir().expect("tempdir");
        let launcher = temp.path().join("provider");
        std::fs::write(&launcher, "#!/bin/sh\necho ctx\n").expect("write launcher");
        assert!(!archive_bin_requires_node_runtime(
            "bin/provider",
            &launcher
        ));
    }

    #[test]
    fn node_runtime_dependency_id_is_target_specific() {
        assert_eq!(
            node_runtime_dependency_id(InstallTarget::Host),
            "runtime-node-host"
        );
        assert_eq!(
            node_runtime_dependency_id(InstallTarget::Container),
            "runtime-node-container"
        );
        assert_eq!(
            node_runtime_dependency_id(InstallTarget::LinuxAarch64),
            "runtime-node-linux-aarch64"
        );
    }

    #[test]
    fn container_node_runtime_target_is_linux_for_host_arch() {
        let target = node_runtime_target_for_install_target(InstallTarget::Container)
            .expect("container target mapping");
        match std::env::consts::ARCH {
            "aarch64" => assert_eq!(target.dist_target, "linux-arm64"),
            "x86_64" => assert_eq!(target.dist_target, "linux-x64"),
            other => panic!("unexpected test arch: {other}"),
        }
        assert!(!target.is_windows);
    }

    #[test]
    fn container_node_runtime_dependency_targets_include_host_on_non_linux() {
        assert_eq!(
            node_runtime_dependency_targets_for_install_target(InstallTarget::Container, "macos"),
            vec![InstallTarget::Container, InstallTarget::Host]
        );
        assert_eq!(
            node_runtime_dependency_targets_for_install_target(InstallTarget::Container, "windows"),
            vec![InstallTarget::Container, InstallTarget::Host]
        );
        assert_eq!(
            node_runtime_dependency_targets_for_install_target(InstallTarget::Container, "linux"),
            vec![InstallTarget::Container]
        );
    }

    #[test]
    fn managed_provider_runtime_command_wraps_acp_providers_with_bridge() {
        let data_root = tempfile::tempdir().expect("tempdir");
        let managed = AgentServerCommand {
            command: "/tmp/opencode".to_string(),
            args: vec!["acp".to_string()],
            dependencies: Vec::new(),
            managed: None,
        };
        let bridge = AgentServerCommand {
            command: "/tmp/acp-crp-bridge".to_string(),
            args: vec!["--stdio".to_string()],
            dependencies: Vec::new(),
            managed: None,
        };

        let runtime =
            managed_provider_runtime_command(data_root.path(), "opencode", managed, Some(&bridge))
                .expect("wrapped runtime command");

        assert_eq!(runtime.command, "/tmp/acp-crp-bridge");
        assert_eq!(runtime.args.first().map(String::as_str), Some("--stdio"));
        assert!(
            runtime.args.iter().any(|arg| arg == "--acp-command"),
            "bridge command must include ACP command wrapper"
        );
        assert!(
            runtime.args.iter().any(|arg| arg == "/tmp/opencode acp"),
            "bridge command should point at the installed ACP command"
        );
    }

    #[test]
    fn managed_provider_runtime_command_keeps_native_crp_providers_raw() {
        let managed = AgentServerCommand {
            command: "/tmp/codex-crp".to_string(),
            args: vec!["--stdio".to_string()],
            dependencies: Vec::new(),
            managed: None,
        };

        let runtime = managed_provider_runtime_command(Path::new("/tmp"), "codex", managed, None)
            .expect("raw runtime command");

        assert_eq!(runtime.command, "/tmp/codex-crp");
        assert_eq!(runtime.args, vec!["--stdio".to_string()]);
    }

    #[test]
    fn bundled_seed_js_runtime_prepends_bundled_node_bin_dir() {
        let temp = tempfile::tempdir().expect("tempdir");
        let script = temp
            .path()
            .join("providers/goose/macos/aarch64/goose-acp.js");
        std::fs::create_dir_all(script.parent().expect("parent")).expect("mkdir script");
        std::fs::write(&script, b"#!/usr/bin/env node\n").expect("write script");

        let node_bin = temp
            .path()
            .join("runtimes/node/macos/aarch64/node-v1/bin/node");
        std::fs::create_dir_all(node_bin.parent().expect("parent")).expect("mkdir node");
        std::fs::write(&node_bin, b"ok").expect("write node");

        let runtime_cmd = ProviderRuntimeCommand {
            provider_id: "goose".to_string(),
            command_abs_path: script.to_string_lossy().to_string(),
            args: Vec::new(),
            dependencies: Vec::new(),
            source: ProviderRuntimeCommandSource::BundledSeed,
        };
        let bundled_node = bundled_assets::BundledRuntimePaths {
            root: node_bin
                .parent()
                .expect("bin dir")
                .parent()
                .expect("runtime root")
                .to_path_buf(),
            bin: node_bin.clone(),
            npm_cli: None,
            version: "1".to_string(),
        };

        let mut bin_dirs = vec![script.parent().expect("script dir").to_path_buf()];
        prepend_bundled_seed_node_bin_dir(&mut bin_dirs, &runtime_cmd, Some(bundled_node));

        assert!(bin_dirs.contains(&node_bin.parent().expect("node dir").to_path_buf()));
    }

    #[test]
    fn dependency_target_compatibility_filters_linux_bins_for_non_linux_host_probes() {
        assert!(dependency_target_compatible_with_context(
            Some(InstallTarget::Host),
            false,
            "macos",
            "aarch64"
        ));
        assert!(!dependency_target_compatible_with_context(
            Some(InstallTarget::Container),
            false,
            "macos",
            "aarch64"
        ));
        assert!(!dependency_target_compatible_with_context(
            Some(InstallTarget::LinuxAarch64),
            false,
            "macos",
            "aarch64"
        ));
        assert!(dependency_target_compatible_with_context(
            Some(InstallTarget::Container),
            false,
            "linux",
            "x86_64"
        ));
    }

    #[test]
    fn dependency_target_compatibility_allows_linux_bins_for_container_exec() {
        assert!(dependency_target_compatible_with_context(
            Some(InstallTarget::Container),
            true,
            "macos",
            "aarch64"
        ));
        assert!(dependency_target_compatible_with_context(
            Some(InstallTarget::LinuxAarch64),
            true,
            "windows",
            "aarch64"
        ));
        assert!(!dependency_target_compatible_with_context(
            Some(InstallTarget::Host),
            true,
            "linux",
            "x86_64"
        ));
    }

    #[test]
    fn managed_provider_target_support_matches_install_kind() {
        let matrix = provider_matrix::builtin_matrix();
        let harness_provider_ids = [
            "claude-crp",
            "codex",
            "qwen",
            "cursor",
            "pi",
            "amp",
            "droid",
            "gemini",
            "copilot",
            "opencode",
            "cline",
            "mistral",
            "auggie",
            "goose",
            "kimi",
            "openhands",
        ];
        let mut archive_count = 0usize;
        assert_eq!(
            harness_provider_ids.len(),
            16,
            "curated harness list changed; update coverage expectation"
        );
        for target in [InstallTarget::Host, InstallTarget::Container] {
            let supported = harness_provider_ids
                .iter()
                .filter(|provider_id| {
                    is_supported_managed_provider_for_target(&matrix, provider_id, target)
                })
                .count();
            let target_label = match target {
                InstallTarget::Host => "host",
                InstallTarget::Container => "container",
                InstallTarget::LinuxAarch64 => "linux-aarch64",
                InstallTarget::LinuxX8664 => "linux-x86_64",
            };
            assert_eq!(
                supported,
                harness_provider_ids.len(),
                "expected full harness support for {target_label}: {supported}/{}",
                harness_provider_ids.len()
            );
        }
        for provider_id in harness_provider_ids {
            let entry = provider_matrix::get_entry(&matrix, provider_id)
                .unwrap_or_else(|| panic!("missing provider matrix entry for {provider_id}"));
            let install = entry
                .managed_install
                .as_ref()
                .unwrap_or_else(|| panic!("missing managed_install for {provider_id}"));
            match install {
                provider_matrix::ProviderInstall::Archive { .. } => {
                    archive_count += 1;
                    assert!(
                        is_supported_managed_provider_for_target(
                            &matrix,
                            provider_id,
                            InstallTarget::LinuxAarch64
                        ),
                        "archive provider {provider_id} missing linux-aarch64 support"
                    );
                    assert!(
                        is_supported_managed_provider_for_target(
                            &matrix,
                            provider_id,
                            InstallTarget::LinuxX8664
                        ),
                        "archive provider {provider_id} missing linux-x86_64 support"
                    );
                }
                provider_matrix::ProviderInstall::Npm { .. }
                | provider_matrix::ProviderInstall::Python { .. } => {
                    assert!(
                        is_supported_managed_provider_for_target(
                            &matrix,
                            provider_id,
                            InstallTarget::Host
                        ),
                        "managed provider {provider_id} must support host installs"
                    );
                    assert!(
                        is_supported_managed_provider_for_target(
                            &matrix,
                            provider_id,
                            InstallTarget::Container
                        ),
                        "managed provider {provider_id} must support container installs"
                    );
                }
            }
        }
        assert_eq!(
            archive_count, 8,
            "curated harness archive set changed; verify linux target coverage expectations"
        );
        assert!(is_supported_managed_provider_for_target(
            &matrix,
            "codex",
            InstallTarget::Container
        ));
        assert!(is_supported_managed_provider_for_target(
            &matrix,
            "auggie",
            InstallTarget::Container
        ));
    }

    #[test]
    fn python_bundled_runtime_is_host_only() {
        assert!(python_target_can_use_bundled_runtime(InstallTarget::Host));
        assert!(!python_target_can_use_bundled_runtime(
            InstallTarget::Container
        ));
        assert!(!python_target_can_use_bundled_runtime(
            InstallTarget::LinuxAarch64
        ));
        assert!(!python_target_can_use_bundled_runtime(
            InstallTarget::LinuxX8664
        ));
    }

    #[test]
    fn expected_managed_dependency_version_detects_runtime_dependencies() {
        assert_eq!(
            expected_managed_dependency_version("runtime-node-host"),
            Some(NODE_VERSION)
        );
        assert_eq!(
            expected_managed_dependency_version("runtime-python-container"),
            Some(PYTHON_VERSION)
        );
        assert_eq!(expected_managed_dependency_version("codex"), None);
    }

    #[test]
    fn python_paths_for_container_target_use_linux_layout() {
        let python_root = Path::new("/tmp/python-runtime");
        let venv_root = Path::new("/tmp/provider-venv");
        assert_eq!(
            resolve_python_bin(python_root, InstallTarget::Container),
            python_root.join("bin").join("python")
        );
        assert_eq!(
            venv_exe(venv_root, "python", InstallTarget::Container),
            venv_root.join("bin").join("python")
        );
    }

    #[test]
    fn python_paths_for_linux_targets_use_linux_layout() {
        let python_root = Path::new("/tmp/python-runtime");
        let venv_root = Path::new("/tmp/provider-venv");
        for target in [InstallTarget::LinuxAarch64, InstallTarget::LinuxX8664] {
            assert_eq!(
                resolve_python_bin(python_root, target),
                python_root.join("bin").join("python")
            );
            assert_eq!(
                venv_exe(venv_root, "python", target),
                venv_root.join("bin").join("python")
            );
        }
    }

    #[test]
    fn validate_sha256_digest_accepts_case_insensitive_match() {
        assert!(validate_sha256_digest("ABcd1234", "abcd1234").is_ok());
    }

    #[test]
    fn validate_sha256_digest_rejects_mismatch() {
        let err = validate_sha256_digest("abcd1234", "ffff1234")
            .expect_err("mismatched digest should fail");
        assert!(err.to_string().contains("archive checksum mismatch"));
    }

    #[test]
    fn resolve_download_resume_handles_partial_content() {
        let (resumed, total) =
            resolve_download_resume(120, reqwest::StatusCode::PARTIAL_CONTENT, Some(880));
        assert!(resumed);
        assert_eq!(total, Some(1000));
    }

    #[test]
    fn resolve_download_resume_restarts_on_non_partial_status() {
        let (resumed, total) = resolve_download_resume(120, reqwest::StatusCode::OK, Some(880));
        assert!(!resumed);
        assert_eq!(total, Some(880));
    }

    #[test]
    fn classify_install_error_maps_codes() {
        assert_eq!(
            classify_install_error("download", &anyhow::anyhow!("sending request failed")),
            InstallErrorCode::DownloadFailed
        );
        assert_eq!(
            classify_install_error("refresh", &anyhow::anyhow!("provider not healthy")),
            InstallErrorCode::HealthCheckFailed
        );
        assert_eq!(
            classify_install_error(
                "registry",
                &anyhow::anyhow!("managed install registry write failed")
            ),
            InstallErrorCode::RegistryWriteFailed
        );
        assert_eq!(
            classify_install_error("download", &anyhow::anyhow!("install canceled by user")),
            InstallErrorCode::Cancelled
        );
    }

    #[test]
    fn apply_install_target_status_marks_mismatch_as_missing() {
        let mut status = status_with_managed_target("host");
        apply_install_target_status(&mut status, InstallTarget::Container);
        assert!(!status.installed);
        assert!(matches!(
            status.health,
            ctx_providers::adapters::ProviderHealth::Missing
        ));
        assert_eq!(
            status.details.get("target_mismatch").map(String::as_str),
            Some("true")
        );
    }

    #[test]
    fn apply_install_target_status_marks_host_detected_status_unverified_for_container() {
        let mut status = host_detected_status();
        apply_install_target_status(&mut status, InstallTarget::Container);
        assert!(!status.installed);
        assert!(matches!(
            status.health,
            ctx_providers::adapters::ProviderHealth::Missing
        ));
        assert_eq!(
            status.details.get("target_unverified").map(String::as_str),
            Some("true")
        );
    }

    #[test]
    fn validate_post_install_status_rejects_target_mismatch() {
        let status = status_with_managed_target("host");
        let err = validate_post_install_status(&status, "codex", InstallTarget::Container)
            .expect_err("target mismatch should fail verification");
        assert!(err.to_string().contains("expected 'container'"));
    }

    #[test]
    fn validate_post_install_status_accepts_matching_target() {
        let status = status_with_managed_target("container");
        validate_post_install_status(&status, "codex", InstallTarget::Container)
            .expect("matching target should pass verification");
    }

    #[tokio::test]
    async fn provider_install_lock_serializes_same_provider_target() {
        let first = acquire_provider_install_lock("codex", InstallTarget::Container).await;
        let acquired = Arc::new(AtomicBool::new(false));
        let acquired2 = acquired.clone();
        let waiter = tokio::spawn(async move {
            let _second = acquire_provider_install_lock("codex", InstallTarget::Container).await;
            acquired2.store(true, Ordering::SeqCst);
        });

        tokio::time::sleep(Duration::from_millis(40)).await;
        assert!(
            !acquired.load(Ordering::SeqCst),
            "second lock should block while first lock is held"
        );

        drop(first);
        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("second lock should acquire after first unlock")
            .expect("waiter task should finish without panic");
        assert!(acquired.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn atomic_install_commit_replaces_existing_install_dir() {
        let temp = tempfile::tempdir().expect("tempdir");
        let install_dir = temp.path().join("providers").join("codex").join("1.2.3");
        tokio::fs::create_dir_all(install_dir.join("old"))
            .await
            .expect("create old dir");
        tokio::fs::write(install_dir.join("old").join("keep.txt"), b"old")
            .await
            .expect("write old file");

        let staging_dir = prepare_atomic_install_dir(&install_dir)
            .await
            .expect("prepare staging dir");
        tokio::fs::create_dir_all(staging_dir.join("new"))
            .await
            .expect("create new dir");
        tokio::fs::write(staging_dir.join("new").join("fresh.txt"), b"new")
            .await
            .expect("write new file");

        commit_atomic_install_dir(&staging_dir, &install_dir)
            .await
            .expect("commit install dir");

        assert!(
            tokio::fs::metadata(staging_dir).await.is_err(),
            "staging dir should be moved into final location"
        );
        assert!(
            tokio::fs::metadata(install_dir.join("new").join("fresh.txt"))
                .await
                .is_ok()
        );
        assert!(
            tokio::fs::metadata(install_dir.join("old").join("keep.txt"))
                .await
                .is_err(),
            "old install contents should be replaced"
        );
    }
}
