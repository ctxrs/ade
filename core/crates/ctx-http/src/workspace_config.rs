use std::io;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use toml::Value as TomlValue;

use crate::settings::{
    ContainerMountMode, ContainerNetworkMode, ContainerRuntimeKind, ExecutionMode,
    ExecutionSettings,
};

pub const WORKSPACE_CONFIG_REL_PATH: &str = ".ctx/config.toml";
pub const DEFAULT_SYSTEM_PROMPT_APPEND: &str = "You are working inside ctx, an agent development environment. Use ctx MCP tools to attach photos/videos as artifacts, start persistent web sessions (Playwright REPL/scripts), and run sub-agents for research or well-scoped implementations. Check `.ctx/attachments/refs/` and `.ctx/attachments/docs/` for extra reference repos and docs.";
pub const DEFAULT_SUBAGENT_SYSTEM_PROMPT_APPEND: &str =
    "Subagents may use rg/grep and other token-heavy commands the main agent avoids.";

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

#[derive(Debug, Clone)]
pub struct SubagentSystemPromptAppendConfig {
    pub config_path: PathBuf,
    pub configured_append: Option<String>,
    pub default_append: String,
}

impl SubagentSystemPromptAppendConfig {
    pub fn new_default(root: &Path) -> Self {
        Self {
            config_path: root.join(WORKSPACE_CONFIG_REL_PATH),
            configured_append: None,
            default_append: DEFAULT_SUBAGENT_SYSTEM_PROMPT_APPEND.to_string(),
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
    subagents: Option<WorkspaceSubagentsConfig>,
    #[serde(default)]
    merge_queue: Option<WorkspaceMergeQueueConfig>,
    #[serde(default)]
    execution: Option<WorkspaceExecutionConfig>,
}

#[derive(Debug, Deserialize)]
struct WorkspaceAgentsConfig {
    #[serde(default)]
    system_prompt_append: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WorkspaceSubagentsConfig {
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
    push_on_success: Option<bool>,
    #[serde(default)]
    push_remote: Option<String>,
    #[serde(default)]
    push_branch: Option<String>,
    #[serde(default)]
    canonical_sync: Option<MergeQueueCanonicalSync>,
}

#[derive(Debug, Deserialize, Default)]
struct WorkspaceExecutionConfig {
    #[serde(default)]
    mode: Option<ExecutionMode>,
    #[serde(default)]
    container: Option<WorkspaceContainerExecutionConfig>,
}

#[derive(Debug, Deserialize, Default)]
struct WorkspaceContainerExecutionConfig {
    #[serde(default)]
    runtime: Option<ContainerRuntimeKind>,
    #[serde(default)]
    mount_mode: Option<ContainerMountMode>,
    #[serde(default)]
    network_mode: Option<ContainerNetworkMode>,
    #[serde(default)]
    allowlist: Option<Vec<String>>,
    #[serde(default)]
    image: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ExecutionSettingsOverride {
    pub mode: Option<ExecutionMode>,
    pub container: ContainerExecutionSettingsOverride,
}

#[derive(Debug, Clone, Default)]
pub struct ContainerExecutionSettingsOverride {
    pub runtime: Option<ContainerRuntimeKind>,
    pub mount_mode: Option<ContainerMountMode>,
    pub network_mode: Option<ContainerNetworkMode>,
    pub allowlist: Option<Vec<String>>,
    pub image: Option<String>,
}

pub async fn load_execution_settings_override(
    root: &Path,
) -> Result<Option<ExecutionSettingsOverride>> {
    let config_path = root.join(WORKSPACE_CONFIG_REL_PATH);
    let txt = match tokio::fs::read_to_string(&config_path).await {
        Ok(v) => v,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).context("reading .ctx/config.toml"),
    };
    let cfg: WorkspaceConfigFile = toml::from_str(&txt).context("parsing .ctx/config.toml")?;
    let Some(exec) = cfg.execution else {
        return Ok(None);
    };
    let mut ov = ExecutionSettingsOverride {
        mode: exec.mode,
        ..Default::default()
    };
    if let Some(c) = exec.container {
        ov.container.runtime = c.runtime;
        ov.container.mount_mode = c.mount_mode;
        ov.container.network_mode = c.network_mode;
        ov.container.allowlist = c.allowlist.map(|v| {
            v.into_iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        });
        ov.container.image = c
            .image
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
    }
    Ok(Some(ov))
}

pub fn apply_execution_settings_override(
    settings: &mut ExecutionSettings,
    ov: &ExecutionSettingsOverride,
) {
    if let Some(mode) = ov.mode.clone() {
        settings.mode = mode;
    }
    if let Some(runtime) = ov.container.runtime.clone() {
        settings.container.runtime = runtime;
    }
    if let Some(mount_mode) = ov.container.mount_mode.clone() {
        settings.container.mount_mode = mount_mode;
    }
    if let Some(network_mode) = ov.container.network_mode.clone() {
        settings.container.network_mode = network_mode;
    }
    if let Some(allowlist) = ov.container.allowlist.clone() {
        settings.container.allowlist = allowlist;
    }
    if let Some(image) = ov.container.image.clone() {
        settings.container.image = Some(image);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeQueueCanonicalSync {
    Never,
    CleanOnly,
    Force,
}

#[derive(Debug, Clone)]
pub struct MergeQueueConfig {
    pub config_path: PathBuf,
    pub enabled: bool,
    pub target_branch: String,
    pub verify_commands: Vec<String>,
    pub push_on_success: bool,
    pub push_remote: String,
    pub push_branch: String,
    pub canonical_sync: MergeQueueCanonicalSync,
}

impl MergeQueueConfig {
    pub fn new_default(root: &Path) -> Self {
        Self {
            config_path: root.join(WORKSPACE_CONFIG_REL_PATH),
            enabled: false,
            target_branch: "main".to_string(),
            verify_commands: Vec::new(),
            push_on_success: false,
            push_remote: "origin".to_string(),
            push_branch: "main".to_string(),
            canonical_sync: MergeQueueCanonicalSync::CleanOnly,
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

pub async fn load_subagent_system_prompt_append(
    root: &Path,
) -> Result<SubagentSystemPromptAppendConfig> {
    let config_path = root.join(WORKSPACE_CONFIG_REL_PATH);
    let configured_append = match tokio::fs::read_to_string(&config_path).await {
        Ok(text) => {
            let cfg: WorkspaceConfigFile =
                toml::from_str(&text).context("parsing .ctx/config.toml")?;
            cfg.subagents
                .and_then(|subagents| subagents.system_prompt_append)
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => None,
        Err(err) => return Err(err).context("reading .ctx/config.toml"),
    };

    Ok(SubagentSystemPromptAppendConfig {
        config_path,
        configured_append,
        default_append: DEFAULT_SUBAGENT_SYSTEM_PROMPT_APPEND.to_string(),
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
    if let Some(canonical_sync) = configured.canonical_sync {
        cfg.canonical_sync = canonical_sync;
    }

    Ok(cfg)
}

pub async fn load_merge_queue_target_branch_override(root: &Path) -> Result<Option<String>> {
    let config_path = root.join(WORKSPACE_CONFIG_REL_PATH);
    let configured = match tokio::fs::read_to_string(&config_path).await {
        Ok(text) => {
            let parsed: WorkspaceConfigFile =
                toml::from_str(&text).context("parsing .ctx/config.toml")?;
            parsed.merge_queue.and_then(|mq| mq.target_branch)
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => None,
        Err(err) => return Err(err).context("reading .ctx/config.toml"),
    };
    let Some(configured) = configured else {
        return Ok(None);
    };
    let trimmed = configured.trim().to_string();
    if trimmed.is_empty() {
        return Ok(None);
    }
    Ok(Some(trimmed))
}

#[derive(Debug, Clone)]
pub struct MergeQueueConfigUpdate {
    pub enabled: bool,
    pub target_branch: Option<String>,
    pub verify_commands: Vec<String>,
    pub push_on_success: Option<bool>,
    pub push_remote: Option<String>,
    pub push_branch: Option<String>,
    pub canonical_sync: Option<MergeQueueCanonicalSync>,
}

impl MergeQueueConfigUpdate {
    pub fn normalized(mut self) -> Self {
        self.target_branch = self
            .target_branch
            .as_ref()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());

        self.verify_commands = self
            .verify_commands
            .into_iter()
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty())
            .collect();

        self.push_remote = self
            .push_remote
            .as_ref()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());

        self.push_branch = self
            .push_branch
            .as_ref()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());

        self
    }
}

pub async fn update_merge_queue_config(
    root: &Path,
    update: MergeQueueConfigUpdate,
) -> Result<PathBuf> {
    let config_path = root.join(WORKSPACE_CONFIG_REL_PATH);
    let update = update.normalized();

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

    if !update.enabled {
        root_table.remove("merge_queue");
    } else {
        let mq_value = root_table
            .entry("merge_queue".to_string())
            .or_insert_with(|| TomlValue::Table(toml::value::Table::new()));
        let mq_table = mq_value
            .as_table_mut()
            .ok_or_else(|| anyhow!(".ctx/config.toml [merge_queue] must be a TOML table"))?;

        mq_table.insert("enabled".to_string(), TomlValue::Boolean(true));
        match update.target_branch {
            Some(target_branch) => {
                mq_table.insert(
                    "target_branch".to_string(),
                    TomlValue::String(target_branch),
                );
            }
            None => {
                mq_table.remove("target_branch");
            }
        }
        if !update.verify_commands.is_empty() {
            mq_table.insert(
                "verify_commands".to_string(),
                TomlValue::Array(
                    update
                        .verify_commands
                        .into_iter()
                        .map(TomlValue::String)
                        .collect(),
                ),
            );
        } else {
            mq_table.remove("verify_commands");
        }
        match update.push_on_success {
            Some(value) => {
                mq_table.insert("push_on_success".to_string(), TomlValue::Boolean(value));
            }
            None => {
                mq_table.remove("push_on_success");
            }
        }
        match update.push_remote {
            Some(value) => {
                mq_table.insert("push_remote".to_string(), TomlValue::String(value));
            }
            None => {
                mq_table.remove("push_remote");
            }
        }
        match update.push_branch {
            Some(value) => {
                mq_table.insert("push_branch".to_string(), TomlValue::String(value));
            }
            None => {
                mq_table.remove("push_branch");
            }
        }
        match update.canonical_sync {
            Some(value) => {
                mq_table.insert(
                    "canonical_sync".to_string(),
                    TomlValue::String(
                        match value {
                            MergeQueueCanonicalSync::Never => "never",
                            MergeQueueCanonicalSync::CleanOnly => "clean_only",
                            MergeQueueCanonicalSync::Force => "force",
                        }
                        .to_string(),
                    ),
                );
            }
            None => {
                mq_table.remove("canonical_sync");
            }
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

pub async fn update_worktree_bootstrap_setup_command(
    root: &Path,
    setup_command: Option<String>,
) -> Result<PathBuf> {
    let config_path = root.join(WORKSPACE_CONFIG_REL_PATH);
    let setup_command = setup_command
        .as_ref()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());

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

    if let Some(cmd) = setup_command {
        let worktree_value = root_table
            .entry("worktree".to_string())
            .or_insert_with(|| TomlValue::Table(toml::value::Table::new()));
        let worktree_table = worktree_value
            .as_table_mut()
            .ok_or_else(|| anyhow!(".ctx/config.toml [worktree] must be a TOML table"))?;
        let bootstrap_value = worktree_table
            .entry("bootstrap".to_string())
            .or_insert_with(|| TomlValue::Table(toml::value::Table::new()));
        let bootstrap_table = bootstrap_value
            .as_table_mut()
            .ok_or_else(|| anyhow!(".ctx/config.toml [worktree.bootstrap] must be a TOML table"))?;

        // `worktree_bootstrap` treats a scalar string as a script path. For a shell command, we
        // must store an array (even for a single command).
        bootstrap_table.insert(
            "setup_worktree".to_string(),
            TomlValue::Array(vec![TomlValue::String(cmd)]),
        );
        // Ensure we don't leave behind OS-specific overrides that would surprise the user.
        bootstrap_table.remove("setup_worktree_unix");
        bootstrap_table.remove("setup_worktree_windows");
    } else {
        // Remove setup key (and prune empty tables).
        if let Some(worktree_value) = root_table.get_mut("worktree") {
            let Some(worktree_table) = worktree_value.as_table_mut() else {
                return Err(anyhow!(".ctx/config.toml [worktree] must be a TOML table"));
            };
            if let Some(bootstrap_value) = worktree_table.get_mut("bootstrap") {
                let Some(bootstrap_table) = bootstrap_value.as_table_mut() else {
                    return Err(anyhow!(
                        ".ctx/config.toml [worktree.bootstrap] must be a TOML table"
                    ));
                };
                bootstrap_table.remove("setup_worktree");
                bootstrap_table.remove("setup_worktree_unix");
                bootstrap_table.remove("setup_worktree_windows");
                if bootstrap_table.is_empty() {
                    worktree_table.remove("bootstrap");
                }
            }
            if worktree_table.is_empty() {
                root_table.remove("worktree");
            }
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

#[derive(Debug, Clone)]
pub struct ExecutionConfigUpdate {
    pub mode: ExecutionMode,
    pub runtime: Option<ContainerRuntimeKind>,
    pub mount_mode: Option<ContainerMountMode>,
    pub network_mode: Option<ContainerNetworkMode>,
    pub allowlist: Option<Vec<String>>,
    pub image: Option<String>,
}

pub async fn update_execution_config(
    root: &Path,
    update: ExecutionConfigUpdate,
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

    let exec_value = root_table
        .entry("execution".to_string())
        .or_insert_with(|| TomlValue::Table(toml::value::Table::new()));
    let exec_table = exec_value
        .as_table_mut()
        .ok_or_else(|| anyhow!(".ctx/config.toml [execution] must be a TOML table"))?;

    exec_table.insert(
        "mode".to_string(),
        TomlValue::String(
            match update.mode {
                ExecutionMode::Host => "host",
                ExecutionMode::Auto => "auto",
                ExecutionMode::Container => "container",
            }
            .to_string(),
        ),
    );

    if matches!(update.mode, ExecutionMode::Container) {
        let container_value = exec_table
            .entry("container".to_string())
            .or_insert_with(|| TomlValue::Table(toml::value::Table::new()));
        let container_table = container_value.as_table_mut().ok_or_else(|| {
            anyhow!(".ctx/config.toml [execution.container] must be a TOML table")
        })?;

        if let Some(runtime) = update.runtime {
            container_table.insert(
                "runtime".to_string(),
                TomlValue::String(
                    match runtime {
                        ContainerRuntimeKind::Podman => "podman",
                    }
                    .to_string(),
                ),
            );
        } else {
            container_table.remove("runtime");
        }
        if let Some(mount_mode) = update.mount_mode {
            container_table.insert(
                "mount_mode".to_string(),
                TomlValue::String(
                    match mount_mode {
                        ContainerMountMode::HostMounted => "host_mounted",
                        ContainerMountMode::Sealed => "sealed",
                        ContainerMountMode::DiskIsolated => "disk_isolated",
                    }
                    .to_string(),
                ),
            );
        } else {
            container_table.remove("mount_mode");
        }
        if let Some(network_mode) = update.network_mode {
            container_table.insert(
                "network_mode".to_string(),
                TomlValue::String(
                    match network_mode {
                        ContainerNetworkMode::LlmOnly => "llm_only",
                        ContainerNetworkMode::Allowlist => "allowlist",
                        ContainerNetworkMode::All => "all",
                    }
                    .to_string(),
                ),
            );
        } else {
            container_table.remove("network_mode");
        }
        if let Some(allowlist) = update.allowlist {
            let values: Vec<TomlValue> = allowlist
                .into_iter()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .map(TomlValue::String)
                .collect();
            if values.is_empty() {
                container_table.remove("allowlist");
            } else {
                container_table.insert("allowlist".to_string(), TomlValue::Array(values));
            }
        } else {
            container_table.remove("allowlist");
        }
        if let Some(image) = update.image {
            let image = image.trim().to_string();
            if image.is_empty() {
                container_table.remove("image");
            } else {
                container_table.insert("image".to_string(), TomlValue::String(image));
            }
        } else {
            container_table.remove("image");
        }

        if container_table.is_empty() {
            exec_table.remove("container");
        }
    } else {
        exec_table.remove("container");
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

pub async fn update_subagent_system_prompt_append(
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
        let subagents_value = root_table
            .entry("subagents".to_string())
            .or_insert_with(|| TomlValue::Table(toml::value::Table::new()));
        let subagents_table = subagents_value
            .as_table_mut()
            .ok_or_else(|| anyhow!(".ctx/config.toml [subagents] must be a TOML table"))?;
        subagents_table.insert(
            "system_prompt_append".to_string(),
            TomlValue::String(trimmed),
        );
    } else if let Some(subagents_value) = root_table.get_mut("subagents") {
        let subagents_table = subagents_value
            .as_table_mut()
            .ok_or_else(|| anyhow!(".ctx/config.toml [subagents] must be a TOML table"))?;
        subagents_table.remove("system_prompt_append");
        if subagents_table.is_empty() {
            root_table.remove("subagents");
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
