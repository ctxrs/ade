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
use crate::daemon::AppState;
use crate::installs::{
    truncate_for_storage, InstallErrorCode, InstallEventLevel, InstallId, InstallProgressEvent,
    InstallTarget,
};
use crate::lsp_catalog::{LspCatalogArchive, LspCatalogInstall};
use crate::provider_matrix;
use crate::title_generation_local;
use crate::updates;
use ctx_providers::crp::Tier1CrpAdapter;

mod config;

pub use config::{
    agent_server_config_path, apply_managed_install_details, apply_managed_lsp_server_config,
    apply_user_lsp_server_config, load_agent_server_config, load_lsp_server_config,
    load_user_lsp_config, resolve_provider_command, resolve_runtime_provider_command,
    save_agent_server_config, save_lsp_server_config, AgentServerCommand, AgentServerConfigFile,
    LspServerConfigFile, ManagedInstallError, ManagedInstallMetadata, ProviderRuntimeCommand,
    ProviderRuntimeCommandSource, UserLspConfigFile, UserLspServerSpec,
};

const NODE_VERSION: &str = "24.14.0";
const PYTHON_VERSION: &str = "3.13.12";
const PYTHON_BUILD_TAG: &str = "20260211";

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

static NODE_RUNTIME_INSTALL_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static PYTHON_RUNTIME_INSTALL_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static PROVIDER_INSTALL_LOCKS: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();
const TITLE_GENERATION_LOCAL_INSTALL_KEY: &str = "title_generation_local";
const MANAGED_PROVIDER_INSTALLS_ENABLED: bool = true;

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

pub(crate) fn prepend_runtime_bin_dirs_to_provider_path(
    provider_env: &mut HashMap<String, String>,
    cfg: &AgentServerConfigFile,
    runtime_provider_id: &str,
    data_root: &Path,
) {
    let mut bin_dirs: Vec<PathBuf> = Vec::new();
    let container_exec = provider_env.contains_key("CTX_HARNESS_CONTAINER_ID");
    if let Ok(Some(runtime_cmd)) = resolve_runtime_provider_command(cfg, runtime_provider_id) {
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
    provider_matrix::invalidate_matrix_cache(&state.providers.matrix_cache).await;
    let res = install_provider_impl(state.as_ref(), &provider_id, target, Some(install_id)).await;
    match &res {
        Ok(()) => state.finish_install(install_id, true, None, None).await,
        Err(e) => {
            let code = classify_install_error("provider_install", e);
            state
                .finish_install(
                    install_id,
                    false,
                    Some(truncate_for_storage(&format!("{e:#}"), 12_000)),
                    Some(code),
                )
                .await
        }
    }
    provider_matrix::invalidate_matrix_cache(&state.providers.matrix_cache).await;
    res
}

fn resolve_command_path(command: &str) -> (bool, Option<PathBuf>) {
    if command.contains(std::path::MAIN_SEPARATOR)
        || command.contains('/')
        || command.contains('\\')
    {
        let p = PathBuf::from(command);
        if p.exists() {
            return (true, Some(p));
        }
        return (false, None);
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(command);
        if candidate.exists() {
            return (true, Some(candidate));
        }
    }
    (false, None)
}

fn target_uses_windows_layout(target: InstallTarget) -> bool {
    match target {
        InstallTarget::Host => cfg!(windows),
        InstallTarget::Container | InstallTarget::LinuxAarch64 | InstallTarget::LinuxX8664 => false,
    }
}

fn venv_bin_dir(venv_dir: &Path, target: InstallTarget) -> PathBuf {
    if target_uses_windows_layout(target) {
        venv_dir.join("Scripts")
    } else {
        venv_dir.join("bin")
    }
}

fn venv_exe(venv_dir: &Path, name: &str, target: InstallTarget) -> PathBuf {
    let bin = venv_bin_dir(venv_dir, target);
    if target_uses_windows_layout(target) {
        bin.join(format!("{name}.exe"))
    } else {
        bin.join(name)
    }
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

fn ensure_executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path)
            .with_context(|| format!("stat {}", path.display()))?
            .permissions();
        perms.set_mode(perms.mode() | 0o111);
        std::fs::set_permissions(path, perms)
            .with_context(|| format!("chmod {}", path.display()))?;
    }
    Ok(())
}

fn find_unique_path_ending_with(root: &Path, suffix: &str) -> Result<PathBuf> {
    let suffix = suffix.replace('\\', "/");
    let mut matches = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in
            std::fs::read_dir(&dir).with_context(|| format!("read_dir {}", dir.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let rel = path.strip_prefix(root).unwrap_or(&path);
            if rel.to_string_lossy().replace('\\', "/").ends_with(&suffix) {
                matches.push(path);
            }
        }
    }
    if matches.len() == 1 {
        return Ok(matches.remove(0));
    }
    if matches.is_empty() {
        anyhow::bail!("could not find extracted binary ending with {suffix}");
    }
    anyhow::bail!("multiple extracted binaries match {suffix}");
}

fn extract_zip_to_dir(zip_path: &Path, out_dir: &Path) -> Result<()> {
    let file =
        std::fs::File::open(zip_path).with_context(|| format!("open {}", zip_path.display()))?;
    let mut archive = zip::ZipArchive::new(file).context("parsing zip")?;
    for i in 0..archive.len() {
        let mut f = archive.by_index(i).context("zip entry")?;
        let name = f.name().to_string();
        if name.ends_with('/') {
            continue;
        }
        let dest = out_dir.join(name);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let mut out =
            std::fs::File::create(&dest).with_context(|| format!("create {}", dest.display()))?;
        std::io::copy(&mut f, &mut out).context("extract zip entry")?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum AgentServerArchive {
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

fn extract_tar_bz2_to_dir(tar_bz2_path: &Path, out_dir: &Path) -> Result<()> {
    let tar_bz2 = std::fs::File::open(tar_bz2_path)
        .with_context(|| format!("open {}", tar_bz2_path.display()))?;
    let dec = bzip2::read::BzDecoder::new(tar_bz2);
    let mut archive = tar::Archive::new(dec);
    archive.unpack(out_dir).context("extract tar.bz2")?;
    Ok(())
}

async fn prepare_atomic_install_dir(install_dir: &Path) -> Result<PathBuf> {
    let parent = install_dir
        .parent()
        .ok_or_else(|| anyhow::anyhow!("install dir has no parent: {}", install_dir.display()))?;
    tokio::fs::create_dir_all(parent).await.ok();
    let install_name = install_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("install");
    let staging_dir = parent.join(format!(
        ".{install_name}.staging-{}",
        uuid::Uuid::new_v4().simple()
    ));
    if staging_dir.exists() {
        tokio::fs::remove_dir_all(&staging_dir).await.ok();
    }
    tokio::fs::create_dir_all(&staging_dir)
        .await
        .with_context(|| format!("creating staging dir: {}", staging_dir.display()))?;
    Ok(staging_dir)
}

async fn commit_atomic_install_dir(staging_dir: &Path, install_dir: &Path) -> Result<()> {
    if install_dir.exists() {
        tokio::fs::remove_dir_all(install_dir).await.ok();
    }
    tokio::fs::rename(staging_dir, install_dir)
        .await
        .with_context(|| {
            format!(
                "committing staging dir {} -> {}",
                staging_dir.display(),
                install_dir.display()
            )
        })?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn install_agent_server_url_binary(
    state: &AppState,
    install_id: Option<InstallId>,
    provider_id: &str,
    event_provider_id: &str,
    version: &str,
    url: &str,
    expected_sha256: Option<&str>,
    archive: AgentServerArchive,
    bin_path: &str,
    target: InstallTarget,
    stage: &mut &'static str,
) -> Result<PathBuf> {
    let data_root = &state.core.data_root;
    let install_dir = install_dir_for_provider(data_root, provider_id, version, target);

    let tmp_dir = data_root.join("providers").join("tmp");
    tokio::fs::create_dir_all(&tmp_dir).await.ok();
    let tmp = tmp_dir.join(format!(
        "{provider_id}-{version}-{}.download",
        target.as_str()
    ));
    let staging_dir = prepare_atomic_install_dir(&install_dir).await?;

    *stage = "download";
    download_to_file(state, install_id, event_provider_id, "download", url, &tmp).await?;

    let expected_sha256 = expected_sha256
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(expected_sha256) = expected_sha256 {
        *stage = "verify";
        emit_install(
            state,
            install_id,
            event_provider_id,
            InstallEventLevel::Info,
            "verify",
            "Verifying archive checksum".to_string(),
            None,
            None,
            None,
        )
        .await;

        let digest = sha256_file(&tmp).await?;
        validate_sha256_digest(expected_sha256, &digest)?;
    } else {
        emit_install(
            state,
            install_id,
            event_provider_id,
            InstallEventLevel::Warning,
            "verify",
            "Archive checksum missing in provider matrix; proceeding without verification"
                .to_string(),
            None,
            None,
            None,
        )
        .await;
    }

    *stage = "extract";
    emit_install(
        state,
        install_id,
        event_provider_id,
        InstallEventLevel::Info,
        "extract",
        "Extracting…".to_string(),
        None,
        None,
        None,
    )
    .await;

    let resolved_in_staging = match archive {
        AgentServerArchive::None => {
            let dest = staging_dir.join(bin_path);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            std::fs::rename(&tmp, &dest).with_context(|| {
                format!("move downloaded binary into staging: {}", dest.display())
            })?;
            ensure_executable(&dest)?;
            dest
        }
        AgentServerArchive::TarGz => {
            let tar_gz =
                std::fs::File::open(&tmp).with_context(|| format!("open {}", tmp.display()))?;
            let dec = flate2::read::GzDecoder::new(tar_gz);
            let mut archive = tar::Archive::new(dec);
            archive.unpack(&staging_dir).context("extract tar.gz")?;
            let direct = staging_dir.join(bin_path);
            if direct.exists() {
                direct
            } else {
                find_unique_path_ending_with(&staging_dir, bin_path)?
            }
        }
        AgentServerArchive::TarBz2 => {
            extract_tar_bz2_to_dir(&tmp, &staging_dir)?;
            let direct = staging_dir.join(bin_path);
            if direct.exists() {
                direct
            } else {
                find_unique_path_ending_with(&staging_dir, bin_path)?
            }
        }
        AgentServerArchive::Zip => {
            extract_zip_to_dir(&tmp, &staging_dir)?;
            let direct = staging_dir.join(bin_path);
            if direct.exists() {
                direct
            } else {
                find_unique_path_ending_with(&staging_dir, bin_path)?
            }
        }
        AgentServerArchive::Dmg => {
            tokio::fs::remove_dir_all(&staging_dir).await.ok();
            anyhow::bail!("dmg archive extraction is not supported in managed installs")
        }
    };
    ensure_executable(&resolved_in_staging)?;

    let relative_bin = resolved_in_staging
        .strip_prefix(&staging_dir)
        .ok()
        .map(|path| path.to_path_buf());

    if let Err(error) = commit_atomic_install_dir(&staging_dir, &install_dir).await {
        tokio::fs::remove_dir_all(&staging_dir).await.ok();
        return Err(error);
    }

    let resolved = if let Some(relative_bin) = relative_bin {
        let candidate = install_dir.join(relative_bin);
        if candidate.exists() {
            candidate
        } else {
            let direct = install_dir.join(bin_path);
            if direct.exists() {
                direct
            } else {
                find_unique_path_ending_with(&install_dir, bin_path)?
            }
        }
    } else {
        let direct = install_dir.join(bin_path);
        if direct.exists() {
            direct
        } else {
            find_unique_path_ending_with(&install_dir, bin_path)?
        }
    };
    ensure_executable(&resolved)?;
    Ok(resolved)
}

#[allow(clippy::too_many_arguments)]
async fn install_url_binary(
    state: &AppState,
    install_id: Option<InstallId>,
    provider_id: &str,
    server_id: &str,
    version: &str,
    url: &str,
    archive: LspCatalogArchive,
    bin_path: &str,
) -> Result<PathBuf> {
    let data_root = &state.core.data_root;
    let install_dir = data_root
        .join("lsp")
        .join("binaries")
        .join(server_id)
        .join(version);
    tokio::fs::create_dir_all(&install_dir).await.ok();

    let tmp_dir = data_root.join("lsp").join("tmp");
    tokio::fs::create_dir_all(&tmp_dir).await.ok();
    let tmp = tmp_dir.join(format!("{server_id}-{version}.download"));

    download_to_file(state, install_id, provider_id, "download", url, &tmp).await?;

    emit_install(
        state,
        install_id,
        provider_id,
        InstallEventLevel::Info,
        "extract",
        "Extracting…".to_string(),
        None,
        None,
        None,
    )
    .await;

    match archive {
        LspCatalogArchive::None => {
            let dest = install_dir.join(bin_path);
            if let Some(parent) = dest.parent() {
                tokio::fs::create_dir_all(parent).await.ok();
            }
            tokio::fs::rename(&tmp, &dest).await.ok();
            ensure_executable(&dest)?;
            Ok(dest)
        }
        LspCatalogArchive::Gz => {
            let gz =
                std::fs::File::open(&tmp).with_context(|| format!("open {}", tmp.display()))?;
            let mut dec = flate2::read::GzDecoder::new(gz);
            let dest = install_dir.join(bin_path);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            let mut out = std::fs::File::create(&dest)
                .with_context(|| format!("create {}", dest.display()))?;
            std::io::copy(&mut dec, &mut out).context("decompress gz")?;
            ensure_executable(&dest)?;
            Ok(dest)
        }
        LspCatalogArchive::TarGz => {
            let tar_gz =
                std::fs::File::open(&tmp).with_context(|| format!("open {}", tmp.display()))?;
            let dec = flate2::read::GzDecoder::new(tar_gz);
            let mut archive = tar::Archive::new(dec);
            archive.unpack(&install_dir).context("extract tar.gz")?;
            let direct = install_dir.join(bin_path);
            let resolved = if direct.exists() {
                direct
            } else {
                find_unique_path_ending_with(&install_dir, bin_path)?
            };
            ensure_executable(&resolved)?;
            Ok(resolved)
        }
        LspCatalogArchive::Zip => {
            extract_zip_to_dir(&tmp, &install_dir)?;
            let direct = install_dir.join(bin_path);
            let resolved = if direct.exists() {
                direct
            } else {
                find_unique_path_ending_with(&install_dir, bin_path)?
            };
            ensure_executable(&resolved)?;
            Ok(resolved)
        }
    }
}

async fn enable_managed_lsp_server(
    data_root: &Path,
    language_id: &str,
    command: &str,
    args: Vec<String>,
    managed_meta_key: &str,
    meta: ManagedInstallMetadata,
) -> Result<()> {
    let mut cfg = load_lsp_server_config(data_root).await.unwrap_or_default();
    cfg.managed_installs
        .insert(managed_meta_key.to_string(), meta.clone());
    cfg.servers.insert(
        language_id.to_string(),
        AgentServerCommand {
            command: command.to_string(),
            args,
            dependencies: Vec::new(),
            managed: Some(meta),
        },
    );
    save_lsp_server_config(data_root, &cfg).await?;
    Ok(())
}

async fn upsert_managed_extra_server(data_root: &Path, spec: UserLspServerSpec) -> Result<()> {
    let mut cfg = load_lsp_server_config(data_root).await.unwrap_or_default();
    if let Some(id) = spec.id.as_deref() {
        cfg.extra_servers.retain(|s| s.id.as_deref() != Some(id));
    }
    cfg.extra_servers.push(spec);
    save_lsp_server_config(data_root, &cfg).await?;
    Ok(())
}

pub async fn install_lsp_catalog_server_with_progress(
    state: std::sync::Arc<AppState>,
    install_id: InstallId,
    catalog_id: String,
) -> Result<()> {
    let res = install_lsp_catalog_server_impl(state.as_ref(), &catalog_id, Some(install_id)).await;
    match &res {
        Ok(()) => state.finish_install(install_id, true, None, None).await,
        Err(e) => {
            let code = classify_install_error("lsp_catalog_install", e);
            state
                .finish_install(
                    install_id,
                    false,
                    Some(truncate_for_storage(&format!("{e:#}"), 12_000)),
                    Some(code),
                )
                .await
        }
    }
    res
}

async fn install_lsp_catalog_server_impl(
    state: &AppState,
    catalog_id: &str,
    install_id: Option<InstallId>,
) -> Result<()> {
    let data_root = state.core.data_root.clone();
    let provider_id = format!("lsp:{catalog_id}");
    let entry = crate::lsp_catalog::get_entry(&data_root, catalog_id).await?;
    let extra_id = entry.id.clone();
    let extra_language_id = entry.language_id.clone();
    let extra_args = entry.args.clone();
    let extra_extensions = entry.extensions.clone();
    let extra_filenames = entry.filenames.clone();

    emit_install(
        state,
        install_id,
        &provider_id,
        InstallEventLevel::Info,
        "start",
        format!("Installing LSP server: {}", entry.title),
        None,
        None,
        None,
    )
    .await;

    match entry.install.clone() {
        LspCatalogInstall::ManagedNode { server_id } => {
            install_lsp_server_impl(state, &server_id, install_id).await?;
            return Ok(());
        }
        LspCatalogInstall::System { command } => {
            let (found, resolved) = resolve_command_path(&command);
            if !found {
                anyhow::bail!("system command not found on PATH: {command}");
            }
            let cmd = resolved
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or(command);
            let meta = ManagedInstallMetadata {
                package: Some("system".to_string()),
                version: None,
                target: None,
                install_dir_rel: None,
                bin_dir_rel: None,
                last_success_at: Some(Utc::now().to_rfc3339()),
                last_error: None,
            };
            enable_managed_lsp_server(
                &data_root,
                &entry.language_id,
                &cmd,
                entry.args.clone(),
                &entry.id,
                meta,
            )
            .await?;
            upsert_managed_extra_server(
                &data_root,
                UserLspServerSpec {
                    id: Some(extra_id.clone()),
                    language_id: extra_language_id.clone(),
                    command: cmd,
                    args: extra_args.clone(),
                    extensions: extra_extensions.clone(),
                    filenames: extra_filenames.clone(),
                },
            )
            .await?;
        }
        LspCatalogInstall::GoInstall {
            module,
            version,
            binary,
        } => {
            let install_dir = data_root
                .join("lsp")
                .join("go")
                .join(&entry.id)
                .join(&version)
                .join("bin");
            tokio::fs::create_dir_all(&install_dir).await.ok();
            emit_install(
                state,
                install_id,
                &provider_id,
                InstallEventLevel::Info,
                "go_install",
                format!("go install {module}@{version}"),
                None,
                None,
                None,
            )
            .await;
            let status = Command::new("go")
                .arg("install")
                .arg(format!("{module}@{version}"))
                .env("GOBIN", &install_dir)
                .status()
                .await
                .context("running go install")?;
            if !status.success() {
                anyhow::bail!("go install failed");
            }
            let bin = install_dir.join(&binary);
            if !bin.exists() {
                anyhow::bail!("go install completed but binary missing: {}", bin.display());
            }
            ensure_executable(&bin)?;
            let meta = ManagedInstallMetadata {
                package: Some(module),
                version: Some(version.clone()),
                target: None,
                install_dir_rel: Some(install_dir_rel(&data_root, &install_dir)),
                bin_dir_rel: None,
                last_success_at: Some(Utc::now().to_rfc3339()),
                last_error: None,
            };
            let cmd = bin.to_string_lossy().to_string();
            enable_managed_lsp_server(
                &data_root,
                &entry.language_id,
                &cmd,
                entry.args.clone(),
                &entry.id,
                meta,
            )
            .await?;
            upsert_managed_extra_server(
                &data_root,
                UserLspServerSpec {
                    id: Some(extra_id.clone()),
                    language_id: extra_language_id.clone(),
                    command: cmd,
                    args: extra_args.clone(),
                    extensions: extra_extensions.clone(),
                    filenames: extra_filenames.clone(),
                },
            )
            .await?;
        }
        LspCatalogInstall::UrlBinary { version, targets } => {
            let target_key = catalog_target_key();
            let t = targets
                .get(target_key)
                .ok_or_else(|| anyhow::anyhow!("no catalog target for {target_key}"))?;
            let bin = install_url_binary(
                state,
                install_id,
                &provider_id,
                &entry.id,
                &version,
                &t.url,
                t.archive,
                &t.bin_path,
            )
            .await?;
            let meta = ManagedInstallMetadata {
                package: Some(t.url.clone()),
                version: Some(version.clone()),
                target: None,
                install_dir_rel: Some(install_dir_rel(&data_root, bin.parent().unwrap_or(&bin))),
                bin_dir_rel: None,
                last_success_at: Some(Utc::now().to_rfc3339()),
                last_error: None,
            };
            let cmd = bin.to_string_lossy().to_string();
            enable_managed_lsp_server(
                &data_root,
                &entry.language_id,
                &cmd,
                entry.args.clone(),
                &entry.id,
                meta,
            )
            .await?;
            upsert_managed_extra_server(
                &data_root,
                UserLspServerSpec {
                    id: Some(extra_id.clone()),
                    language_id: extra_language_id.clone(),
                    command: cmd,
                    args: extra_args.clone(),
                    extensions: extra_extensions.clone(),
                    filenames: extra_filenames.clone(),
                },
            )
            .await?;
        }
    }

    emit_install(
        state,
        install_id,
        &provider_id,
        InstallEventLevel::Success,
        "done",
        "Install complete (restart daemon to apply)".to_string(),
        None,
        None,
        None,
    )
    .await;
    Ok(())
}

pub async fn install_lsp_server_with_progress(
    state: std::sync::Arc<AppState>,
    install_id: InstallId,
    server_id: String,
) -> Result<()> {
    let res = install_lsp_server_impl(state.as_ref(), &server_id, Some(install_id)).await;
    match &res {
        Ok(()) => state.finish_install(install_id, true, None, None).await,
        Err(e) => {
            let code = classify_install_error("lsp_install", e);
            state
                .finish_install(
                    install_id,
                    false,
                    Some(truncate_for_storage(&format!("{e:#}"), 12_000)),
                    Some(code),
                )
                .await
        }
    }
    res
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

async fn install_provider_impl(
    state: &AppState,
    provider_id: &str,
    target: InstallTarget,
    install_id: Option<InstallId>,
) -> Result<()> {
    if !MANAGED_PROVIDER_INSTALLS_ENABLED {
        anyhow::bail!(
            "managed provider installs are disabled; provider '{}' must be shipped in bundled harness assets",
            provider_id
        );
    }

    let provider_id = provider_id.to_string();
    let _provider_install_lock = acquire_provider_install_lock(&provider_id, target).await;
    let requested_target_label = target.as_str();
    let resolved_target_key =
        resolve_matrix_target_key(target).context("resolving install target key")?;
    let mut stage: &'static str = "start";
    let mut error_package: Option<String> = None;
    let mut error_version: Option<String> = None;
    let mut error_install_dir_rel: Option<String> = None;

    let res: Result<()> = async {
        ensure_install_not_cancelled(state, install_id).await?;
        emit_install(
            state,
            install_id,
            &provider_id,
            InstallEventLevel::Info,
            "start",
            format!(
                "Installing managed provider: {provider_id} (target: {requested_target_label}, resolved: {resolved_target_key})"
            ),
            None,
            None,
            None,
        )
        .await;

        let matrix = provider_matrix::load_matrix_cached(
            &state.core.data_root,
            &state.providers.matrix_cache,
        )
        .await;
        let entry = provider_matrix::get_entry(&matrix, &provider_id)
            .ok_or_else(|| anyhow::anyhow!("unsupported provider for install: {provider_id}"))?;
        let Some(install) = entry.managed_install.as_ref() else {
            anyhow::bail!("provider has no managed install: {provider_id}");
        };
        let context_version = updates::normalize_version_str(env!("CARGO_PKG_VERSION"));
        let release = provider_matrix::recommended_release(entry, context_version.as_ref())
            .ok_or_else(|| anyhow::anyhow!("no compatible release for provider: {provider_id}"))?;

        let mut dependency_ids: Vec<String> = Vec::new();
        let mut implicit_managed_dependencies: Vec<(String, ManagedInstallMetadata)> = Vec::new();
        if !entry.dependencies.is_empty() {
            stage = "dependencies";
            ensure_install_not_cancelled(state, install_id).await?;
            emit_install(
                state,
                install_id,
                &provider_id,
                InstallEventLevel::Info,
                "dependencies",
                format!("Installing dependencies for {provider_id}"),
                None,
                None,
                None,
            )
            .await;

            for dep in &entry.dependencies {
                ensure_install_not_cancelled(state, install_id).await?;
                dependency_ids.push(dep.id.clone());
                let managed = match &dep.install {
                    provider_matrix::DependencyInstall::Npm { package, version } => {
                        if !matches!(target, InstallTarget::Host) {
                            anyhow::bail!(
                                "target '{}' is not supported for npm dependency '{}' (provider '{}'); use target host",
                                requested_target_label,
                                dep.id,
                                provider_id
                            );
                        }
                        error_package = Some(package.clone());
                        error_version = Some(version.clone());
                        error_install_dir_rel =
                            Some(format!("providers/agent-servers/{}/{}", dep.id, version));
                        install_managed_npm_dependency(
                            state,
                            install_id,
                            &provider_id,
                            &dep.id,
                            package,
                            version,
                            &mut stage,
                        )
                        .await?
                    }
                    provider_matrix::DependencyInstall::Archive { version, targets } => {
                        let target_entry = targets.get(resolved_target_key).ok_or_else(|| {
                            anyhow::anyhow!(
                                "unsupported dependency target {}: {}",
                                dep.id,
                                resolved_target_key
                            )
                        })?;
                        error_package = Some(target_entry.url.clone());
                        error_version = Some(version.clone());
                        error_install_dir_rel =
                            Some(format!("providers/agent-servers/{}/{}", dep.id, version));
                        install_managed_archive_dependency(
                            state,
                            install_id,
                            &provider_id,
                            &dep.id,
                            version,
                            &target_entry.url,
                            target_entry.sha256.as_deref(),
                            map_archive_kind(target_entry.archive),
                            &target_entry.bin_path,
                            target,
                            &mut stage,
                        )
                        .await?
                    }
                };

                let mut cfg = load_agent_server_config(&state.core.data_root)
                    .await
                    .unwrap_or_default();
                cfg.managed_installs
                    .insert(dep.id.clone(), managed.meta.clone());
                save_agent_server_config(&state.core.data_root, &cfg)
                    .await
                    .context("saving managed install registry")?;
            }
        }

        ensure_install_not_cancelled(state, install_id).await?;
        let managed = match install {
            provider_matrix::ProviderInstall::Npm {
                package,
                entrypoint,
                args,
            } => {
                if !matches!(target, InstallTarget::Host | InstallTarget::Container) {
                    anyhow::bail!(
                        "target '{}' is not supported for npm provider '{}' installs; use target host or container",
                        requested_target_label,
                        provider_id
                    );
                }
                let version = release.version.clone();
                error_package = Some(package.clone());
                error_version = Some(version.clone());
                error_install_dir_rel = Some(format!(
                    "providers/agent-servers/{}/{}",
                    provider_id, version
                ));
                install_managed_npm_provider(
                    state,
                    install_id,
                    &provider_id,
                    package,
                    &version,
                    entrypoint,
                    resolve_install_args(args),
                    target,
                    &mut stage,
                )
                .await?
            }
            provider_matrix::ProviderInstall::Python {
                package,
                version,
                entrypoint,
                args,
            } => {
                if !matches!(target, InstallTarget::Host | InstallTarget::Container) {
                    anyhow::bail!(
                        "target '{}' is not supported for python provider '{}' installs; use target host or container",
                        requested_target_label,
                        provider_id
                    );
                }
                if provider_matrix::normalize_version(version)
                    != provider_matrix::normalize_version(&release.version)
                {
                    anyhow::bail!(
                        "provider matrix version mismatch for {provider_id}: release={} install={}",
                        release.version,
                        version
                    );
                }
                error_package = Some(package.clone());
                error_version = Some(version.clone());
                error_install_dir_rel = Some(format!(
                    "providers/agent-servers/{}/{}",
                    provider_id, version
                ));
                install_managed_python_provider(
                    state,
                    install_id,
                    &provider_id,
                    package,
                    version,
                    entrypoint,
                    resolve_install_args(args),
                    target,
                    &mut stage,
                )
                .await?
            }
            provider_matrix::ProviderInstall::Archive {
                version,
                args,
                targets,
            } => {
                if provider_matrix::normalize_version(version)
                    != provider_matrix::normalize_version(&release.version)
                {
                    anyhow::bail!(
                        "provider matrix version mismatch for {provider_id}: release={} install={}",
                        release.version,
                        version
                    );
                }
                let target_entry = targets.get(resolved_target_key).ok_or_else(|| {
                    anyhow::anyhow!(
                        "unsupported provider target {provider_id}: {resolved_target_key}"
                    )
                })?;
                error_package = Some(target_entry.url.clone());
                error_version = Some(version.clone());
                error_install_dir_rel = Some(format!(
                    "providers/agent-servers/{}/{}",
                    provider_id, version
                ));
                let managed = install_managed_archive_provider(
                    state,
                    install_id,
                    &provider_id,
                    version,
                    &target_entry.url,
                    target_entry.sha256.as_deref(),
                    map_archive_kind(target_entry.archive),
                    &target_entry.bin_path,
                    resolve_install_args(args),
                    target,
                    &mut stage,
                )
                .await?;
                if archive_bin_requires_node_runtime(
                    &target_entry.bin_path,
                    Path::new(&managed.command),
                ) {
                    stage = "node";
                    for dependency_target in
                        node_runtime_dependency_targets_for_install_target(target, std::env::consts::OS)
                    {
                        let node = ensure_node_runtime(
                            state,
                            install_id,
                            &provider_id,
                            &state.core.data_root,
                            dependency_target,
                        )
                        .await
                        .context("ensuring managed Node runtime for archive provider")?;
                        let dep_id = node_runtime_dependency_id(dependency_target);
                        if !dependency_ids.contains(&dep_id) {
                            dependency_ids.push(dep_id.clone());
                        }
                        implicit_managed_dependencies.push((
                            dep_id,
                            node_runtime_dependency_metadata(
                                &state.core.data_root,
                                &node,
                                dependency_target,
                            ),
                        ));
                    }
                }
                managed
            }
        };

        stage = "inspect";
        ensure_install_not_cancelled(state, install_id).await?;
        emit_install(
            state,
            install_id,
            &provider_id,
            InstallEventLevel::Info,
            "inspect",
            "Verifying provider install".to_string(),
            None,
            None,
            None,
        )
        .await;

        let adapter: std::sync::Arc<Tier1CrpAdapter> = std::sync::Arc::new(
            Tier1CrpAdapter::from_raw(&provider_id, managed.command.clone(), managed.args.clone()),
        );

        // Refresh the in-memory adapter so new Sessions use the managed install.
        {
            let mut map = state.providers.adapters.lock().await;
            map.insert(provider_id.clone(), adapter.clone());
        }

        stage = "refresh";
        ensure_install_not_cancelled(state, install_id).await?;
        let mut status_cfg = load_agent_server_config(&state.core.data_root)
            .await
            .unwrap_or_default();
        for (dependency_id, metadata) in &implicit_managed_dependencies {
            status_cfg
                .managed_installs
                .insert(dependency_id.clone(), metadata.clone());
        }
        status_cfg
            .managed_installs
            .insert(provider_id.clone(), managed.meta.clone());
        refresh_provider_statuses_with_cfg(state, status_cfg).await?;

        let status = state
            .providers
            .statuses
            .lock()
            .await
            .get(&provider_id)
            .cloned();
        if let Some(status) = status {
            if !status.installed
                || !matches!(status.health, ctx_providers::adapters::ProviderHealth::Ok)
            {
                anyhow::bail!(
                    "install completed but provider is not healthy: {}",
                    status.diagnostics.join("; ")
                );
            }
        }

        stage = "registry";
        ensure_install_not_cancelled(state, install_id).await?;
        emit_install(
            state,
            install_id,
            &provider_id,
            InstallEventLevel::Info,
            "registry",
            "Writing managed install registry".to_string(),
            None,
            None,
            None,
        )
        .await;

        let mut cfg = load_agent_server_config(&state.core.data_root)
            .await
            .context("loading managed install registry")?;
        for (dependency_id, metadata) in &implicit_managed_dependencies {
            cfg.managed_installs
                .insert(dependency_id.clone(), metadata.clone());
        }
        cfg.managed_installs
            .insert(provider_id.clone(), managed.meta.clone());
        cfg.providers.insert(
            provider_id.clone(),
            AgentServerCommand {
                command: managed.command.clone(),
                args: managed.args.clone(),
                dependencies: dependency_ids.clone(),
                managed: Some(managed.meta.clone()),
            },
        );
        save_agent_server_config(&state.core.data_root, &cfg)
            .await
            .context("saving managed install registry")?;

        emit_install(
            state,
            install_id,
            &provider_id,
            InstallEventLevel::Success,
            "registry",
            "Wrote managed install registry".to_string(),
            None,
            None,
            None,
        )
        .await;

        emit_install(
            state,
            install_id,
            &provider_id,
            InstallEventLevel::Success,
            "done",
            "Install complete".to_string(),
            None,
            None,
            None,
        )
        .await;
        Ok(())
    }
    .await;

    if let Err(e) = &res {
        let error_code = classify_install_error(stage, e);
        emit_install_with_code(
            state,
            install_id,
            &provider_id,
            InstallEventLevel::Error,
            "error",
            truncate_for_storage(&format!("{e:#}"), INSTALL_EVENT_ERROR_MAX_LEN),
            None,
            None,
            None,
            Some(error_code),
        )
        .await;
        update_registry_last_error(
            &state.core.data_root,
            &provider_id,
            stage,
            e,
            error_code,
            error_package.as_deref(),
            error_version.as_deref(),
            error_install_dir_rel.clone(),
            Some(target),
        )
        .await;
    }

    res
}

#[allow(clippy::too_many_arguments)]
async fn emit_install(
    state: &AppState,
    install_id: Option<InstallId>,
    provider_id: &str,
    level: InstallEventLevel,
    stage: &str,
    message: String,
    bytes: Option<u64>,
    total_bytes: Option<u64>,
    attempt: Option<u32>,
) {
    emit_install_with_code(
        state,
        install_id,
        provider_id,
        level,
        stage,
        message,
        bytes,
        total_bytes,
        attempt,
        None,
    )
    .await;
}

#[allow(clippy::too_many_arguments)]
async fn emit_install_with_code(
    state: &AppState,
    install_id: Option<InstallId>,
    provider_id: &str,
    level: InstallEventLevel,
    stage: &str,
    message: String,
    bytes: Option<u64>,
    total_bytes: Option<u64>,
    attempt: Option<u32>,
    error_code: Option<InstallErrorCode>,
) {
    let Some(install_id) = install_id else {
        return;
    };
    let target = state
        .get_install_info(install_id)
        .await
        .and_then(|info| info.target);
    state
        .emit_install_event(
            install_id,
            InstallProgressEvent {
                install_id,
                provider_id: provider_id.to_string(),
                target,
                at: Utc::now(),
                stage: stage.to_string(),
                message,
                level,
                bytes,
                total_bytes,
                attempt,
                error_code,
            },
        )
        .await;
}

async fn ensure_install_not_cancelled(
    state: &AppState,
    install_id: Option<InstallId>,
) -> Result<()> {
    let Some(install_id) = install_id else {
        return Ok(());
    };
    if state.is_install_cancelled(install_id).await {
        anyhow::bail!("install canceled by user");
    }
    Ok(())
}

fn classify_install_error(stage: &str, err: &anyhow::Error) -> InstallErrorCode {
    let text = format!("{err:#}").to_ascii_lowercase();
    if text.contains("install canceled by user") {
        return InstallErrorCode::Cancelled;
    }
    if text.contains("invalid install target") {
        return InstallErrorCode::InvalidTarget;
    }
    if text.contains("unsupported provider target")
        || text.contains("unsupported dependency target")
        || text.contains("is not supported for")
    {
        return InstallErrorCode::UnsupportedTarget;
    }
    if text.contains("checksum mismatch") {
        return InstallErrorCode::ChecksumMismatch;
    }
    if text.contains("timed out") {
        return InstallErrorCode::Timeout;
    }
    if stage == "refresh" || text.contains("not healthy") {
        return InstallErrorCode::HealthCheckFailed;
    }
    if stage == "registry"
        || text.contains("managed install registry")
        || text.contains("saving lsp server config")
    {
        return InstallErrorCode::RegistryWriteFailed;
    }
    if text.contains("matrix version mismatch") || text.contains("no compatible release") {
        return InstallErrorCode::MatrixMismatch;
    }
    if text.contains("download")
        || text.contains("http error")
        || text.contains("sending request")
        || text.contains("streaming download")
    {
        return InstallErrorCode::DownloadFailed;
    }
    if text.contains("install failed")
        || text.contains("process")
        || text.contains("command")
        || text.contains("pip")
        || text.contains("npm")
    {
        return InstallErrorCode::CommandFailed;
    }
    InstallErrorCode::Unknown
}

#[allow(clippy::too_many_arguments)]
async fn update_registry_last_error(
    data_root: &Path,
    provider_id: &str,
    stage: &str,
    err: &anyhow::Error,
    code: InstallErrorCode,
    package: Option<&str>,
    version: Option<&str>,
    install_dir_rel: Option<String>,
    target: Option<InstallTarget>,
) {
    let mut cfg = load_agent_server_config(data_root)
        .await
        .unwrap_or_default();
    let install_dir_rel_clone = install_dir_rel.clone();
    let mut meta =
        cfg.managed_installs
            .get(provider_id)
            .cloned()
            .unwrap_or(ManagedInstallMetadata {
                package: package.map(|s| s.to_string()),
                version: version.map(|s| s.to_string()),
                target,
                install_dir_rel: install_dir_rel_clone,
                bin_dir_rel: None,
                last_success_at: None,
                last_error: None,
            });
    if meta.package.is_none() {
        meta.package = package.map(|s| s.to_string());
    }
    if meta.version.is_none() {
        meta.version = version.map(|s| s.to_string());
    }
    if meta.install_dir_rel.is_none() {
        meta.install_dir_rel = install_dir_rel;
    }
    if meta.target.is_none() {
        meta.target = target;
    }

    meta.last_error = Some(ManagedInstallError {
        at: Utc::now().to_rfc3339(),
        stage: stage.to_string(),
        message: truncate_for_storage(&format!("{err:#}"), LAST_ERROR_MAX_LEN),
        code: Some(code),
    });
    cfg.managed_installs
        .insert(provider_id.to_string(), meta.clone());

    if let Some(entry) = cfg.providers.get_mut(provider_id) {
        entry.managed = Some(meta);
    }

    let _ = save_agent_server_config(data_root, &cfg).await;
}

async fn repair_install_dir(
    install_id: Option<InstallId>,
    state: &AppState,
    provider_id: &str,
    install_dir: &Path,
    expected_entrypoint_rel: &str,
) -> Result<()> {
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

    // Repair semantics: remove obviously-corrupted partial installs.
    if install_dir.exists() {
        let expected = install_dir.join(expected_entrypoint_rel);
        let node_modules = install_dir.join("node_modules");
        if !node_modules.exists() || !expected.exists() {
            tokio::fs::remove_dir_all(install_dir).await.ok();
        }
    }
    tokio::fs::create_dir_all(install_dir)
        .await
        .with_context(|| format!("creating install dir: {}", install_dir.display()))?;
    Ok(())
}

pub async fn refresh_provider_statuses(state: &AppState) -> Result<()> {
    let cfg = load_agent_server_config(&state.core.data_root)
        .await
        .unwrap_or_default();
    refresh_provider_statuses_with_cfg(state, cfg).await
}

async fn refresh_provider_statuses_with_cfg(
    state: &AppState,
    cfg: AgentServerConfigFile,
) -> Result<()> {
    let matrix =
        provider_matrix::load_matrix_cached(&state.core.data_root, &state.providers.matrix_cache)
            .await;

    let map = state.providers.adapters.lock().await;
    let mut statuses = HashMap::new();
    for (id, adapter) in map.iter() {
        match adapter.inspect().await {
            Ok(mut status) => {
                apply_managed_install_details(&mut status, &cfg);
                if let Some(entry) = provider_matrix::get_entry(&matrix, id) {
                    provider_matrix::apply_matrix_to_status(
                        &state.core.data_root,
                        &cfg,
                        entry,
                        &mut status,
                    )
                    .await;
                }
                statuses.insert(id.clone(), status);
            }
            Err(e) => {
                statuses.insert(
                    id.clone(),
                    ctx_providers::adapters::ProviderStatus {
                        provider_id: id.clone(),
                        installed: false,
                        detected_path: None,
                        version: None,
                        capabilities: None,
                        health: ctx_providers::adapters::ProviderHealth::Error,
                        diagnostics: vec![e.to_string()],
                        details: HashMap::new(),
                    },
                );
            }
        }
    }
    drop(map);
    *state.providers.statuses.lock().await = statuses;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct NodeRuntime {
    pub node_root: PathBuf,
    pub node_bin: PathBuf,
    pub npm_cli_js: PathBuf,
}

#[derive(Debug, Clone, Copy)]
struct NodeRuntimeTarget {
    dist_target: &'static str,
    is_windows: bool,
}

#[derive(Debug, Clone)]
pub struct PythonRuntime {
    pub python_root: PathBuf,
    pub python_bin: PathBuf,
}

fn install_dir_rel(data_root: &Path, install_dir: &Path) -> String {
    install_dir
        .strip_prefix(data_root)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| install_dir.to_string_lossy().to_string())
}

fn install_target_dir_component(target: InstallTarget) -> Option<&'static str> {
    match target {
        InstallTarget::Host => None,
        InstallTarget::Container => Some("container"),
        InstallTarget::LinuxAarch64 => Some("linux-aarch64"),
        InstallTarget::LinuxX8664 => Some("linux-x86_64"),
    }
}

fn install_dir_for_provider(
    data_root: &Path,
    provider_id: &str,
    version: &str,
    target: InstallTarget,
) -> PathBuf {
    let mut out = data_root
        .join("providers")
        .join("agent-servers")
        .join(provider_id)
        .join(version);
    if let Some(component) = install_target_dir_component(target) {
        out = out.join(component);
    }
    out
}

pub(crate) async fn ensure_node_runtime(
    state: &AppState,
    install_id: Option<InstallId>,
    provider_id: &str,
    data_root: &Path,
    target: InstallTarget,
) -> Result<NodeRuntime> {
    let node_target = node_runtime_target_for_install_target(target)?;
    let target_label = node_target.dist_target;
    if matches!(target, InstallTarget::Host) {
        if let Some(bundled) = bundled_assets::bundled_node_runtime() {
            if bundled.version == NODE_VERSION {
                if let Some(npm_cli_js) = bundled.npm_cli.clone() {
                    emit_install(
                        state,
                        install_id,
                        provider_id,
                        InstallEventLevel::Info,
                        "node",
                        format!("Using bundled Node runtime v{NODE_VERSION} ({target_label})"),
                        None,
                        None,
                        None,
                    )
                    .await;
                    return Ok(NodeRuntime {
                        node_root: bundled.root,
                        node_bin: bundled.bin,
                        npm_cli_js,
                    });
                }
            } else {
                tracing::warn!(
                    "bundled Node runtime version {} does not match expected {}",
                    bundled.version,
                    NODE_VERSION
                );
            }
        }
    }
    let folder = format!("node-v{NODE_VERSION}-{}", node_target.dist_target);
    let node_root = data_root.join("runtimes").join("node").join(&folder);
    let (node_bin, npm_cli_js) = node_runtime_paths(&node_root, node_target.is_windows);

    if node_bin.exists() && npm_cli_js.exists() {
        emit_install(
            state,
            install_id,
            provider_id,
            InstallEventLevel::Info,
            "node",
            format!("Using existing Node runtime v{NODE_VERSION} ({target_label})"),
            None,
            None,
            None,
        )
        .await;
        return Ok(NodeRuntime {
            node_root,
            node_bin,
            npm_cli_js,
        });
    }

    // `install_all` runs provider installs concurrently; without a lock, multiple tasks can race by
    // deleting/recreating the same `.extract` directory and corrupting the unpack.
    let _lock = node_runtime_install_lock().lock().await;

    // Another task may have completed the install while we waited.
    if node_bin.exists() && npm_cli_js.exists() {
        emit_install(
            state,
            install_id,
            provider_id,
            InstallEventLevel::Info,
            "node",
            format!("Using existing Node runtime v{NODE_VERSION} ({target_label})"),
            None,
            None,
            None,
        )
        .await;
        return Ok(NodeRuntime {
            node_root,
            node_bin,
            npm_cli_js,
        });
    }

    if let Some(parent) = node_root.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let (archive_ext, archive_label) = if node_target.is_windows {
        ("zip", "zip")
    } else {
        ("tar.gz", "tar_gz")
    };
    let url = format!("https://nodejs.org/dist/v{NODE_VERSION}/{folder}.{archive_ext}");
    let tmp = data_root
        .join("runtimes")
        .join("node")
        .join(format!("{folder}.{archive_ext}"));
    emit_install(
        state,
        install_id,
        provider_id,
        InstallEventLevel::Info,
        "node_download",
        format!("Downloading Node runtime from {url}"),
        None,
        None,
        None,
    )
    .await;
    download_to_file(state, install_id, provider_id, "node_download", &url, &tmp).await?;

    let extract_root = data_root
        .join("runtimes")
        .join("node")
        .join(format!("{folder}.extract"));
    if extract_root.exists() {
        tokio::fs::remove_dir_all(&extract_root).await.ok();
    }
    tokio::fs::create_dir_all(&extract_root).await?;

    emit_install(
        state,
        install_id,
        provider_id,
        InstallEventLevel::Info,
        "node_extract",
        "Extracting Node runtime".to_string(),
        None,
        None,
        None,
    )
    .await;

    let tmp2 = tmp.clone();
    let extract_root2 = extract_root.clone();
    tokio::task::spawn_blocking(move || -> Result<()> {
        if archive_label == "zip" {
            extract_zip_to_dir(&tmp2, &extract_root2)?;
        } else {
            let tar_gz = std::fs::File::open(&tmp2)?;
            let dec = flate2::read::GzDecoder::new(tar_gz);
            let mut archive = tar::Archive::new(dec);
            archive.unpack(&extract_root2)?;
        }
        Ok(())
    })
    .await??;

    // Node archives contain a single top-level folder named `node-vX.Y.Z-<target>`.
    let extracted = extract_root.join(&folder);
    if !extracted.exists() {
        anyhow::bail!(
            "node extraction failed: missing {folder} in {}",
            extract_root.display()
        );
    }

    if node_root.exists() {
        tokio::fs::remove_dir_all(&node_root).await.ok();
    }
    tokio::fs::rename(&extracted, &node_root).await?;
    tokio::fs::remove_dir_all(&extract_root).await.ok();
    tokio::fs::remove_file(&tmp).await.ok();

    if !node_bin.exists() || !npm_cli_js.exists() {
        anyhow::bail!(
            "node runtime incomplete after install (node: {}, npm: {})",
            node_bin.display(),
            npm_cli_js.display()
        );
    }

    emit_install(
        state,
        install_id,
        provider_id,
        InstallEventLevel::Success,
        "node_extract",
        "Node runtime ready".to_string(),
        None,
        None,
        None,
    )
    .await;

    Ok(NodeRuntime {
        node_root,
        node_bin,
        npm_cli_js,
    })
}

fn node_runtime_paths(node_root: &Path, is_windows: bool) -> (PathBuf, PathBuf) {
    if is_windows {
        (
            node_root.join("node.exe"),
            node_root
                .join("node_modules")
                .join("npm")
                .join("bin")
                .join("npm-cli.js"),
        )
    } else {
        (
            node_root.join("bin").join("node"),
            node_root
                .join("lib")
                .join("node_modules")
                .join("npm")
                .join("bin")
                .join("npm-cli.js"),
        )
    }
}

fn node_runtime_target_for_install_target(target: InstallTarget) -> Result<NodeRuntimeTarget> {
    match target {
        InstallTarget::Host => {
            node_runtime_target_for_os_arch(std::env::consts::OS, std::env::consts::ARCH)
        }
        InstallTarget::Container => {
            node_runtime_target_for_os_arch("linux", std::env::consts::ARCH)
        }
        InstallTarget::LinuxAarch64 => node_runtime_target_for_os_arch("linux", "aarch64"),
        InstallTarget::LinuxX8664 => node_runtime_target_for_os_arch("linux", "x86_64"),
    }
}

fn node_runtime_dependency_targets_for_install_target(
    target: InstallTarget,
    host_os: &str,
) -> Vec<InstallTarget> {
    let mut targets = vec![target];
    if matches!(target, InstallTarget::Container) && host_os != "linux" {
        targets.push(InstallTarget::Host);
    }
    targets
}

fn node_runtime_target_for_os_arch(os: &str, arch: &str) -> Result<NodeRuntimeTarget> {
    match (os, arch) {
        ("macos", "aarch64") => Ok(NodeRuntimeTarget {
            dist_target: "darwin-arm64",
            is_windows: false,
        }),
        ("macos", "x86_64") => Ok(NodeRuntimeTarget {
            dist_target: "darwin-x64",
            is_windows: false,
        }),
        ("linux", "aarch64") => Ok(NodeRuntimeTarget {
            dist_target: "linux-arm64",
            is_windows: false,
        }),
        ("linux", "x86_64") => Ok(NodeRuntimeTarget {
            dist_target: "linux-x64",
            is_windows: false,
        }),
        ("windows", "x86_64") => Ok(NodeRuntimeTarget {
            dist_target: "win-x64",
            is_windows: true,
        }),
        ("windows", "aarch64") => Ok(NodeRuntimeTarget {
            dist_target: "win-arm64",
            is_windows: true,
        }),
        _ => anyhow::bail!(
            "unsupported platform for managed node install: {os}/{arch}. Supported: macos (aarch64/x86_64), linux (aarch64/x86_64), windows (aarch64/x86_64)."
        ),
    }
}

fn archive_bin_requires_node_runtime(bin_path: &str, installed_bin_path: &Path) -> bool {
    let ext = Path::new(bin_path)
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase());
    if matches!(ext.as_deref(), Some("js") | Some("mjs") | Some("cjs")) {
        return true;
    }
    archive_bin_has_node_shebang(installed_bin_path)
}

fn archive_bin_has_node_shebang(path: &Path) -> bool {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return false,
    };
    let mut reader = std::io::BufReader::new(file);
    let mut first_line = String::new();
    let bytes = match std::io::BufRead::read_line(&mut reader, &mut first_line) {
        Ok(bytes) => bytes,
        Err(_) => return false,
    };
    if bytes == 0 {
        return false;
    }
    shebang_invokes_node(first_line.trim())
}

fn shebang_invokes_node(line: &str) -> bool {
    let Some(shebang) = line.strip_prefix("#!") else {
        return false;
    };
    let mut tokens = shebang.split_whitespace();
    let Some(program) = tokens.next() else {
        return false;
    };
    if shebang_token_is_node(program) {
        return true;
    }
    if !shebang_token_is_env(program) {
        return false;
    }
    for token in tokens {
        if token.starts_with('-') || token.contains('=') {
            continue;
        }
        return shebang_token_is_node(token);
    }
    false
}

fn shebang_token_is_env(token: &str) -> bool {
    let base = shebang_token_basename(token);
    base.eq_ignore_ascii_case("env")
}

fn shebang_token_is_node(token: &str) -> bool {
    let base = shebang_token_basename(token);
    base.eq_ignore_ascii_case("node") || base.eq_ignore_ascii_case("node.exe")
}

fn shebang_token_basename(token: &str) -> &str {
    token
        .trim_matches('"')
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(token)
}

fn node_runtime_dependency_id(target: InstallTarget) -> String {
    format!("runtime-node-{}", target.as_str())
}

fn node_runtime_dependency_metadata(
    data_root: &Path,
    node: &NodeRuntime,
    target: InstallTarget,
) -> ManagedInstallMetadata {
    let bin_dir = node
        .node_bin
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| node.node_root.clone());
    ManagedInstallMetadata {
        package: Some("node-runtime".to_string()),
        version: Some(NODE_VERSION.to_string()),
        target: Some(target),
        install_dir_rel: Some(install_dir_rel(data_root, &node.node_root)),
        bin_dir_rel: Some(install_dir_rel(data_root, &bin_dir)),
        last_success_at: Some(Utc::now().to_rfc3339()),
        last_error: None,
    }
}

async fn ensure_python_runtime(
    state: &AppState,
    install_id: Option<InstallId>,
    provider_id: &str,
    data_root: &Path,
    target: InstallTarget,
) -> Result<PythonRuntime> {
    let target_triple = python_target_triple_for_install_target(target)?;
    if python_target_can_use_bundled_runtime(target) {
        if let Some(bundled) = bundled_assets::bundled_python_runtime() {
            if bundled.version == PYTHON_VERSION {
                emit_install(
                    state,
                    install_id,
                    provider_id,
                    InstallEventLevel::Info,
                    "python",
                    format!("Using bundled Python runtime {PYTHON_VERSION} ({target_triple})"),
                    None,
                    None,
                    None,
                )
                .await;
                return Ok(PythonRuntime {
                    python_root: bundled.root,
                    python_bin: bundled.bin,
                });
            } else {
                tracing::warn!(
                    "bundled Python runtime version {} does not match expected {}",
                    bundled.version,
                    PYTHON_VERSION
                );
            }
        }
    }
    let folder = format!("cpython-{PYTHON_VERSION}+{PYTHON_BUILD_TAG}-{target_triple}");
    let python_root = data_root.join("runtimes").join("python").join(&folder);
    let python_bin = resolve_python_bin(&python_root, target);

    if python_bin.exists() {
        emit_install(
            state,
            install_id,
            provider_id,
            InstallEventLevel::Info,
            "python",
            format!("Using existing Python runtime {PYTHON_VERSION} ({target_triple})"),
            None,
            None,
            None,
        )
        .await;
        return Ok(PythonRuntime {
            python_root,
            python_bin,
        });
    }

    let _lock = python_runtime_install_lock().lock().await;
    let python_bin = resolve_python_bin(&python_root, target);
    if python_bin.exists() {
        emit_install(
            state,
            install_id,
            provider_id,
            InstallEventLevel::Info,
            "python",
            format!("Using existing Python runtime {PYTHON_VERSION} ({target_triple})"),
            None,
            None,
            None,
        )
        .await;
        return Ok(PythonRuntime {
            python_root,
            python_bin,
        });
    }

    if let Some(parent) = python_root.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let asset =
        format!("cpython-{PYTHON_VERSION}+{PYTHON_BUILD_TAG}-{target_triple}-install_only.tar.gz");
    let url = format!(
        "https://github.com/indygreg/python-build-standalone/releases/download/{PYTHON_BUILD_TAG}/{asset}"
    );
    let tmp = data_root.join("runtimes").join("python").join(&asset);

    emit_install(
        state,
        install_id,
        provider_id,
        InstallEventLevel::Info,
        "python_download",
        format!("Downloading Python runtime from {url}"),
        None,
        None,
        None,
    )
    .await;
    download_to_file(
        state,
        install_id,
        provider_id,
        "python_download",
        &url,
        &tmp,
    )
    .await?;

    let extract_root = data_root
        .join("runtimes")
        .join("python")
        .join(format!("{folder}.extract"));
    if extract_root.exists() {
        tokio::fs::remove_dir_all(&extract_root).await.ok();
    }
    tokio::fs::create_dir_all(&extract_root).await?;

    emit_install(
        state,
        install_id,
        provider_id,
        InstallEventLevel::Info,
        "python_extract",
        "Extracting Python runtime".to_string(),
        None,
        None,
        None,
    )
    .await;

    let tmp2 = tmp.clone();
    let extract_root2 = extract_root.clone();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let tar_gz = std::fs::File::open(&tmp2)?;
        let dec = flate2::read::GzDecoder::new(tar_gz);
        let mut archive = tar::Archive::new(dec);
        archive.unpack(&extract_root2)?;
        Ok(())
    })
    .await??;

    let extracted = extract_root.join("python");
    if !extracted.exists() {
        anyhow::bail!(
            "python extraction failed: missing python/ in {}",
            extract_root.display()
        );
    }

    if python_root.exists() {
        tokio::fs::remove_dir_all(&python_root).await.ok();
    }
    tokio::fs::rename(&extracted, &python_root).await?;
    tokio::fs::remove_dir_all(&extract_root).await.ok();
    tokio::fs::remove_file(&tmp).await.ok();

    let python_bin = resolve_python_bin(&python_root, target);
    if !python_bin.exists() {
        anyhow::bail!(
            "python runtime incomplete after install (python: {})",
            python_bin.display()
        );
    }

    emit_install(
        state,
        install_id,
        provider_id,
        InstallEventLevel::Success,
        "python_extract",
        format!("Installed Python runtime {PYTHON_VERSION} ({target_triple})"),
        None,
        None,
        None,
    )
    .await;

    Ok(PythonRuntime {
        python_root,
        python_bin,
    })
}

fn python_target_can_use_bundled_runtime(target: InstallTarget) -> bool {
    matches!(target, InstallTarget::Host)
}

fn python_target_triple_for_install_target(target: InstallTarget) -> Result<&'static str> {
    match target {
        InstallTarget::Host => {
            python_target_triple_for_os_arch(std::env::consts::OS, std::env::consts::ARCH)
        }
        InstallTarget::Container => {
            python_target_triple_for_os_arch("linux", std::env::consts::ARCH)
        }
        InstallTarget::LinuxAarch64 => python_target_triple_for_os_arch("linux", "aarch64"),
        InstallTarget::LinuxX8664 => python_target_triple_for_os_arch("linux", "x86_64"),
    }
}

fn python_target_triple_for_os_arch(os: &str, arch: &str) -> Result<&'static str> {
    match (os, arch) {
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        ("macos", "x86_64") => Ok("x86_64-apple-darwin"),
        ("linux", "aarch64") => Ok("aarch64-unknown-linux-gnu"),
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-gnu"),
        ("windows", "x86_64") => Ok("x86_64-pc-windows-msvc"),
        ("windows", "aarch64") => Ok("aarch64-pc-windows-msvc"),
        _ => anyhow::bail!(
            "unsupported platform for managed python install: {os}/{arch}. Supported: macos (aarch64/x86_64), linux (aarch64/x86_64), windows (aarch64/x86_64)."
        ),
    }
}

fn resolve_python_bin(python_root: &Path, target: InstallTarget) -> PathBuf {
    if target_uses_windows_layout(target) {
        python_root.join("python.exe")
    } else {
        let primary = python_root.join("bin").join("python3");
        if primary.exists() {
            primary
        } else {
            python_root.join("bin").join("python")
        }
    }
}

async fn ensure_python_pip(python: &Path) -> Result<()> {
    let mut pip_check = Command::new(python);
    pip_check
        .arg("-m")
        .arg("pip")
        .arg("--version")
        .kill_on_drop(true);
    let out = run_command_with_timeout(pip_check, Duration::from_secs(60))
        .await
        .context("checking pip availability")?;
    if out.status.success() {
        return Ok(());
    }

    let mut ensure = Command::new(python);
    ensure
        .arg("-m")
        .arg("ensurepip")
        .arg("--upgrade")
        .kill_on_drop(true);
    let out = run_command_with_timeout(ensure, Duration::from_secs(5 * 60))
        .await
        .context("running ensurepip")?;
    if !out.status.success() {
        anyhow::bail!(
            "ensurepip failed status={}\nstdout:\n{}\nstderr:\n{}",
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(())
}

async fn npm_install(
    state: &AppState,
    install_id: Option<InstallId>,
    provider_id: &str,
    node: &NodeRuntime,
    install_dir: &Path,
    package_spec: &str,
    target: InstallTarget,
) -> Result<()> {
    let cache_dir = install_dir.join(".npm-cache");
    tokio::fs::create_dir_all(&cache_dir).await.ok();
    let node_bin_dir = node.node_bin.parent().unwrap_or(node.node_root.as_path());
    let path_sep = if cfg!(windows) { ";" } else { ":" };
    let mut combined_path = std::ffi::OsString::new();
    combined_path.push(node_bin_dir);
    combined_path.push(path_sep);
    if let Some(existing) = std::env::var_os("PATH") {
        combined_path.push(existing);
    }
    let pnpm_bin = if matches!(target, InstallTarget::Host) {
        which::which("pnpm").ok()
    } else {
        None
    };
    let package_manager = if pnpm_bin.is_some() { "pnpm" } else { "npm" };

    for attempt in 1..=RETRY_COUNT {
        ensure_install_not_cancelled(state, install_id).await?;
        emit_install(
            state,
            install_id,
            provider_id,
            InstallEventLevel::Info,
            "npm_install",
            format!("{package_manager} install {package_spec} (attempt {attempt}/{RETRY_COUNT})"),
            None,
            None,
            Some(attempt),
        )
        .await;

        let out = if matches!(target, InstallTarget::Container) {
            container_builder::ensure_builder_ready(&state.core.data_root)
                .await
                .context("ensuring container builder readiness")?;
            let mut argv = Vec::with_capacity(16);
            argv.push(node.node_bin.to_string_lossy().to_string());
            argv.push(node.npm_cli_js.to_string_lossy().to_string());
            argv.push("install".to_string());
            argv.push("--prefix".to_string());
            argv.push(install_dir.to_string_lossy().to_string());
            argv.push("--no-audit".to_string());
            argv.push("--no-fund".to_string());
            argv.push("--silent".to_string());
            argv.push("--ignore-scripts".to_string());
            argv.push(package_spec.to_string());
            let env = vec![
                (
                    "PATH".to_string(),
                    combined_path.to_string_lossy().to_string(),
                ),
                (
                    "npm_config_update_notifier".to_string(),
                    "false".to_string(),
                ),
                ("npm_config_fund".to_string(), "false".to_string()),
                ("npm_config_audit".to_string(), "false".to_string()),
                ("npm_config_progress".to_string(), "false".to_string()),
                (
                    "npm_config_cache".to_string(),
                    cache_dir.to_string_lossy().to_string(),
                ),
                ("npm_config_ignore_scripts".to_string(), "true".to_string()),
            ];
            container_builder::run_command(
                &state.core.data_root,
                install_dir,
                &env,
                &argv,
                NPM_INSTALL_TIMEOUT,
            )
            .await
        } else {
            let mut cmd = if let Some(pnpm) = pnpm_bin.as_ref() {
                let mut cmd = Command::new(pnpm);
                cmd.arg("add")
                    .arg("--dir")
                    .arg(install_dir)
                    .arg("--ignore-scripts")
                    .arg("--lockfile=false")
                    .arg("--reporter")
                    .arg("silent")
                    .arg(package_spec);
                cmd
            } else {
                let mut cmd = Command::new(&node.node_bin);
                cmd.arg(&node.npm_cli_js)
                    .arg("install")
                    .arg("--prefix")
                    .arg(install_dir)
                    .arg("--no-audit")
                    .arg("--no-fund")
                    .arg("--silent")
                    .arg("--ignore-scripts")
                    .arg(package_spec)
                    .env("npm_config_update_notifier", "false")
                    .env("npm_config_fund", "false")
                    .env("npm_config_audit", "false")
                    .env("npm_config_progress", "false")
                    .env("npm_config_cache", cache_dir.clone())
                    .env("npm_config_ignore_scripts", "true");
                cmd
            };
            cmd.env("PATH", combined_path.clone()).kill_on_drop(true);
            run_command_with_timeout(cmd, NPM_INSTALL_TIMEOUT).await
        };
        let out = match out {
            Ok(out) => out,
            Err(e) => {
                emit_install(
                    state,
                    install_id,
                    provider_id,
                    InstallEventLevel::Error,
                    "npm_install",
                    format!("{package_manager} install failed: {e:#}"),
                    None,
                    None,
                    Some(attempt),
                )
                .await;
                if attempt < RETRY_COUNT {
                    tokio::time::sleep(Duration::from_millis(
                        RETRY_BACKOFF_BASE_MS * attempt as u64,
                    ))
                    .await;
                    continue;
                }
                return Err(e.context("running package install"));
            }
        };
        if out.status.success() {
            emit_install(
                state,
                install_id,
                provider_id,
                InstallEventLevel::Success,
                "npm_install",
                format!("{package_manager} install succeeded"),
                None,
                None,
                Some(attempt),
            )
            .await;
            return Ok(());
        }

        let err_txt = format!(
            "{package_manager} install failed ({package_spec}) status={}\nstdout:\n{}\nstderr:\n{}",
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        emit_install(
            state,
            install_id,
            provider_id,
            InstallEventLevel::Error,
            "npm_install",
            truncate_for_storage(&err_txt, INSTALL_EVENT_ERROR_MAX_LEN),
            None,
            None,
            Some(attempt),
        )
        .await;
        if attempt < RETRY_COUNT {
            tokio::time::sleep(Duration::from_millis(
                RETRY_BACKOFF_BASE_MS * attempt as u64,
            ))
            .await;
        }
    }

    anyhow::bail!(
        "{package_manager} install failed after {RETRY_COUNT} attempts ({package_spec}). Try again, or check your network / npm registry access."
    );
}

async fn download_to_file(
    state: &AppState,
    install_id: Option<InstallId>,
    provider_id: &str,
    stage: &str,
    url: &str,
    path: &Path,
) -> Result<()> {
    for attempt in 1..=RETRY_COUNT {
        ensure_install_not_cancelled(state, install_id).await?;
        let attempt_res: Result<()> = async {
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent).await.ok();
            }

            if url.starts_with("file://") {
                let u = url::Url::parse(url).context("parsing file:// url")?;
                let src = u
                    .to_file_path()
                    .map_err(|_| anyhow::anyhow!("invalid file url: {url}"))?;
                tokio::fs::copy(&src, path)
                    .await
                    .with_context(|| format!("copying {} -> {}", src.display(), path.display()))?;
            } else {
                let client = reqwest::Client::builder()
                    .connect_timeout(Duration::from_secs(15))
                    .timeout(DOWNLOAD_TIMEOUT)
                    .build()
                    .context("building http client")?;
                let existing_len = tokio::fs::metadata(path)
                    .await
                    .map(|meta| meta.len())
                    .unwrap_or(0);
                let mut request = client.get(url);
                if existing_len > 0 {
                    use reqwest::header::RANGE;
                    request = request.header(RANGE, format!("bytes={existing_len}-"));
                    emit_install(
                        state,
                        install_id,
                        provider_id,
                        InstallEventLevel::Info,
                        stage,
                        format!("resuming download from byte {existing_len}"),
                        Some(existing_len),
                        None,
                        Some(attempt),
                    )
                    .await;
                }

                let resp = request.send().await.context("sending request")?;
                let status = resp.status();
                if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
                    tokio::fs::remove_file(path).await.ok();
                    anyhow::bail!("server rejected ranged resume request");
                }
                let resp = resp.error_for_status().context("http error")?;

                let (resumed, total) =
                    resolve_download_resume(existing_len, status, resp.content_length());
                let mut stream = resp.bytes_stream();
                let mut file = if resumed {
                    tokio::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(path)
                        .await
                        .with_context(|| {
                            format!("opening download target for append: {}", path.display())
                        })?
                } else {
                    if existing_len > 0 {
                        emit_install(
                            state,
                            install_id,
                            provider_id,
                            InstallEventLevel::Warning,
                            stage,
                            "server does not support resume; restarting download from byte 0"
                                .to_string(),
                            None,
                            total,
                            Some(attempt),
                        )
                        .await;
                    }
                    tokio::fs::File::create(path)
                        .await
                        .with_context(|| format!("creating download target: {}", path.display()))?
                };
                use futures::StreamExt;
                use tokio::io::AsyncWriteExt;

                let mut downloaded: u64 = if resumed { existing_len } else { 0 };
                while let Some(chunk) = stream.next().await {
                    ensure_install_not_cancelled(state, install_id).await?;
                    let bytes = chunk.context("streaming download")?;
                    downloaded += bytes.len() as u64;
                    file.write_all(&bytes).await.context("writing download")?;
                    emit_install(
                        state,
                        install_id,
                        provider_id,
                        InstallEventLevel::Info,
                        stage,
                        "downloading…".to_string(),
                        Some(downloaded),
                        total,
                        Some(attempt),
                    )
                    .await;
                }
                file.flush().await.context("flushing download")?;
                emit_install(
                    state,
                    install_id,
                    provider_id,
                    InstallEventLevel::Success,
                    stage,
                    "download complete".to_string(),
                    Some(downloaded),
                    total,
                    Some(attempt),
                )
                .await;
            }
            Ok(())
        }
        .await;

        match attempt_res {
            Ok(()) => return Ok(()),
            Err(e) => {
                emit_install(
                    state,
                    install_id,
                    provider_id,
                    InstallEventLevel::Error,
                    stage,
                    format!("download failed: {e}"),
                    None,
                    None,
                    Some(attempt),
                )
                .await;
                if attempt < RETRY_COUNT {
                    tokio::time::sleep(Duration::from_millis(
                        RETRY_BACKOFF_BASE_MS * attempt as u64,
                    ))
                    .await;
                    continue;
                }
                return Err(e);
            }
        }
    }

    Ok(())
}

fn resolve_download_resume(
    existing_len: u64,
    status: reqwest::StatusCode,
    content_length: Option<u64>,
) -> (bool, Option<u64>) {
    let resumed = existing_len > 0 && status == reqwest::StatusCode::PARTIAL_CONTENT;
    let total = match content_length {
        Some(remaining) if resumed => Some(existing_len.saturating_add(remaining)),
        other => other,
    };
    (resumed, total)
}

async fn run_command_with_timeout(mut cmd: Command, dur: Duration) -> Result<std::process::Output> {
    let child = cmd.spawn().context("spawning process")?;
    let wait = async move { child.wait_with_output().await };
    match timeout(dur, wait).await {
        Ok(res) => Ok(res.context("waiting for process")?),
        Err(_) => anyhow::bail!("process timed out after {}s", dur.as_secs()),
    }
}

fn sanitize_npm_package_for_path(pkg: &str) -> String {
    pkg.trim()
        .trim_start_matches('@')
        .replace(['/', '\\'], "__")
}

async fn npm_install_one(
    state: &AppState,
    install_id: Option<InstallId>,
    provider_id: &str,
    node: &NodeRuntime,
    install_dir: &Path,
    package: &str,
    version: &str,
) -> Result<()> {
    let package_spec = format!("{package}@{version}");
    npm_install(
        state,
        install_id,
        provider_id,
        node,
        install_dir,
        &package_spec,
        InstallTarget::Host,
    )
    .await
}

async fn npm_dependency_matches(install_dir: &Path, package: &str, version: &str) -> Result<bool> {
    let pkg_dir = install_dir.join("node_modules").join(package);
    let pkg_json_path = pkg_dir.join("package.json");
    if !pkg_json_path.exists() {
        return Ok(false);
    }
    let txt = tokio::fs::read_to_string(&pkg_json_path)
        .await
        .with_context(|| format!("reading {}", pkg_json_path.display()))?;
    let v: serde_json::Value = serde_json::from_str(&txt).context("parsing package.json")?;
    Ok(v.get("version")
        .and_then(|v| v.as_str())
        .map(|v| v == version)
        .unwrap_or(false))
}

async fn resolve_node_package_bin(
    install_dir: &Path,
    package: &str,
    preferred_bin_name: Option<&str>,
) -> Result<PathBuf> {
    let pkg_dir = install_dir.join("node_modules").join(package);
    let pkg_json_path = pkg_dir.join("package.json");
    let txt = tokio::fs::read_to_string(&pkg_json_path)
        .await
        .with_context(|| format!("reading {}", pkg_json_path.display()))?;
    let v: serde_json::Value = serde_json::from_str(&txt).context("parsing package.json")?;
    let bin = v.get("bin").context("package.json missing bin")?;
    let entry_rel = match bin {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Object(map) => {
            if let Some(preferred) = preferred_bin_name {
                if let Some(v) = map.get(preferred).and_then(|v| v.as_str()) {
                    v.to_string()
                } else if map.len() == 1 {
                    map.values()
                        .next()
                        .and_then(|v| v.as_str())
                        .context("bin map invalid")?
                        .to_string()
                } else {
                    anyhow::bail!(
                        "bin {preferred} not found in {} (available: {})",
                        package,
                        map.keys().cloned().collect::<Vec<_>>().join(", ")
                    );
                }
            } else if map.len() == 1 {
                map.values()
                    .next()
                    .and_then(|v| v.as_str())
                    .context("bin map invalid")?
                    .to_string()
            } else {
                anyhow::bail!(
                    "multiple bins in {} but no preferred bin specified",
                    package
                );
            }
        }
        _ => anyhow::bail!("package.json bin has unsupported type"),
    };
    Ok(pkg_dir.join(entry_rel))
}

async fn install_lsp_server_impl(
    state: &AppState,
    server_id: &str,
    install_id: Option<InstallId>,
) -> Result<()> {
    let data_root = state.core.data_root.clone();
    let provider_id = format!("lsp:{server_id}");

    if !is_supported_managed_lsp_server(server_id) {
        anyhow::bail!("unsupported managed lsp server: {server_id}");
    }

    let mut stage: &'static str = "start";

    let res: Result<()> = async {
        emit_install(
            state,
            install_id,
            &provider_id,
            InstallEventLevel::Info,
            "start",
            format!("Installing managed LSP server: {server_id}"),
            None,
            None,
            None,
        )
        .await;

        stage = "node";
        let node = ensure_node_runtime(
            state,
            install_id,
            &provider_id,
            &data_root,
            InstallTarget::Host,
        )
            .await
            .context("ensuring managed Node runtime")?;

        stage = "registry_load";
        let mut cfg = load_lsp_server_config(&data_root).await.unwrap_or_default();

        let mut register = |key: &str,
                            package: &str,
                            version: &str,
                            install_dir: &Path,
                            entry: PathBuf,
                            args: Vec<String>| {
            let install_dir_rel = install_dir_rel(&data_root, install_dir);
            let meta = ManagedInstallMetadata {
                package: Some(package.to_string()),
                version: Some(version.to_string()),
                target: None,
                install_dir_rel: Some(install_dir_rel),
                bin_dir_rel: None,
                last_success_at: Some(Utc::now().to_rfc3339()),
                last_error: None,
            };
            cfg.managed_installs.insert(key.to_string(), meta.clone());
            cfg.servers.insert(
                key.to_string(),
                AgentServerCommand {
                    command: node.node_bin.to_string_lossy().to_string(),
                    args: std::iter::once(entry.to_string_lossy().to_string())
                        .chain(args.into_iter())
                        .collect(),
                    dependencies: Vec::new(),
                    managed: Some(meta),
                },
            );
        };

        stage = "install";
        match server_id {
            "typescript" => {
                let package = "typescript-language-server";
                let package_dir = data_root
                    .join("lsp")
                    .join("node")
                    .join(sanitize_npm_package_for_path(package))
                    .join(TYPESCRIPT_LS_VERSION);
                tokio::fs::create_dir_all(&package_dir).await.ok();

                emit_install(
                    state,
                    install_id,
                    &provider_id,
                    InstallEventLevel::Info,
                    "npm_install",
                    format!("Installing {package}@{TYPESCRIPT_LS_VERSION} (+ typescript@{TYPESCRIPT_VERSION})"),
                    None,
                    None,
                    None,
                )
                .await;

                npm_install_one(
                    state,
                    install_id,
                    &provider_id,
                    &node,
                    &package_dir,
                    "typescript",
                    TYPESCRIPT_VERSION,
                )
                .await
                .context("installing typescript")?;
                npm_install_one(
                    state,
                    install_id,
                    &provider_id,
                    &node,
                    &package_dir,
                    package,
                    TYPESCRIPT_LS_VERSION,
                )
                .await
                .context("installing typescript-language-server")?;

                let entry =
                    resolve_node_package_bin(&package_dir, package, Some("typescript-language-server"))
                        .await
                        .context("resolving typescript-language-server bin")?;
                register(
                    "typescript",
                    package,
                    TYPESCRIPT_LS_VERSION,
                    &package_dir,
                    entry,
                    vec!["--stdio".to_string()],
                );
            }
            "python" => {
                let package = "pyright";
                let package_dir = data_root
                    .join("lsp")
                    .join("node")
                    .join(sanitize_npm_package_for_path(package))
                    .join(PYRIGHT_VERSION);
                tokio::fs::create_dir_all(&package_dir).await.ok();

                emit_install(
                    state,
                    install_id,
                    &provider_id,
                    InstallEventLevel::Info,
                    "npm_install",
                    format!("Installing {package}@{PYRIGHT_VERSION}"),
                    None,
                    None,
                    None,
                )
                .await;

                npm_install_one(
                    state,
                    install_id,
                    &provider_id,
                    &node,
                    &package_dir,
                    package,
                    PYRIGHT_VERSION,
                )
                .await
                .context("installing pyright")?;

                let entry =
                    resolve_node_package_bin(&package_dir, package, Some("pyright-langserver"))
                        .await
                        .context("resolving pyright-langserver bin")?;
                register(
                    "python",
                    package,
                    PYRIGHT_VERSION,
                    &package_dir,
                    entry,
                    vec!["--stdio".to_string()],
                );
            }
            "html" | "css" | "json" => {
                // Shared install for html/css/json.
                let package = "vscode-langservers-extracted";
                let package_dir = data_root
                    .join("lsp")
                    .join("node")
                    .join(sanitize_npm_package_for_path(package))
                    .join(VSCODE_LANGSERVERS_EXTRACTED_VERSION);
                tokio::fs::create_dir_all(&package_dir).await.ok();

                emit_install(
                    state,
                    install_id,
                    &provider_id,
                    InstallEventLevel::Info,
                    "npm_install",
                    format!("Installing {package}@{VSCODE_LANGSERVERS_EXTRACTED_VERSION} (html/css/json)"),
                    None,
                    None,
                    None,
                )
                .await;

                npm_install_one(
                    state,
                    install_id,
                    &provider_id,
                    &node,
                    &package_dir,
                    package,
                    VSCODE_LANGSERVERS_EXTRACTED_VERSION,
                )
                .await
                .context("installing vscode-langservers-extracted")?;

                let html_entry =
                    resolve_node_package_bin(&package_dir, package, Some("vscode-html-language-server"))
                        .await
                        .context("resolving vscode-html-language-server")?;
                register(
                    "html",
                    package,
                    VSCODE_LANGSERVERS_EXTRACTED_VERSION,
                    &package_dir,
                    html_entry,
                    vec!["--stdio".to_string()],
                );

                let css_entry =
                    resolve_node_package_bin(&package_dir, package, Some("vscode-css-language-server"))
                        .await
                        .context("resolving vscode-css-language-server")?;
                register(
                    "css",
                    package,
                    VSCODE_LANGSERVERS_EXTRACTED_VERSION,
                    &package_dir,
                    css_entry,
                    vec!["--stdio".to_string()],
                );

                let json_entry =
                    resolve_node_package_bin(&package_dir, package, Some("vscode-json-language-server"))
                        .await
                        .context("resolving vscode-json-language-server")?;
                register(
                    "json",
                    package,
                    VSCODE_LANGSERVERS_EXTRACTED_VERSION,
                    &package_dir,
                    json_entry,
                    vec!["--stdio".to_string()],
                );
            }
            "yaml" => {
                let package = "yaml-language-server";
                let package_dir = data_root
                    .join("lsp")
                    .join("node")
                    .join(sanitize_npm_package_for_path(package))
                    .join(YAML_LANGUAGE_SERVER_VERSION);
                tokio::fs::create_dir_all(&package_dir).await.ok();

                emit_install(
                    state,
                    install_id,
                    &provider_id,
                    InstallEventLevel::Info,
                    "npm_install",
                    format!("Installing {package}@{YAML_LANGUAGE_SERVER_VERSION}"),
                    None,
                    None,
                    None,
                )
                .await;

                npm_install_one(
                    state,
                    install_id,
                    &provider_id,
                    &node,
                    &package_dir,
                    package,
                    YAML_LANGUAGE_SERVER_VERSION,
                )
                .await
                .context("installing yaml-language-server")?;

                let entry =
                    resolve_node_package_bin(&package_dir, package, Some("yaml-language-server"))
                        .await
                        .context("resolving yaml-language-server")?;
                register(
                    "yaml",
                    package,
                    YAML_LANGUAGE_SERVER_VERSION,
                    &package_dir,
                    entry,
                    vec!["--stdio".to_string()],
                );
            }
            "bash" => {
                let package = "bash-language-server";
                let package_dir = data_root
                    .join("lsp")
                    .join("node")
                    .join(sanitize_npm_package_for_path(package))
                    .join(BASH_LANGUAGE_SERVER_VERSION);
                tokio::fs::create_dir_all(&package_dir).await.ok();

                emit_install(
                    state,
                    install_id,
                    &provider_id,
                    InstallEventLevel::Info,
                    "npm_install",
                    format!("Installing {package}@{BASH_LANGUAGE_SERVER_VERSION}"),
                    None,
                    None,
                    None,
                )
                .await;

                npm_install_one(
                    state,
                    install_id,
                    &provider_id,
                    &node,
                    &package_dir,
                    package,
                    BASH_LANGUAGE_SERVER_VERSION,
                )
                .await
                .context("installing bash-language-server")?;

                let entry =
                    resolve_node_package_bin(&package_dir, package, Some("bash-language-server"))
                        .await
                        .context("resolving bash-language-server")?;
                register(
                    "bash",
                    package,
                    BASH_LANGUAGE_SERVER_VERSION,
                    &package_dir,
                    entry,
                    vec!["start".to_string(), "--stdio".to_string()],
                );
            }
            "dockerfile" => {
                let package = "dockerfile-language-server-nodejs";
                let package_dir = data_root
                    .join("lsp")
                    .join("node")
                    .join(sanitize_npm_package_for_path(package))
                    .join(DOCKERFILE_LANGUAGE_SERVER_VERSION);
                tokio::fs::create_dir_all(&package_dir).await.ok();

                emit_install(
                    state,
                    install_id,
                    &provider_id,
                    InstallEventLevel::Info,
                    "npm_install",
                    format!("Installing {package}@{DOCKERFILE_LANGUAGE_SERVER_VERSION}"),
                    None,
                    None,
                    None,
                )
                .await;

                npm_install_one(
                    state,
                    install_id,
                    &provider_id,
                    &node,
                    &package_dir,
                    package,
                    DOCKERFILE_LANGUAGE_SERVER_VERSION,
                )
                .await
                .context("installing dockerfile-language-server-nodejs")?;

                let entry =
                    resolve_node_package_bin(&package_dir, package, Some("docker-langserver"))
                        .await
                        .context("resolving dockerfile language server bin")?;
                register(
                    "dockerfile",
                    package,
                    DOCKERFILE_LANGUAGE_SERVER_VERSION,
                    &package_dir,
                    entry,
                    vec!["--stdio".to_string()],
                );
            }
            _ => unreachable!(),
        }

        stage = "registry_save";
        save_lsp_server_config(&data_root, &cfg)
            .await
            .context("saving lsp server config")?;

        emit_install(
            state,
            install_id,
            &provider_id,
            InstallEventLevel::Success,
            "done",
            "Install complete (restart daemon to apply)".to_string(),
            None,
            None,
            None,
        )
        .await;
        Ok(())
    }
    .await;

    if let Err(e) = &res {
        let error_code = classify_install_error(stage, e);
        emit_install_with_code(
            state,
            install_id,
            &provider_id,
            InstallEventLevel::Error,
            "error",
            truncate_for_storage(&format!("{e:#}"), INSTALL_EVENT_ERROR_MAX_LEN),
            None,
            None,
            None,
            Some(error_code),
        )
        .await;

        // Best-effort: record error in registry.
        let mut cfg = load_lsp_server_config(&data_root).await.unwrap_or_default();
        cfg.managed_installs.insert(
            server_id.to_string(),
            ManagedInstallMetadata {
                package: None,
                version: None,
                target: None,
                install_dir_rel: None,
                bin_dir_rel: None,
                last_success_at: None,
                last_error: Some(ManagedInstallError {
                    at: Utc::now().to_rfc3339(),
                    stage: stage.to_string(),
                    message: truncate_for_storage(&format!("{e:#}"), LAST_ERROR_MAX_LEN),
                    code: Some(error_code),
                }),
            },
        );
        let _ = save_lsp_server_config(&data_root, &cfg).await;
    }

    res
}

pub async fn install_title_generation_local_with_progress(
    state: std::sync::Arc<AppState>,
    install_id: InstallId,
) -> Result<()> {
    let res = install_title_generation_local_impl(state.as_ref(), Some(install_id)).await;
    match &res {
        Ok(()) => state.finish_install(install_id, true, None, None).await,
        Err(e) => {
            let code = classify_install_error("title_generation_install", e);
            state
                .finish_install(
                    install_id,
                    false,
                    Some(truncate_for_storage(&format!("{e:#}"), 12_000)),
                    Some(code),
                )
                .await
        }
    }
    res
}

async fn install_title_generation_local_impl(
    state: &AppState,
    install_id: Option<InstallId>,
) -> Result<()> {
    let Some(runtime_spec) = title_generation_local::runtime_download_spec() else {
        anyhow::bail!("llama.cpp runtime not available for this platform");
    };

    let runtime_dir = title_generation_local::runtime_dir(&state.core.data_root);
    tokio::fs::create_dir_all(&runtime_dir).await.ok();

    let runtime_bin = title_generation_local::find_runtime_binary(&state.core.data_root);
    if runtime_bin.is_none() {
        emit_install(
            state,
            install_id,
            TITLE_GENERATION_LOCAL_INSTALL_KEY,
            InstallEventLevel::Info,
            "runtime_download",
            "downloading llama.cpp runtime".to_string(),
            None,
            None,
            None,
        )
        .await;

        let tmp = std::env::temp_dir().join(format!(
            "ctx-title-runtime-{}.download",
            install_id.unwrap_or_else(InstallId::new_v4)
        ));
        download_to_file(
            state,
            install_id,
            TITLE_GENERATION_LOCAL_INSTALL_KEY,
            "runtime_download",
            runtime_spec.url,
            &tmp,
        )
        .await?;

        emit_install(
            state,
            install_id,
            TITLE_GENERATION_LOCAL_INSTALL_KEY,
            InstallEventLevel::Info,
            "runtime_verify",
            "verifying runtime checksum".to_string(),
            None,
            None,
            None,
        )
        .await;

        let digest = sha256_file(&tmp).await?;
        if !digest.eq_ignore_ascii_case(runtime_spec.sha256) {
            anyhow::bail!(
                "llama.cpp runtime checksum mismatch: expected {}, got {}",
                runtime_spec.sha256,
                digest
            );
        }

        emit_install(
            state,
            install_id,
            TITLE_GENERATION_LOCAL_INSTALL_KEY,
            InstallEventLevel::Info,
            "runtime_extract",
            "extracting runtime".to_string(),
            None,
            None,
            None,
        )
        .await;

        match runtime_spec.archive_kind {
            title_generation_local::RuntimeArchiveKind::TarGz => {
                let tar_gz =
                    std::fs::File::open(&tmp).with_context(|| format!("open {}", tmp.display()))?;
                let decompressor = flate2::read::GzDecoder::new(tar_gz);
                let mut archive = tar::Archive::new(decompressor);
                archive
                    .unpack(&runtime_dir)
                    .context("extract runtime tar.gz")?;
            }
            title_generation_local::RuntimeArchiveKind::Zip => {
                extract_zip_to_dir(&tmp, &runtime_dir)?;
            }
        }

        let runtime_bin = title_generation_local::find_runtime_binary(&state.core.data_root)
            .ok_or_else(|| anyhow::anyhow!("llama-server binary not found after extraction"))?;
        ensure_executable(&runtime_bin)?;
        tokio::fs::remove_file(&tmp).await.ok();
    }

    let model_dir = title_generation_local::model_dir(&state.core.data_root);
    tokio::fs::create_dir_all(&model_dir).await.ok();
    let model_path = title_generation_local::model_path(&state.core.data_root);

    let expected_sha = fetch_hf_etag_sha256(title_generation_local::LOCAL_MODEL_URL)
        .await
        .unwrap_or(None);
    let mut model_exists = model_path.exists();
    if model_exists {
        let digest = sha256_file(&model_path).await?;
        let needs_metadata = title_generation_local::load_model_metadata(&state.core.data_root)
            .await
            .is_none();
        if let Some(expected) = expected_sha.as_ref() {
            if !digest.eq_ignore_ascii_case(expected) {
                emit_install(
                    state,
                    install_id,
                    TITLE_GENERATION_LOCAL_INSTALL_KEY,
                    InstallEventLevel::Warning,
                    "model_verify",
                    "installed model checksum mismatch; re-downloading".to_string(),
                    None,
                    None,
                    None,
                )
                .await;
                tokio::fs::remove_file(&model_path).await.ok();
                model_exists = false;
            } else if needs_metadata {
                let size = tokio::fs::metadata(&model_path).await?.len();
                let meta = title_generation_local::LocalModelMetadata {
                    id: title_generation_local::LOCAL_MODEL_ID.to_string(),
                    version: title_generation_local::LOCAL_MODEL_VERSION.to_string(),
                    sha256: digest,
                    size,
                    installed_at: Utc::now(),
                };
                title_generation_local::write_model_metadata(&state.core.data_root, &meta).await?;
            }
        } else if needs_metadata {
            let size = tokio::fs::metadata(&model_path).await?.len();
            let meta = title_generation_local::LocalModelMetadata {
                id: title_generation_local::LOCAL_MODEL_ID.to_string(),
                version: title_generation_local::LOCAL_MODEL_VERSION.to_string(),
                sha256: digest,
                size,
                installed_at: Utc::now(),
            };
            title_generation_local::write_model_metadata(&state.core.data_root, &meta).await?;
        }
    }

    if !model_exists {
        emit_install(
            state,
            install_id,
            TITLE_GENERATION_LOCAL_INSTALL_KEY,
            InstallEventLevel::Info,
            "model_download",
            "downloading model".to_string(),
            None,
            None,
            None,
        )
        .await;

        let tmp = std::env::temp_dir().join(format!(
            "ctx-title-model-{}.download",
            install_id.unwrap_or_else(InstallId::new_v4)
        ));
        download_to_file(
            state,
            install_id,
            TITLE_GENERATION_LOCAL_INSTALL_KEY,
            "model_download",
            title_generation_local::LOCAL_MODEL_URL,
            &tmp,
        )
        .await?;

        emit_install(
            state,
            install_id,
            TITLE_GENERATION_LOCAL_INSTALL_KEY,
            InstallEventLevel::Info,
            "model_verify",
            "verifying model checksum".to_string(),
            None,
            None,
            None,
        )
        .await;

        let digest = sha256_file(&tmp).await?;
        if let Some(expected) = expected_sha.as_ref() {
            if !digest.eq_ignore_ascii_case(expected) {
                anyhow::bail!(
                    "model checksum mismatch: expected {}, got {}",
                    expected,
                    digest
                );
            }
        }

        tokio::fs::rename(&tmp, &model_path)
            .await
            .with_context(|| format!("move model to {}", model_path.display()))?;
        let size = tokio::fs::metadata(&model_path).await?.len();
        let meta = title_generation_local::LocalModelMetadata {
            id: title_generation_local::LOCAL_MODEL_ID.to_string(),
            version: title_generation_local::LOCAL_MODEL_VERSION.to_string(),
            sha256: digest,
            size,
            installed_at: Utc::now(),
        };
        title_generation_local::write_model_metadata(&state.core.data_root, &meta).await?;
    }

    Ok(())
}

async fn fetch_hf_etag_sha256(url: &str) -> Result<Option<String>> {
    let client = reqwest::Client::builder()
        .timeout(DOWNLOAD_TIMEOUT)
        .build()
        .context("building http client")?;
    let resp = client
        .head(url)
        .send()
        .await
        .context("requesting model header")?;
    if !resp.status().is_success() {
        return Ok(None);
    }
    let header = resp
        .headers()
        .get("x-linked-etag")
        .and_then(|value| value.to_str().ok());
    Ok(header.and_then(normalize_etag_sha256))
}

fn normalize_etag_sha256(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let trimmed = trimmed.strip_prefix("W/").unwrap_or(trimmed);
    let trimmed = trimmed.trim_matches('"');
    if trimmed.len() != 64 {
        return None;
    }
    if !trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(trimmed.to_string())
}

fn validate_sha256_digest(expected_sha256: &str, digest: &str) -> Result<()> {
    let expected_sha256 = expected_sha256.trim();
    if digest.eq_ignore_ascii_case(expected_sha256) {
        return Ok(());
    }
    anyhow::bail!(
        "archive checksum mismatch: expected {}, got {}",
        expected_sha256,
        digest
    );
}

async fn sha256_file(path: &Path) -> Result<String> {
    use tokio::io::AsyncReadExt;

    let mut file = tokio::fs::File::open(path)
        .await
        .with_context(|| format!("open {}", path.display()))?;
    let mut hasher = sha2::Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

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
