use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tokio::time::timeout;

use crate::daemon::AppState;
use crate::installs::{truncate_for_storage, InstallEventLevel, InstallId, InstallProgressEvent};
use context_providers::tier1::Tier1AcpAdapter;

const NODE_VERSION: &str = "22.11.0";
const CODEX_ACP_VERSION: &str = "0.7.1";
const CLAUDE_CODE_ACP_VERSION: &str = "0.12.4";
const GEMINI_CLI_VERSION: &str = "0.19.0";

const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const NPM_INSTALL_TIMEOUT: Duration = Duration::from_secs(12 * 60);
const RETRY_COUNT: u32 = 2;
const RETRY_BACKOFF_BASE_MS: u64 = 750;
const LAST_ERROR_MAX_LEN: usize = 8000;
const INSTALL_EVENT_ERROR_MAX_LEN: usize = 6000;

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
    pub last_success_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<ManagedInstallError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentServerCommand {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
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

pub async fn install_provider(state: &AppState, provider_id: &str) -> Result<()> {
    install_provider_impl(state, provider_id, None).await
}

pub fn is_supported_managed_provider(provider_id: &str) -> bool {
    matches!(provider_id, "codex" | "claude" | "gemini")
}

pub fn apply_managed_install_details(
    status: &mut context_providers::adapters::ProviderStatus,
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
        status.details.insert("managed_version".to_string(), v.clone());
    }
    if let Some(p) = &meta.package {
        status.details.insert("managed_package".to_string(), p.clone());
    }
    if let Some(d) = &meta.install_dir_rel {
        status
            .details
            .insert("managed_install_dir".to_string(), d.clone());
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
        status.details.insert(
            "managed_last_error_stage".to_string(),
            err.stage.clone(),
        );
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

async fn install_provider_impl(
    state: &AppState,
    provider_id: &str,
    install_id: Option<InstallId>,
) -> Result<()> {
    let provider_id = provider_id.to_string();
    let (package, version, script_rel, extra_args) = match provider_id.as_str() {
        "codex" => (
            "@zed-industries/codex-acp",
            CODEX_ACP_VERSION,
            "node_modules/@zed-industries/codex-acp/bin/codex-acp.js",
            Vec::<String>::new(),
        ),
        "claude" => (
            "@zed-industries/claude-code-acp",
            CLAUDE_CODE_ACP_VERSION,
            "node_modules/@zed-industries/claude-code-acp/dist/index.js",
            Vec::<String>::new(),
        ),
        "gemini" => (
            "@google/gemini-cli",
            GEMINI_CLI_VERSION,
            "node_modules/.bin/gemini",
            vec!["--experimental-acp".to_string()],
        ),
        other => anyhow::bail!("unsupported provider for install: {other}"),
    };

    let data_root = state.data_root.clone();
    let install_dir = data_root
        .join("providers")
        .join("agent-servers")
        .join(&provider_id)
        .join(version);
    let install_dir_rel = install_dir_rel(&data_root, &install_dir);
    let mut stage: &'static str = "start";

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

        stage = "node";
        let node = ensure_node_runtime(state, install_id, &provider_id, &data_root)
            .await
            .context("ensuring managed Node runtime")?;

        stage = "prepare";
        repair_install_dir(install_id, state, &provider_id, &install_dir, script_rel)
            .await
            .context("preparing install directory")?;

        let package_spec = format!("{package}@{version}");
        stage = "npm_install";
        npm_install(
            state,
            install_id,
            &provider_id,
            &node,
            &install_dir,
            &package_spec,
        )
        .await
        .context("running npm install")?;

        stage = "entrypoint";
        let script_path = install_dir.join(script_rel);
        if !script_path.exists() {
            tokio::fs::remove_dir_all(&install_dir).await.ok();
            anyhow::bail!("install completed but entrypoint missing: {}", script_path.display());
        }

        let mut args = vec![script_path.to_string_lossy().to_string()];
        args.extend(extra_args.clone());

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

        let adapter = std::sync::Arc::new(Tier1AcpAdapter::from_command(
            &provider_id,
            &node.node_bin,
            &script_path,
            extra_args,
        ));

        // Refresh the in-memory adapter so new Sessions use the managed install.
        {
            let mut map = state.providers.lock().await;
            map.insert(provider_id.clone(), adapter.clone());
        }

        stage = "refresh";
        refresh_provider_statuses(state).await?;

        let status = state
            .provider_statuses
            .lock()
            .await
            .get(&provider_id)
            .cloned();
        if let Some(status) = status {
            if !status.installed || !matches!(status.health, context_providers::adapters::ProviderHealth::Ok) {
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

        let mut cfg = load_agent_server_config(&data_root)
            .await
            .context("loading managed install registry")?;
        let meta = ManagedInstallMetadata {
            package: Some(package.to_string()),
            version: Some(version.to_string()),
            install_dir_rel: Some(install_dir_rel.clone()),
            last_success_at: Some(Utc::now().to_rfc3339()),
            last_error: None,
        };
        cfg.managed_installs.insert(provider_id.clone(), meta.clone());
        cfg.providers.insert(
            provider_id.clone(),
            AgentServerCommand {
                command: node.node_bin.to_string_lossy().to_string(),
                args,
                managed: Some(meta),
            },
        );
        save_agent_server_config(&data_root, &cfg)
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
            &data_root,
            &provider_id,
            stage,
            e,
            Some(package),
            Some(version),
            Some(install_dir_rel),
        )
        .await;
    }

    res
}

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
    let mut cfg = load_agent_server_config(data_root).await.unwrap_or_default();
    let install_dir_rel_clone = install_dir_rel.clone();
    let mut meta = cfg
        .managed_installs
        .get(provider_id)
        .cloned()
        .unwrap_or(ManagedInstallMetadata {
            package: package.map(|s| s.to_string()),
            version: version.map(|s| s.to_string()),
            install_dir_rel: install_dir_rel_clone,
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

    let map = state.providers.lock().await;
    let mut statuses = HashMap::new();
    for (id, adapter) in map.iter() {
        match adapter.inspect().await {
            Ok(mut status) => {
                apply_managed_install_details(&mut status, &cfg);
                statuses.insert(id.clone(), status);
            }
            Err(e) => {
                statuses.insert(
                    id.clone(),
                    context_providers::adapters::ProviderStatus {
                        provider_id: id.clone(),
                        installed: false,
                        detected_path: None,
                        version: None,
                        capabilities: None,
                        health: context_providers::adapters::ProviderHealth::Error,
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

pub async fn load_agent_server_config(data_root: &Path) -> Result<AgentServerConfigFile> {
    let path = agent_server_config_path(data_root);
    if !path.exists() {
        return Ok(AgentServerConfigFile::default());
    }
    let txt = tokio::fs::read_to_string(&path).await?;
    Ok(serde_json::from_str(&txt).context("parsing agent server config")?)
}

pub async fn save_agent_server_config(
    data_root: &Path,
    cfg: &AgentServerConfigFile,
) -> Result<()> {
    let path = agent_server_config_path(data_root);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(&path, serde_json::to_string_pretty(cfg)?).await?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct NodeRuntime {
    pub node_root: PathBuf,
    pub node_bin: PathBuf,
    pub npm_cli_js: PathBuf,
}

fn install_dir_rel(data_root: &Path, install_dir: &Path) -> String {
    install_dir
        .strip_prefix(data_root)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| install_dir.to_string_lossy().to_string())
}

async fn ensure_node_runtime(
    state: &AppState,
    install_id: Option<InstallId>,
    provider_id: &str,
    data_root: &Path,
) -> Result<NodeRuntime> {
    let target = node_target_triple()?;
    let folder = format!("node-v{NODE_VERSION}-{target}");
    let node_root = data_root.join("runtimes").join("node").join(&folder);
    let node_bin = node_root.join("bin").join("node");
    let npm_cli_js = node_root
        .join("lib")
        .join("node_modules")
        .join("npm")
        .join("bin")
        .join("npm-cli.js");

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

    let url = format!(
        "https://nodejs.org/dist/v{NODE_VERSION}/{folder}.tar.gz"
    );
    let tmp = data_root.join("runtimes").join("node").join(format!("{folder}.tar.gz"));
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

    let extract_root = data_root.join("runtimes").join("node").join(format!("{folder}.extract"));
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
        let tar_gz = std::fs::File::open(&tmp2)?;
        let dec = flate2::read::GzDecoder::new(tar_gz);
        let mut archive = tar::Archive::new(dec);
        archive.unpack(&extract_root2)?;
        Ok(())
    })
    .await??;

    // Node tarballs contain a single top-level folder named `node-vX.Y.Z-<target>`.
    let extracted = extract_root.join(&folder);
    if !extracted.exists() {
        anyhow::bail!("node extraction failed: missing {folder} in {}", extract_root.display());
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

fn node_target_triple() -> Result<&'static str> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    match (os, arch) {
        ("macos", "aarch64") => Ok("darwin-arm64"),
        ("macos", "x86_64") => Ok("darwin-x64"),
        ("linux", "aarch64") => Ok("linux-arm64"),
        ("linux", "x86_64") => Ok("linux-x64"),
        _ => anyhow::bail!(
            "unsupported platform for managed node install: {os}/{arch}. Supported: macos (aarch64/x86_64), linux (aarch64/x86_64)."
        ),
    }
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
            tokio::time::sleep(Duration::from_millis(RETRY_BACKOFF_BASE_MS * attempt as u64))
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
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(DOWNLOAD_TIMEOUT)
            .build()
            .context("building http client")?;

        let attempt_res: Result<()> = async {
            let resp = client.get(url).send().await.context("sending request")?;
            let resp = resp.error_for_status().context("http error")?;

            let total = resp.content_length();
            let mut stream = resp.bytes_stream();
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent).await.ok();
            }
            let mut file = tokio::fs::File::create(path)
                .await
                .with_context(|| format!("creating download target: {}", path.display()))?;
            use futures::StreamExt;
            use tokio::io::AsyncWriteExt;

            let mut downloaded: u64 = 0;
            while let Some(chunk) = stream.next().await {
                let bytes = chunk.context("streaming download")?;
                downloaded += bytes.len() as u64;
                file.write_all(&bytes)
                    .await
                    .context("writing download")?;
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

async fn run_command_with_timeout(
    mut cmd: Command,
    dur: Duration,
) -> Result<std::process::Output> {
    let child = cmd.spawn().context("spawning process")?;
    let wait = async move { child.wait_with_output().await };
    match timeout(dur, wait).await {
        Ok(res) => Ok(res.context("waiting for process")?),
        Err(_) => anyhow::bail!("process timed out after {}s", dur.as_secs()),
    }
}
