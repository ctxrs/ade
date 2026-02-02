use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use ctx_lsp::LspManagerConfig;

use crate::bundled_assets;
use crate::installs::truncate_for_storage;

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

pub fn resolve_provider_command(
    cfg: &AgentServerConfigFile,
    provider_id: &str,
) -> Option<AgentServerCommand> {
    if let Some(bundled) = bundled_assets::bundled_provider_command(provider_id) {
        return Some(AgentServerCommand {
            command: bundled.command,
            args: bundled.args,
            dependencies: Vec::new(),
            managed: None,
        });
    }
    cfg.providers.get(provider_id).cloned()
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
