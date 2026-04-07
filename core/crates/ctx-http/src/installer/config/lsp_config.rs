use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use ctx_lsp::LspManagerConfig;

use super::{AgentServerCommand, ManagedInstallMetadata};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct LspServerConfigFile {
    #[serde(default)]
    pub servers: HashMap<String, AgentServerCommand>,
    #[serde(default)]
    pub extra_servers: Vec<UserLspServerSpec>,
    #[serde(default)]
    pub managed_installs: HashMap<String, ManagedInstallMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserLspServerSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub language_id: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default)]
    pub filenames: Vec<String>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct UserLspConfigFile {
    #[serde(default)]
    pub servers: Vec<UserLspServerSpec>,
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

fn apply_server_command(
    cfg: &mut LspManagerConfig,
    language_id: &str,
    command: String,
    args: Vec<String>,
) {
    match language_id {
        "rust" => {
            cfg.rust_command = command;
            cfg.rust_args = args;
        }
        "typescript" | "javascript" => {
            cfg.ts_command = command;
            cfg.ts_args = args;
        }
        "python" => {
            cfg.py_command = command;
            cfg.py_args = args;
        }
        "go" => {
            cfg.go_command = command;
            cfg.go_args = args;
        }
        "html" => {
            cfg.html_command = command;
            cfg.html_args = args;
        }
        "css" => {
            cfg.css_command = command;
            cfg.css_args = args;
        }
        "json" => {
            cfg.json_command = command;
            cfg.json_args = args;
        }
        "yaml" => {
            cfg.yaml_command = command;
            cfg.yaml_args = args;
        }
        "bash" => {
            cfg.bash_command = command;
            cfg.bash_args = args;
        }
        "dockerfile" => {
            cfg.dockerfile_command = command;
            cfg.dockerfile_args = args;
        }
        "cpp" | "c" => {
            cfg.clangd_command = command;
            cfg.clangd_args = args;
        }
        "lua" => {
            cfg.lua_command = command;
            cfg.lua_args = args;
        }
        "toml" => {
            cfg.toml_command = command;
            cfg.toml_args = args;
        }
        "markdown" => {
            cfg.markdown_command = command;
            cfg.markdown_args = args;
        }
        other => {
            cfg.custom_servers
                .insert(other.to_string(), (command, args));
        }
    }
}

fn apply_server_mappings(
    cfg: &mut LspManagerConfig,
    language_id: &str,
    extensions: Vec<String>,
    filenames: Vec<String>,
) {
    let language_id = language_id.trim().to_string();
    if language_id.is_empty() {
        return;
    }
    for ext in extensions {
        let ext = ext.trim().trim_start_matches('.').to_ascii_lowercase();
        if !ext.is_empty() {
            cfg.custom_extension_map.insert(ext, language_id.clone());
        }
    }
    for name in filenames {
        let name = name.trim().to_string();
        if !name.is_empty() {
            cfg.custom_filename_map.insert(name, language_id.clone());
        }
    }
}

pub async fn apply_managed_lsp_server_config(
    data_root: &Path,
    cfg: &mut LspManagerConfig,
) -> Result<()> {
    let installed = load_lsp_server_config(data_root).await.unwrap_or_default();
    for (language_id, cmd) in installed.servers {
        apply_server_command(cfg, &language_id, cmd.command, cmd.args);
    }
    for server in installed.extra_servers {
        let language_id = server.language_id.trim().to_string();
        if language_id.is_empty() || server.command.trim().is_empty() {
            continue;
        }
        apply_server_command(cfg, &language_id, server.command, server.args);
        apply_server_mappings(cfg, &language_id, server.extensions, server.filenames);
    }
    Ok(())
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
        apply_server_command(cfg, &language_id, server.command, server.args);
        apply_server_mappings(cfg, &language_id, server.extensions, server.filenames);
    }
    Ok(())
}
