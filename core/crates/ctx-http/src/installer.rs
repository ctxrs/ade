use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::time::timeout;

use crate::daemon::AppState;
use crate::installs::{truncate_for_storage, InstallEventLevel, InstallId, InstallProgressEvent};
use crate::lsp_catalog::{LspCatalogArchive, LspCatalogInstall};
use crate::provider_matrix;
use crate::updates;
use ctx_lsp::LspManagerConfig;
use ctx_providers::tier1::Tier1AcpAdapter;

const NODE_VERSION: &str = "24.12.0";
const PYTHON_VERSION: &str = "3.13.11";
const PYTHON_BUILD_TAG: &str = "20251217";

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

fn node_runtime_install_lock() -> &'static Mutex<()> {
    NODE_RUNTIME_INSTALL_LOCK.get_or_init(|| Mutex::new(()))
}

fn python_runtime_install_lock() -> &'static Mutex<()> {
    PYTHON_RUNTIME_INSTALL_LOCK.get_or_init(|| Mutex::new(()))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedInstallError {
    pub at: String,
    pub stage: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedInstallMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
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

pub async fn install_provider(state: &AppState, provider_id: &str) -> Result<()> {
    install_provider_impl(state, provider_id, None).await
}

pub fn is_supported_managed_provider(
    matrix: &provider_matrix::ProviderMatrix,
    provider_id: &str,
) -> bool {
    provider_matrix::is_managed_supported(matrix, provider_id)
}

pub fn is_supported_managed_lsp_server(server_id: &str) -> bool {
    matches!(
        server_id,
        "typescript" | "python" | "html" | "css" | "json" | "yaml" | "bash" | "dockerfile"
    )
}

pub fn apply_managed_install_details(
    status: &mut ctx_providers::adapters::ProviderStatus,
    cfg: &AgentServerConfigFile,
) {
    let meta = cfg
        .providers
        .get(&status.provider_id)
        .and_then(|e| e.managed.as_ref())
        .or_else(|| cfg.managed_installs.get(&status.provider_id));
    let Some(meta) = meta else {
        return;
    };

    if let Some(v) = &meta.version {
        status
            .details
            .insert("managed_version".to_string(), v.clone());
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

pub async fn install_provider_with_progress(
    state: std::sync::Arc<AppState>,
    install_id: InstallId,
    provider_id: String,
) -> Result<()> {
    let res = install_provider_impl(state.as_ref(), &provider_id, Some(install_id)).await;
    match &res {
        Ok(()) => state.finish_install(install_id, true, None).await,
        Err(e) => {
            state
                .finish_install(
                    install_id,
                    false,
                    Some(truncate_for_storage(&format!("{e:#}"), 12_000)),
                )
                .await
        }
    }
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

fn venv_bin_dir(venv_dir: &Path) -> PathBuf {
    if cfg!(windows) {
        venv_dir.join("Scripts")
    } else {
        venv_dir.join("bin")
    }
}

fn venv_exe(venv_dir: &Path, name: &str) -> PathBuf {
    let bin = venv_bin_dir(venv_dir);
    if cfg!(windows) {
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
}

fn zed_target_key() -> Result<&'static str> {
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

fn extract_tar_bz2_to_dir(tar_bz2_path: &Path, out_dir: &Path) -> Result<()> {
    let tar_bz2 = std::fs::File::open(tar_bz2_path)
        .with_context(|| format!("open {}", tar_bz2_path.display()))?;
    let dec = bzip2::read::BzDecoder::new(tar_bz2);
    let mut archive = tar::Archive::new(dec);
    archive.unpack(out_dir).context("extract tar.bz2")?;
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
    archive: AgentServerArchive,
    bin_path: &str,
    stage: &mut &'static str,
) -> Result<PathBuf> {
    let data_root = &state.data_root;
    let install_dir = data_root
        .join("providers")
        .join("agent-servers")
        .join(provider_id)
        .join(version);
    tokio::fs::create_dir_all(&install_dir).await.ok();

    let tmp_dir = data_root.join("providers").join("tmp");
    tokio::fs::create_dir_all(&tmp_dir).await.ok();
    let tmp = tmp_dir.join(format!("{provider_id}-{version}.download"));

    *stage = "download";
    download_to_file(state, install_id, event_provider_id, "download", url, &tmp).await?;

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

    match archive {
        AgentServerArchive::None => {
            let dest = install_dir.join(bin_path);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            std::fs::rename(&tmp, &dest).ok();
            ensure_executable(&dest)?;
            Ok(dest)
        }
        AgentServerArchive::TarGz => {
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
        AgentServerArchive::TarBz2 => {
            extract_tar_bz2_to_dir(&tmp, &install_dir)?;
            let direct = install_dir.join(bin_path);
            let resolved = if direct.exists() {
                direct
            } else {
                find_unique_path_ending_with(&install_dir, bin_path)?
            };
            ensure_executable(&resolved)?;
            Ok(resolved)
        }
        AgentServerArchive::Zip => {
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
    let data_root = &state.data_root;
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
        Ok(()) => state.finish_install(install_id, true, None).await,
        Err(e) => {
            state
                .finish_install(
                    install_id,
                    false,
                    Some(truncate_for_storage(&format!("{e:#}"), 12_000)),
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
    let data_root = state.data_root.clone();
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
        Ok(()) => state.finish_install(install_id, true, None).await,
        Err(e) => {
            state
                .finish_install(
                    install_id,
                    false,
                    Some(truncate_for_storage(&format!("{e:#}"), 12_000)),
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
    stage: &mut &'static str,
) -> Result<ManagedProviderInstall> {
    let data_root = state.data_root.clone();
    let install_dir = data_root
        .join("providers")
        .join("agent-servers")
        .join(provider_id)
        .join(version);
    let install_dir_rel = install_dir_rel(&data_root, &install_dir);

    *stage = "node";
    let node = ensure_node_runtime(state, install_id, provider_id, &data_root)
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
    )
    .await
    .context("running npm install")?;

    if provider_id == "claude" {
        *stage = "patch";
        if let Err(e) =
            patch_claude_code_acp_for_ask_user_question(&install_dir.join(script_rel)).await
        {
            // Don't hard-fail install if patching fails; Claude can still run without the UI.
            // This is a best-effort enhancement until the upstream package includes AskUserQuestion routing.
            tracing::warn!("failed to patch claude-code-acp for AskUserQuestion: {e:#}");
        }
    }

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

pub async fn ensure_claude_code_acp_ask_user_question_patched(
    agent_cfg: &AgentServerConfigFile,
) -> Result<bool> {
    let Some(cmd) = agent_cfg.providers.get("claude") else {
        return Ok(false);
    };
    let Some(script_path) = cmd.args.first() else {
        return Ok(false);
    };
    let script_path = PathBuf::from(script_path);
    patch_claude_code_acp_for_ask_user_question(&script_path).await
}

async fn patch_claude_code_acp_for_ask_user_question(script_path: &Path) -> Result<bool> {
    // `script_path` is usually `.../node_modules/@zed-industries/claude-code-acp/dist/index.js`.
    // We patch `dist/acp-agent.js` in the same directory to route `AskUserQuestion` tool calls via
    // the ACP extension method `_claude_code_acp/ask_user_question`.
    let Some(dist_dir) = script_path.parent() else {
        return Ok(false);
    };
    if script_path.file_name().and_then(|s| s.to_str()) != Some("index.js") {
        return Ok(false);
    }
    let acp_agent_js = dist_dir.join("acp-agent.js");
    if !acp_agent_js.exists() {
        return Ok(false);
    }

    let original = tokio::fs::read_to_string(&acp_agent_js)
        .await
        .with_context(|| format!("reading {}", acp_agent_js.display()))?;
    let mut patched = original.clone();
    let mut changed = false;

    // `@agentclientprotocol/sdk` implements `extMethod(method, ...)` by sending the JSON-RPC
    // request method `_${method}`. That means callers should pass `claude_code_acp/...` (no
    // leading underscore) to produce `_claude_code_acp/...` on the wire.
    //
    // Older/broken patches (including our initial one) used `_claude_code_acp/...` as the method
    // argument, which results in `__claude_code_acp/...` on the wire.
    if patched.contains("extMethod(\"_claude_code_acp/ask_user_question\"") {
        patched = patched.replace(
            "extMethod(\"_claude_code_acp/ask_user_question\"",
            "extMethod(\"claude_code_acp/ask_user_question\"",
        );
        changed = true;
    }

    let needle = "if (toolName === \"ExitPlanMode\") {";
    let ask_block = r#"if (toolName === "AskUserQuestion") {
                if (signal.aborted) {
                    throw new Error("Tool use aborted");
                }
                try {
                    const rawResponse = await this.client.extMethod("claude_code_acp/ask_user_question", {
                        sessionId,
                        toolCallId: toolUseID,
                        input: toolInput,
                    });
                    if (signal.aborted) {
                        throw new Error("Tool use aborted");
                    }
                    const outcome = rawResponse?.outcome === "submitted" || rawResponse?.outcome === "cancelled" ? rawResponse.outcome : undefined;
                    let answers;
                    if (rawResponse && typeof rawResponse.answers === "object" && rawResponse.answers !== null && !Array.isArray(rawResponse.answers)) {
                        answers = {};
                        for (const [key, value] of Object.entries(rawResponse.answers)) {
                            if (typeof value === "string") {
                                answers[key] = value;
                            }
                        }
                    }
                    if (outcome === "cancelled") {
                        return {
                            behavior: "deny",
                            message: "User cancelled the question prompt. Proceed without using AskUserQuestion and ask for clarification in the chat instead.",
                        };
                    }
                    return {
                        behavior: "allow",
                        updatedInput: {
                            ...toolInput,
                            answers: answers ?? {},
                        },
                    };
                }
                catch (error) {
                    const errorMessage = error instanceof Error && error.message ? error.message : String(error);
                    return {
                        behavior: "deny",
                        message: "This ACP client does not support the AskUserQuestion interactive UI. Ask the user your questions in plain text and continue after the next user message. Details: " +
                            errorMessage,
                    };
                }
            }
            "#;

    if !patched.contains("extMethod(\"claude_code_acp/ask_user_question\"") {
        let Some(insert_at) = patched.find(needle) else {
            return Ok(changed);
        };
        let mut next = String::with_capacity(patched.len() + ask_block.len() + 16);
        next.push_str(&patched[..insert_at]);
        next.push_str(ask_block);
        next.push_str(&patched[insert_at..]);
        patched = next;
        changed = true;
    }

    if !patched.contains("CLAUDE_CODE_ENABLE_ASK_USER_QUESTION_TOOL") {
        let allow_needle = "const disableBuiltInTools = params._meta?.disableBuiltInTools === true;";
        let allow_block = r#"
        if (!disableBuiltInTools && process.env.CLAUDE_CODE_ENABLE_ASK_USER_QUESTION_TOOL === "1") {
            allowedTools.push("AskUserQuestion");
        }"#;
        if let Some(insert_at) = patched.find(allow_needle) {
            let insert_at = insert_at + allow_needle.len();
            let mut next = String::with_capacity(patched.len() + allow_block.len() + 8);
            next.push_str(&patched[..insert_at]);
            next.push_str(allow_block);
            next.push_str(&patched[insert_at..]);
            patched = next;
            changed = true;
        }
    }

    if !changed {
        return Ok(false);
    }

    tokio::fs::write(&acp_agent_js, patched)
        .await
        .with_context(|| format!("writing {}", acp_agent_js.display()))?;

    Ok(true)
}

#[allow(clippy::too_many_arguments)]
async fn install_managed_archive_provider(
    state: &AppState,
    install_id: Option<InstallId>,
    provider_id: &str,
    version: &str,
    url: &str,
    archive: AgentServerArchive,
    bin_path: &str,
    args: Vec<String>,
    stage: &mut &'static str,
) -> Result<ManagedProviderInstall> {
    let bin = install_agent_server_url_binary(
        state,
        install_id,
        provider_id,
        provider_id,
        version,
        url,
        archive,
        bin_path,
        stage,
    )
    .await
    .context("installing agent server binary")?;

    let install_dir = state
        .data_root
        .join("providers")
        .join("agent-servers")
        .join(provider_id)
        .join(version);
    let meta = ManagedInstallMetadata {
        package: Some(url.to_string()),
        version: Some(version.to_string()),
        install_dir_rel: Some(install_dir_rel(&state.data_root, &install_dir)),
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
    stage: &mut &'static str,
) -> Result<ManagedProviderInstall> {
    *stage = "python";
    let python = ensure_python_runtime(state, install_id, provider_id, &state.data_root)
        .await
        .context("ensuring managed Python runtime")?
        .python_bin;
    let data_root = state.data_root.clone();
    let install_dir = data_root
        .join("providers")
        .join("agent-servers")
        .join(provider_id)
        .join(version);
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
        let expected = venv_exe(&venv_dir, entrypoint);
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

    let mut venv_cmd = Command::new(&python);
    venv_cmd
        .arg("-m")
        .arg("venv")
        .arg(&venv_dir)
        .kill_on_drop(true);
    run_command_with_timeout(venv_cmd, Duration::from_secs(5 * 60))
        .await
        .context("creating virtualenv")?;

    let venv_python = venv_exe(&venv_dir, "python");

    ensure_python_pip(&venv_python)
        .await
        .context("ensuring pip in virtualenv")?;

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
    let out = run_command_with_timeout(pip_cmd, PIP_INSTALL_TIMEOUT)
        .await
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

    let exe = venv_exe(&venv_dir, entrypoint);
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
    let data_root = state.data_root.clone();
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
    let node = ensure_node_runtime(state, install_id, provider_id, &data_root)
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
    )
    .await
    .context("running npm install for dependency")?;

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
    archive: AgentServerArchive,
    bin_path: &str,
    stage: &mut &'static str,
) -> Result<ManagedDependencyInstall> {
    let data_root = state.data_root.clone();
    let install_dir = data_root
        .join("providers")
        .join("agent-servers")
        .join(dependency_id)
        .join(version);

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
            archive,
            bin_path,
            stage,
        )
        .await
        .context("installing dependency binary")?
    };

    let bin_dir = bin.parent().unwrap_or(&install_dir).to_path_buf();
    let meta = ManagedInstallMetadata {
        package: Some(url.to_string()),
        version: Some(version.to_string()),
        install_dir_rel: Some(install_dir_rel(&data_root, &install_dir)),
        bin_dir_rel: Some(install_dir_rel(&data_root, &bin_dir)),
        last_success_at: Some(Utc::now().to_rfc3339()),
        last_error: None,
    };

    Ok(ManagedDependencyInstall { meta })
}

fn resolve_install_args(args: &[String], data_root: &Path) -> Vec<String> {
    args.iter()
        .map(|arg| {
            if arg == "{{cagent_config}}" {
                cagent_config_path(data_root).to_string_lossy().to_string()
            } else {
                arg.clone()
            }
        })
        .collect()
}

fn map_archive_kind(kind: provider_matrix::ProviderArchiveKind) -> AgentServerArchive {
    match kind {
        provider_matrix::ProviderArchiveKind::None => AgentServerArchive::None,
        provider_matrix::ProviderArchiveKind::TarGz => AgentServerArchive::TarGz,
        provider_matrix::ProviderArchiveKind::TarBz2 => AgentServerArchive::TarBz2,
        provider_matrix::ProviderArchiveKind::Zip => AgentServerArchive::Zip,
    }
}

async fn ensure_cagent_config(
    state: &AppState,
    install_id: Option<InstallId>,
    provider_id: &str,
    stage: &mut &'static str,
) -> Result<PathBuf> {
    let cfg_path = cagent_config_path(&state.data_root);
    if cfg_path.exists() {
        return Ok(cfg_path);
    }
    *stage = "prepare";
    emit_install(
        state,
        install_id,
        provider_id,
        InstallEventLevel::Info,
        "prepare",
        "Writing default cagent config".to_string(),
        None,
        None,
        None,
    )
    .await;
    if let Some(parent) = cfg_path.parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }
    let cfg = r#"agents:
  root:
    model: openai/gpt-5-mini
    description: ctx default agent
    instruction: |
      You are a helpful coding assistant.
"#;
    tokio::fs::write(&cfg_path, cfg).await.ok();
    Ok(cfg_path)
}

async fn install_provider_impl(
    state: &AppState,
    provider_id: &str,
    install_id: Option<InstallId>,
) -> Result<()> {
    let provider_id = provider_id.to_string();
    let mut stage: &'static str = "start";
    let mut error_package: Option<String> = None;
    let mut error_version: Option<String> = None;
    let mut error_install_dir_rel: Option<String> = None;

    let res: Result<()> = async {
        emit_install(
            state,
            install_id,
            &provider_id,
            InstallEventLevel::Info,
            "start",
            format!("Installing managed provider: {provider_id}"),
            None,
            None,
            None,
        )
        .await;

        let matrix =
            provider_matrix::load_matrix_cached(&state.data_root, &state.provider_matrix_cache)
                .await;
        let entry = provider_matrix::get_entry(&matrix, &provider_id)
            .ok_or_else(|| anyhow::anyhow!("unsupported provider for install: {provider_id}"))?;
        let Some(install) = entry.managed_install.as_ref() else {
            anyhow::bail!("provider has no managed install: {provider_id}");
        };
        let context_version = updates::normalize_version_str(env!("CARGO_PKG_VERSION"));
        let release = provider_matrix::recommended_release(entry, context_version.as_ref())
            .ok_or_else(|| anyhow::anyhow!("no compatible release for provider: {provider_id}"))?;

        if provider_id == "cagent" {
            let _ = ensure_cagent_config(state, install_id, &provider_id, &mut stage).await?;
        }

        let mut dependency_ids: Vec<String> = Vec::new();
        if !entry.dependencies.is_empty() {
            stage = "dependencies";
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
                dependency_ids.push(dep.id.clone());
                let managed = match &dep.install {
                    provider_matrix::DependencyInstall::Npm { package, version } => {
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
                        let target = zed_target_key().context("resolving platform target")?;
                        let target_entry = targets.get(target).ok_or_else(|| {
                            anyhow::anyhow!("unsupported dependency target {}: {target}", dep.id)
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
                            map_archive_kind(target_entry.archive),
                            &target_entry.bin_path,
                            &mut stage,
                        )
                        .await?
                    }
                };

                let mut cfg = load_agent_server_config(&state.data_root)
                    .await
                    .unwrap_or_default();
                cfg.managed_installs
                    .insert(dep.id.clone(), managed.meta.clone());
                save_agent_server_config(&state.data_root, &cfg)
                    .await
                    .context("saving managed install registry")?;
            }
        }

        let managed = match install {
            provider_matrix::ProviderInstall::Npm {
                package,
                entrypoint,
                args,
            } => {
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
                    resolve_install_args(args, &state.data_root),
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
                    resolve_install_args(args, &state.data_root),
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
                let target = zed_target_key().context("resolving platform target")?;
                let target_entry = targets.get(target).ok_or_else(|| {
                    anyhow::anyhow!("unsupported provider target {provider_id}: {target}")
                })?;
                error_package = Some(target_entry.url.clone());
                error_version = Some(version.clone());
                error_install_dir_rel = Some(format!(
                    "providers/agent-servers/{}/{}",
                    provider_id, version
                ));
                install_managed_archive_provider(
                    state,
                    install_id,
                    &provider_id,
                    version,
                    &target_entry.url,
                    map_archive_kind(target_entry.archive),
                    &target_entry.bin_path,
                    resolve_install_args(args, &state.data_root),
                    &mut stage,
                )
                .await?
            }
        };

        stage = "inspect";
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

        let adapter: std::sync::Arc<Tier1AcpAdapter> =
            std::sync::Arc::new(if provider_id == "claude" {
                Tier1AcpAdapter::claude_from_raw_with_ask_user_question(
                    managed.command.clone(),
                    managed.args.clone(),
                    std::sync::Arc::clone(&state.ask_user_question),
                )
            } else {
                Tier1AcpAdapter::from_raw(
                    &provider_id,
                    managed.command.clone(),
                    managed.args.clone(),
                )
            });

        // Refresh the in-memory adapter so new Sessions use the managed install.
        {
            let mut map = state.providers.lock().await;
            map.insert(provider_id.clone(), adapter.clone());
        }

        stage = "refresh";
        let mut status_cfg = load_agent_server_config(&state.data_root)
            .await
            .unwrap_or_default();
        status_cfg
            .managed_installs
            .insert(provider_id.clone(), managed.meta.clone());
        refresh_provider_statuses_with_cfg(state, status_cfg).await?;

        let status = state
            .provider_statuses
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

        let mut cfg = load_agent_server_config(&state.data_root)
            .await
            .context("loading managed install registry")?;
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
        save_agent_server_config(&state.data_root, &cfg)
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
        emit_install(
            state,
            install_id,
            &provider_id,
            InstallEventLevel::Error,
            "error",
            truncate_for_storage(&format!("{e:#}"), INSTALL_EVENT_ERROR_MAX_LEN),
            None,
            None,
            None,
        )
        .await;
        update_registry_last_error(
            &state.data_root,
            &provider_id,
            stage,
            e,
            error_package.as_deref(),
            error_version.as_deref(),
            error_install_dir_rel.clone(),
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
    let Some(install_id) = install_id else {
        return;
    };
    state
        .emit_install_event(
            install_id,
            InstallProgressEvent {
                install_id,
                provider_id: provider_id.to_string(),
                at: Utc::now(),
                stage: stage.to_string(),
                message,
                level,
                bytes,
                total_bytes,
                attempt,
            },
        )
        .await;
}

async fn update_registry_last_error(
    data_root: &Path,
    provider_id: &str,
    stage: &str,
    err: &anyhow::Error,
    package: Option<&str>,
    version: Option<&str>,
    install_dir_rel: Option<String>,
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

    meta.last_error = Some(ManagedInstallError {
        at: Utc::now().to_rfc3339(),
        stage: stage.to_string(),
        message: truncate_for_storage(&format!("{err:#}"), LAST_ERROR_MAX_LEN),
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
    let cfg = load_agent_server_config(&state.data_root)
        .await
        .unwrap_or_default();
    refresh_provider_statuses_with_cfg(state, cfg).await
}

async fn refresh_provider_statuses_with_cfg(
    state: &AppState,
    cfg: AgentServerConfigFile,
) -> Result<()> {
    let matrix =
        provider_matrix::load_matrix_cached(&state.data_root, &state.provider_matrix_cache).await;

    let map = state.providers.lock().await;
    let mut statuses = HashMap::new();
    for (id, adapter) in map.iter() {
        match adapter.inspect().await {
            Ok(mut status) => {
                apply_managed_install_details(&mut status, &cfg);
                if let Some(entry) = provider_matrix::get_entry(&matrix, id) {
                    provider_matrix::apply_matrix_to_status(
                        &state.data_root,
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
    *state.provider_statuses.lock().await = statuses;
    Ok(())
}

pub fn agent_server_config_path(data_root: &Path) -> PathBuf {
    data_root
        .join("providers")
        .join("agent-servers")
        .join("agent_servers.json")
}

pub fn cagent_config_path(data_root: &Path) -> PathBuf {
    data_root
        .join("providers")
        .join("agent-servers")
        .join("cagent")
        .join("config.yaml")
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
    serde_json::from_str(&txt).context("parsing agent server config")
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

#[derive(Debug, Clone)]
pub struct NodeRuntime {
    pub node_root: PathBuf,
    pub node_bin: PathBuf,
    pub npm_cli_js: PathBuf,
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

pub(crate) async fn ensure_node_runtime(
    state: &AppState,
    install_id: Option<InstallId>,
    provider_id: &str,
    data_root: &Path,
) -> Result<NodeRuntime> {
    let target = node_target_triple()?;
    let folder = format!("node-v{NODE_VERSION}-{target}");
    let node_root = data_root.join("runtimes").join("node").join(&folder);
    let (node_bin, npm_cli_js) = node_runtime_paths(&node_root);

    if node_bin.exists() && npm_cli_js.exists() {
        emit_install(
            state,
            install_id,
            provider_id,
            InstallEventLevel::Info,
            "node",
            format!("Using existing Node runtime v{NODE_VERSION} ({target})"),
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
            format!("Using existing Node runtime v{NODE_VERSION} ({target})"),
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

    let (archive_ext, archive_label) = if cfg!(windows) {
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

    // Node tarballs contain a single top-level folder named `node-vX.Y.Z-<target>`.
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

fn node_runtime_paths(node_root: &Path) -> (PathBuf, PathBuf) {
    if cfg!(windows) {
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

fn node_target_triple() -> Result<&'static str> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    match (os, arch) {
        ("macos", "aarch64") => Ok("darwin-arm64"),
        ("macos", "x86_64") => Ok("darwin-x64"),
        ("linux", "aarch64") => Ok("linux-arm64"),
        ("linux", "x86_64") => Ok("linux-x64"),
        ("windows", "x86_64") => Ok("win-x64"),
        ("windows", "aarch64") => Ok("win-arm64"),
        _ => anyhow::bail!(
            "unsupported platform for managed node install: {os}/{arch}. Supported: macos (aarch64/x86_64), linux (aarch64/x86_64), windows (aarch64/x86_64)."
        ),
    }
}

async fn ensure_python_runtime(
    state: &AppState,
    install_id: Option<InstallId>,
    provider_id: &str,
    data_root: &Path,
) -> Result<PythonRuntime> {
    let target = python_target_triple()?;
    let folder = format!("cpython-{PYTHON_VERSION}+{PYTHON_BUILD_TAG}-{target}");
    let python_root = data_root.join("runtimes").join("python").join(&folder);
    let python_bin = resolve_python_bin(&python_root);

    if python_bin.exists() {
        emit_install(
            state,
            install_id,
            provider_id,
            InstallEventLevel::Info,
            "python",
            format!("Using existing Python runtime {PYTHON_VERSION} ({target})"),
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
    let python_bin = resolve_python_bin(&python_root);
    if python_bin.exists() {
        emit_install(
            state,
            install_id,
            provider_id,
            InstallEventLevel::Info,
            "python",
            format!("Using existing Python runtime {PYTHON_VERSION} ({target})"),
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

    let asset = format!("cpython-{PYTHON_VERSION}+{PYTHON_BUILD_TAG}-{target}-install_only.tar.gz");
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

    let python_bin = resolve_python_bin(&python_root);
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
        format!("Installed Python runtime {PYTHON_VERSION} ({target})"),
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

fn python_target_triple() -> Result<&'static str> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
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

fn resolve_python_bin(python_root: &Path) -> PathBuf {
    if cfg!(windows) {
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

    for attempt in 1..=RETRY_COUNT {
        emit_install(
            state,
            install_id,
            provider_id,
            InstallEventLevel::Info,
            "npm_install",
            format!("npm install {package_spec} (attempt {attempt}/{RETRY_COUNT})"),
            None,
            None,
            Some(attempt),
        )
        .await;

        let mut cmd = Command::new(&node.node_bin);
        cmd.arg(&node.npm_cli_js)
            .arg("install")
            .arg("--prefix")
            .arg(install_dir)
            .arg("--no-audit")
            .arg("--no-fund")
            .arg("--silent")
            .arg(package_spec)
            .env("PATH", combined_path.clone())
            .env("npm_config_update_notifier", "false")
            .env("npm_config_fund", "false")
            .env("npm_config_audit", "false")
            .env("npm_config_progress", "false")
            .env("npm_config_cache", cache_dir.clone())
            .kill_on_drop(true);

        let out = match run_command_with_timeout(cmd, NPM_INSTALL_TIMEOUT).await {
            Ok(out) => out,
            Err(e) => {
                emit_install(
                    state,
                    install_id,
                    provider_id,
                    InstallEventLevel::Error,
                    "npm_install",
                    format!("npm install failed: {e:#}"),
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
                return Err(e.context("running npm install"));
            }
        };
        if out.status.success() {
            emit_install(
                state,
                install_id,
                provider_id,
                InstallEventLevel::Success,
                "npm_install",
                "npm install succeeded".to_string(),
                None,
                None,
                Some(attempt),
            )
            .await;
            return Ok(());
        }

        let err_txt = format!(
            "npm install failed ({package_spec}) status={}\nstdout:\n{}\nstderr:\n{}",
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
        "npm install failed after {RETRY_COUNT} attempts ({package_spec}). Try again, or check your network / npm registry access."
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

                let resp = client.get(url).send().await.context("sending request")?;
                let resp = resp.error_for_status().context("http error")?;

                let total = resp.content_length();
                let mut stream = resp.bytes_stream();
                let mut file = tokio::fs::File::create(path)
                    .await
                    .with_context(|| format!("creating download target: {}", path.display()))?;
                use futures::StreamExt;
                use tokio::io::AsyncWriteExt;

                let mut downloaded: u64 = 0;
                while let Some(chunk) = stream.next().await {
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
    let data_root = state.data_root.clone();
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
        let node = ensure_node_runtime(state, install_id, &provider_id, &data_root)
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
        emit_install(
            state,
            install_id,
            &provider_id,
            InstallEventLevel::Error,
            "error",
            truncate_for_storage(&format!("{e:#}"), INSTALL_EVENT_ERROR_MAX_LEN),
            None,
            None,
            None,
        )
        .await;

        // Best-effort: record error in registry.
        let mut cfg = load_lsp_server_config(&data_root).await.unwrap_or_default();
        cfg.managed_installs.insert(
            server_id.to_string(),
            ManagedInstallMetadata {
                package: None,
                version: None,
                install_dir_rel: None,
                bin_dir_rel: None,
                last_success_at: None,
                last_error: Some(ManagedInstallError {
                    at: Utc::now().to_rfc3339(),
                    stage: stage.to_string(),
                    message: truncate_for_storage(&format!("{e:#}"), LAST_ERROR_MAX_LEN),
                }),
            },
        );
        let _ = save_lsp_server_config(&data_root, &cfg).await;
    }

    res
}
