use std::io;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use toml::Value as TomlValue;

pub const WORKSPACE_CONFIG_REL_PATH: &str = ".ctx/config.toml";
pub const DEFAULT_SYSTEM_PROMPT_APPEND: &str = "You are working inside ctx, an agent development environment. Use ctx MCP tools to attach photos/videos as artifacts, start persistent web sessions (Playwright REPL/scripts), and run sub-agents for research or well-scoped implementations. Check `.ctx/.refs/` and `.ctx/docs/` for extra reference repos and docs.";

#[derive(Debug, Clone)]
pub struct AgentSystemPromptAppendConfig {
    pub config_path: PathBuf,
    pub configured_append: Option<String>,
    pub default_append: String,
}

impl AgentSystemPromptAppendConfig {
    pub fn new_default(root: &Path) -> Self {
        Self {
            config_path: root.join(WORKSPACE_CONFIG_REL_PATH),
            configured_append: None,
            default_append: DEFAULT_SYSTEM_PROMPT_APPEND.to_string(),
        }
    }

    pub fn effective_append(&self) -> Option<String> {
        match self.configured_append.as_deref() {
            Some(value) => trimmed_nonempty(value),
            None => trimmed_nonempty(&self.default_append),
        }
    }

    pub fn source(&self) -> AgentSystemPromptAppendSource {
        match self.configured_append.as_deref() {
            Some(value) => {
                if value.trim().is_empty() {
                    AgentSystemPromptAppendSource::Disabled
                } else {
                    AgentSystemPromptAppendSource::Config
                }
            }
            None => {
                if self.default_append.trim().is_empty() {
                    AgentSystemPromptAppendSource::Disabled
                } else {
                    AgentSystemPromptAppendSource::Default
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentSystemPromptAppendSource {
    Default,
    Config,
    Disabled,
}

#[derive(Debug, Deserialize)]
struct WorkspaceConfigFile {
    #[serde(default)]
    agents: Option<WorkspaceAgentsConfig>,
    #[serde(default)]
    merge_queue: Option<WorkspaceMergeQueueConfig>,
}

#[derive(Debug, Deserialize)]
struct WorkspaceAgentsConfig {
    #[serde(default)]
    system_prompt_append: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct WorkspaceMergeQueueConfig {
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    target_branch: Option<String>,
    #[serde(default)]
    verify_commands: Option<Vec<String>>,
    #[serde(default)]
    halt_on_fail: Option<bool>,
    #[serde(default)]
    push_on_success: Option<bool>,
    #[serde(default)]
    push_remote: Option<String>,
    #[serde(default)]
    push_branch: Option<String>,
}

#[derive(Debug, Clone)]
pub struct MergeQueueConfig {
    pub config_path: PathBuf,
    pub enabled: bool,
    pub target_branch: String,
    pub verify_commands: Vec<String>,
    pub halt_on_fail: bool,
    pub push_on_success: bool,
    pub push_remote: String,
    pub push_branch: String,
}

impl MergeQueueConfig {
    pub fn new_default(root: &Path) -> Self {
        Self {
            config_path: root.join(WORKSPACE_CONFIG_REL_PATH),
            enabled: false,
            target_branch: "main".to_string(),
            verify_commands: Vec::new(),
            halt_on_fail: true,
            push_on_success: false,
            push_remote: "origin".to_string(),
            push_branch: "main".to_string(),
        }
    }
}

pub async fn load_agent_system_prompt_append(root: &Path) -> Result<AgentSystemPromptAppendConfig> {
    let config_path = root.join(WORKSPACE_CONFIG_REL_PATH);
    let configured_append = match tokio::fs::read_to_string(&config_path).await {
        Ok(text) => {
            let cfg: WorkspaceConfigFile =
                toml::from_str(&text).context("parsing .ctx/config.toml")?;
            cfg.agents.and_then(|agents| agents.system_prompt_append)
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => None,
        Err(err) => return Err(err).context("reading .ctx/config.toml"),
    };

    Ok(AgentSystemPromptAppendConfig {
        config_path,
        configured_append,
        default_append: DEFAULT_SYSTEM_PROMPT_APPEND.to_string(),
    })
}

pub async fn load_merge_queue_config(root: &Path) -> Result<MergeQueueConfig> {
    let config_path = root.join(WORKSPACE_CONFIG_REL_PATH);
    let mut cfg = MergeQueueConfig::new_default(root);
    let configured = match tokio::fs::read_to_string(&config_path).await {
        Ok(text) => {
            let parsed: WorkspaceConfigFile =
                toml::from_str(&text).context("parsing .ctx/config.toml")?;
            parsed.merge_queue
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => None,
        Err(err) => return Err(err).context("reading .ctx/config.toml"),
    };

    let Some(configured) = configured else {
        return Ok(cfg);
    };

    if let Some(enabled) = configured.enabled {
        cfg.enabled = enabled;
    }
    if let Some(target_branch) = configured.target_branch {
        let trimmed = target_branch.trim().to_string();
        if !trimmed.is_empty() {
            cfg.target_branch = trimmed;
        }
    }
    if let Some(verify_commands) = configured.verify_commands {
        let trimmed = verify_commands
            .into_iter()
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty())
            .collect::<Vec<_>>();
        if !trimmed.is_empty() {
            cfg.verify_commands = trimmed;
        }
    }
    if let Some(halt_on_fail) = configured.halt_on_fail {
        cfg.halt_on_fail = halt_on_fail;
    }
    if let Some(push_on_success) = configured.push_on_success {
        cfg.push_on_success = push_on_success;
    }
    if let Some(push_remote) = configured.push_remote {
        let trimmed = push_remote.trim().to_string();
        if !trimmed.is_empty() {
            cfg.push_remote = trimmed;
        }
    }
    if let Some(push_branch) = configured.push_branch {
        let trimmed = push_branch.trim().to_string();
        cfg.push_branch = if trimmed.is_empty() {
            cfg.target_branch.clone()
        } else {
            trimmed
        };
    } else {
        cfg.push_branch = cfg.target_branch.clone();
    }

    Ok(cfg)
}

pub async fn update_agent_system_prompt_append(
    root: &Path,
    system_prompt_append: Option<String>,
) -> Result<PathBuf> {
    let config_path = root.join(WORKSPACE_CONFIG_REL_PATH);
    let mut root_table = if config_path.exists() {
        let text = tokio::fs::read_to_string(&config_path)
            .await
            .context("reading .ctx/config.toml")?;
        match toml::from_str::<TomlValue>(&text).context("parsing .ctx/config.toml")? {
            TomlValue::Table(table) => table,
            _ => {
                return Err(anyhow!(
                    ".ctx/config.toml must contain a TOML table at the root"
                ))
            }
        }
    } else {
        toml::value::Table::new()
    };

    if let Some(value) = system_prompt_append {
        let trimmed = value.trim().to_string();
        let agents_value = root_table
            .entry("agents".to_string())
            .or_insert_with(|| TomlValue::Table(toml::value::Table::new()));
        let agents_table = agents_value
            .as_table_mut()
            .ok_or_else(|| anyhow!(".ctx/config.toml [agents] must be a TOML table"))?;
        agents_table.insert(
            "system_prompt_append".to_string(),
            TomlValue::String(trimmed),
        );
    } else if let Some(agents_value) = root_table.get_mut("agents") {
        let agents_table = agents_value
            .as_table_mut()
            .ok_or_else(|| anyhow!(".ctx/config.toml [agents] must be a TOML table"))?;
        agents_table.remove("system_prompt_append");
        if agents_table.is_empty() {
            root_table.remove("agents");
        }
    }

    if root_table.is_empty() {
        if config_path.exists() {
            tokio::fs::remove_file(&config_path)
                .await
                .context("removing empty .ctx/config.toml")?;
        }
        return Ok(config_path);
    }

    if let Some(parent) = config_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .context("creating .ctx directory")?;
    }
    let serialized = toml::to_string_pretty(&TomlValue::Table(root_table))
        .context("serializing .ctx/config.toml")?;
    tokio::fs::write(&config_path, serialized)
        .await
        .context("writing .ctx/config.toml")?;
    Ok(config_path)
}

fn trimmed_nonempty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}
