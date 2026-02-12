use anyhow::{Context, Result};
use ctx_store::Store;
use serde::{Deserialize, Serialize};

use crate::settings::{
    ContainerMountMode, ContainerNetworkMode, ContainerRuntimeKind, ExecutionMode,
    ExecutionSettings,
};

const WORKSPACE_SETTINGS_SCHEMA_VERSION: i64 = 1;

pub const DEFAULT_SYSTEM_PROMPT_APPEND: &str = "You are working inside ctx, an agent development environment. Use ctx MCP tools to attach photos/videos as artifacts, start persistent web sessions (Playwright REPL/scripts), and run sub-agents for research or well-scoped implementations. Check `.ctx/attachments/refs/` and `.ctx/attachments/docs/` for extra reference repos and docs.";
pub const DEFAULT_SUBAGENT_SYSTEM_PROMPT_APPEND: &str =
    "Subagents may use rg/grep and other token-heavy commands the main agent avoids.";

#[derive(Debug, Clone)]
pub struct AgentSystemPromptAppendConfig {
    pub configured_append: Option<String>,
    pub default_append: String,
}

impl AgentSystemPromptAppendConfig {
    pub fn new_default() -> Self {
        Self {
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
    pub configured_append: Option<String>,
    pub default_append: String,
}

impl SubagentSystemPromptAppendConfig {
    pub fn new_default() -> Self {
        Self {
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

#[derive(Debug, Deserialize, Serialize, Default, Clone)]
struct WorkspaceRuntimeSettingsDoc {
    #[serde(default)]
    agents: Option<WorkspaceAgentsConfig>,
    #[serde(default)]
    subagents: Option<WorkspaceSubagentsConfig>,
    #[serde(default)]
    merge_queue: Option<WorkspaceMergeQueueConfig>,
    #[serde(default)]
    execution: Option<WorkspaceExecutionConfig>,
    #[serde(default)]
    worktree_bootstrap: Option<WorkspaceWorktreeBootstrapConfig>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct WorkspaceAgentsConfig {
    #[serde(default)]
    system_prompt_append: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct WorkspaceSubagentsConfig {
    #[serde(default)]
    system_prompt_append: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Default, Clone)]
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

#[derive(Debug, Deserialize, Serialize, Default, Clone)]
struct WorkspaceExecutionConfig {
    #[serde(default)]
    environment: Option<ExecutionEnvironment>,
    #[serde(default)]
    container: Option<WorkspaceContainerExecutionConfig>,
}

#[derive(Debug, Deserialize, Serialize, Default, Clone)]
struct WorkspaceContainerExecutionConfig {
    #[serde(default)]
    runtime: Option<ContainerRuntimeKind>,
    #[serde(default)]
    network_mode: Option<ContainerNetworkMode>,
    #[serde(default)]
    allowlist: Option<Vec<String>>,
    #[serde(default)]
    image: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Default, Clone)]
struct WorkspaceWorktreeBootstrapConfig {
    #[serde(default)]
    setup_command: Option<String>,
    #[serde(default)]
    timeout_sec: Option<u64>,
    #[serde(default)]
    wait_for_completion: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct ExecutionSettingsOverride {
    pub mode: Option<ExecutionMode>,
    pub container: ContainerExecutionSettingsOverride,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionEnvironment {
    Host,
    ContainerHostMounted,
    ContainerDiskIsolated,
}

impl ExecutionEnvironment {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::ContainerHostMounted => "container_host_mounted",
            Self::ContainerDiskIsolated => "container_disk_isolated",
        }
    }
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
    store: &Store,
) -> Result<Option<ExecutionSettingsOverride>> {
    let cfg = load_workspace_settings_doc(store).await?;
    let Some(exec) = cfg.execution else {
        return Ok(None);
    };

    let mut ov = ExecutionSettingsOverride::default();
    let environment = exec.environment;
    if let Some(environment) = environment {
        match environment {
            ExecutionEnvironment::Host => {
                ov.mode = Some(ExecutionMode::Host);
            }
            ExecutionEnvironment::ContainerHostMounted => {
                ov.mode = Some(ExecutionMode::Container);
                ov.container.mount_mode = Some(ContainerMountMode::HostMounted);
            }
            ExecutionEnvironment::ContainerDiskIsolated => {
                ov.mode = Some(ExecutionMode::Container);
                ov.container.mount_mode = Some(ContainerMountMode::DiskIsolated);
            }
        }
    }

    if let Some(c) = exec.container {
        ov.container.runtime = c.runtime;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeQueueCanonicalSync {
    Never,
    CleanOnly,
    Force,
}

#[derive(Debug, Clone)]
pub struct MergeQueueConfig {
    pub enabled: bool,
    pub target_branch: String,
    pub verify_commands: Vec<String>,
    pub push_on_success: bool,
    pub push_remote: String,
    pub push_branch: String,
    pub canonical_sync: MergeQueueCanonicalSync,
}

impl MergeQueueConfig {
    pub fn new_default() -> Self {
        Self {
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

pub async fn load_agent_system_prompt_append(
    store: &Store,
) -> Result<AgentSystemPromptAppendConfig> {
    let cfg = load_workspace_settings_doc(store).await?;
    let configured_append = cfg.agents.and_then(|agents| agents.system_prompt_append);

    Ok(AgentSystemPromptAppendConfig {
        configured_append,
        default_append: DEFAULT_SYSTEM_PROMPT_APPEND.to_string(),
    })
}

pub async fn load_subagent_system_prompt_append(
    store: &Store,
) -> Result<SubagentSystemPromptAppendConfig> {
    let cfg = load_workspace_settings_doc(store).await?;
    let configured_append = cfg
        .subagents
        .and_then(|subagents| subagents.system_prompt_append);

    Ok(SubagentSystemPromptAppendConfig {
        configured_append,
        default_append: DEFAULT_SUBAGENT_SYSTEM_PROMPT_APPEND.to_string(),
    })
}

pub async fn load_merge_queue_config(store: &Store) -> Result<MergeQueueConfig> {
    let runtime_cfg = load_workspace_settings_doc(store).await?;
    let mut cfg = MergeQueueConfig::new_default();

    let Some(configured) = runtime_cfg.merge_queue else {
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

pub async fn load_merge_queue_target_branch_override(store: &Store) -> Result<Option<String>> {
    let runtime_cfg = load_workspace_settings_doc(store).await?;
    let configured = runtime_cfg.merge_queue.and_then(|mq| mq.target_branch);
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
    store: &Store,
    update: MergeQueueConfigUpdate,
) -> Result<()> {
    let mut cfg = load_workspace_settings_doc(store).await?;
    let update = update.normalized();

    if !update.enabled {
        cfg.merge_queue = None;
        return save_workspace_settings_doc(store, &cfg).await;
    }

    cfg.merge_queue = Some(WorkspaceMergeQueueConfig {
        enabled: Some(true),
        target_branch: update.target_branch,
        verify_commands: if update.verify_commands.is_empty() {
            None
        } else {
            Some(update.verify_commands)
        },
        push_on_success: update.push_on_success,
        push_remote: update.push_remote,
        push_branch: update.push_branch,
        canonical_sync: update.canonical_sync,
    });

    save_workspace_settings_doc(store, &cfg).await
}

#[derive(Debug, Clone)]
pub struct WorktreeBootstrapConfig {
    pub setup_command: Option<String>,
    pub timeout_sec: Option<u64>,
    pub wait_for_completion: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct WorktreeBootstrapConfigUpdate {
    pub setup_command: Option<String>,
    pub timeout_sec: Option<u64>,
    pub wait_for_completion: Option<bool>,
}

pub async fn load_worktree_bootstrap_config(
    store: &Store,
) -> Result<Option<WorktreeBootstrapConfig>> {
    let cfg = load_workspace_settings_doc(store).await?;
    let bootstrap = cfg.worktree_bootstrap.map(|b| WorktreeBootstrapConfig {
        setup_command: b
            .setup_command
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        timeout_sec: b.timeout_sec,
        wait_for_completion: b.wait_for_completion,
    });
    Ok(bootstrap)
}

pub async fn update_worktree_bootstrap_config(
    store: &Store,
    update: WorktreeBootstrapConfigUpdate,
) -> Result<()> {
    let mut cfg = load_workspace_settings_doc(store).await?;

    let setup_command = update
        .setup_command
        .as_ref()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());

    if setup_command.is_none()
        && update.timeout_sec.is_none()
        && update.wait_for_completion.is_none()
    {
        cfg.worktree_bootstrap = None;
    } else {
        cfg.worktree_bootstrap = Some(WorkspaceWorktreeBootstrapConfig {
            setup_command,
            timeout_sec: update.timeout_sec,
            wait_for_completion: update.wait_for_completion,
        });
    }

    save_workspace_settings_doc(store, &cfg).await
}

#[derive(Debug, Clone)]
pub struct ExecutionConfigUpdate {
    pub environment: ExecutionEnvironment,
    pub runtime: Option<ContainerRuntimeKind>,
    pub network_mode: Option<ContainerNetworkMode>,
    pub allowlist: Option<Vec<String>>,
    pub image: Option<String>,
}

pub async fn update_execution_config(store: &Store, update: ExecutionConfigUpdate) -> Result<()> {
    let mut cfg = load_workspace_settings_doc(store).await?;

    let container = if matches!(
        update.environment,
        ExecutionEnvironment::ContainerHostMounted | ExecutionEnvironment::ContainerDiskIsolated
    ) {
        Some(WorkspaceContainerExecutionConfig {
            runtime: update.runtime,
            network_mode: update.network_mode,
            allowlist: update.allowlist.map(|v| {
                v.into_iter()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
            }),
            image: update
                .image
                .as_ref()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
        })
    } else {
        None
    };

    cfg.execution = Some(WorkspaceExecutionConfig {
        environment: Some(update.environment),
        container,
    });

    save_workspace_settings_doc(store, &cfg).await
}

pub async fn update_agent_system_prompt_append(
    store: &Store,
    system_prompt_append: Option<String>,
) -> Result<()> {
    let mut cfg = load_workspace_settings_doc(store).await?;
    match system_prompt_append {
        Some(value) => {
            let trimmed = value.trim().to_string();
            cfg.agents = Some(WorkspaceAgentsConfig {
                system_prompt_append: Some(trimmed),
            });
        }
        None => cfg.agents = None,
    }

    save_workspace_settings_doc(store, &cfg).await
}

pub async fn update_subagent_system_prompt_append(
    store: &Store,
    system_prompt_append: Option<String>,
) -> Result<()> {
    let mut cfg = load_workspace_settings_doc(store).await?;
    match system_prompt_append {
        Some(value) => {
            let trimmed = value.trim().to_string();
            cfg.subagents = Some(WorkspaceSubagentsConfig {
                system_prompt_append: Some(trimmed),
            });
        }
        None => cfg.subagents = None,
    }

    save_workspace_settings_doc(store, &cfg).await
}

async fn load_workspace_settings_doc(store: &Store) -> Result<WorkspaceRuntimeSettingsDoc> {
    let Some(doc) = store.get_runtime_settings_document().await? else {
        return Ok(WorkspaceRuntimeSettingsDoc::default());
    };

    let parsed = serde_json::from_str::<WorkspaceRuntimeSettingsDoc>(&doc.settings_json)
        .context("parsing workspace runtime settings document")?;
    Ok(parsed)
}

async fn save_workspace_settings_doc(
    store: &Store,
    cfg: &WorkspaceRuntimeSettingsDoc,
) -> Result<()> {
    let settings_json = serde_json::to_string_pretty(cfg)?;
    store
        .upsert_runtime_settings_document(WORKSPACE_SETTINGS_SCHEMA_VERSION, &settings_json)
        .await?;
    Ok(())
}

fn trimmed_nonempty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}
