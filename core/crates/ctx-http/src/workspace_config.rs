use std::collections::HashMap;
use std::sync::OnceLock;

use anyhow::{bail, Context, Result};
pub use ctx_core::models::ExecutionEnvironment;
use ctx_store::Store;
use serde::{Deserialize, Deserializer, Serialize};
use tokio::sync::Mutex as AsyncMutex;

use crate::settings::{ContainerNetworkMode, ExecutionMode, ExecutionSettings};

const WORKSPACE_SETTINGS_SCHEMA_VERSION: i64 = 1;

pub const DEFAULT_SYSTEM_PROMPT_APPEND: &str = "You are working inside ctx, an agent development environment. Use ctx MCP tools to attach photos/videos as artifacts, start persistent web sessions (Playwright REPL/scripts), and run sub-agents for research or well-scoped implementations. Check `.ctx/attachments/refs/` and `.ctx/attachments/docs/` for extra reference repos and docs.";
pub const DEFAULT_SUBAGENT_SYSTEM_PROMPT_APPEND: &str = "You are a subagent. The user messaging you is the primary agent who will provide your instructions.";

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
    new_session: Option<WorkspaceNewSessionConfig>,
    #[serde(default)]
    vcs: Option<WorkspaceVcsConfig>,
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
struct WorkspaceNewSessionConfig {
    #[serde(default, deserialize_with = "deserialize_optional_string_map")]
    preferred_model_by_provider: Option<HashMap<String, String>>,
}

#[derive(Debug, Deserialize, Serialize, Default, Clone)]
struct WorkspaceVcsConfig {
    #[serde(default)]
    primary_branch: Option<String>,
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

#[derive(Debug, Clone, Default)]
pub struct ContainerExecutionSettingsOverride {
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
            ExecutionEnvironment::Sandbox => {
                ov.mode = Some(ExecutionMode::Sandbox);
            }
        }
    }

    if let Some(c) = exec.container {
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

pub async fn load_primary_branch(store: &Store) -> Result<Option<String>> {
    let cfg = load_workspace_settings_doc(store).await?;
    let configured = cfg.vcs.and_then(|vcs| vcs.primary_branch);
    let Some(configured) = configured else {
        return Ok(None);
    };
    let trimmed = configured.trim().to_string();
    if trimmed.is_empty() {
        return Ok(None);
    }
    Ok(Some(trimmed))
}

pub async fn load_preferred_new_session_model_id(
    store: &Store,
    provider_id: &str,
) -> Result<Option<String>> {
    let provider_id = normalize_provider_preference_key(provider_id);
    let Some(provider_id) = provider_id else {
        return Ok(None);
    };
    let prefs = load_preferred_new_session_models(store).await?;
    Ok(prefs.get(&provider_id).cloned())
}

pub async fn load_preferred_new_session_models(store: &Store) -> Result<HashMap<String, String>> {
    let cfg = load_workspace_settings_doc(store).await?;
    let raw = cfg
        .new_session
        .and_then(|new_session| new_session.preferred_model_by_provider)
        .unwrap_or_default();
    let mut normalized = HashMap::new();
    for (provider_id, model_id) in raw {
        let Some(provider_id) = normalize_provider_preference_key(&provider_id) else {
            continue;
        };
        let Some(model_id) = trimmed_nonempty(&model_id) else {
            continue;
        };
        normalized.insert(provider_id, model_id);
    }
    Ok(normalized)
}

pub async fn update_preferred_new_session_model_id(
    store: &Store,
    provider_id: &str,
    model_id: Option<String>,
) -> Result<()> {
    let Some(provider_id) = normalize_provider_preference_key(provider_id) else {
        bail!("provider_id is required");
    };
    mutate_workspace_settings_doc(store, "preferred_new_session_model", move |cfg| {
        let mut prefs = cfg
            .new_session
            .as_ref()
            .and_then(|new_session| new_session.preferred_model_by_provider.clone())
            .unwrap_or_default();
        if let Some(model_id) = model_id.as_deref().and_then(trimmed_nonempty) {
            prefs.insert(provider_id, model_id);
        } else {
            prefs.remove(&provider_id);
        }

        if prefs.is_empty() {
            cfg.new_session = None;
        } else {
            cfg.new_session = Some(WorkspaceNewSessionConfig {
                preferred_model_by_provider: Some(prefs),
            });
        }
        Ok(())
    })
    .await
}

pub async fn update_primary_branch(store: &Store, primary_branch: &str) -> Result<()> {
    let trimmed = primary_branch.trim().to_string();
    if trimmed.is_empty() {
        bail!("primary_branch is required");
    }
    mutate_workspace_settings_doc(store, "primary_branch", move |cfg| {
        cfg.vcs = Some(WorkspaceVcsConfig {
            primary_branch: Some(trimmed),
        });
        Ok(())
    })
    .await
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
    let update = update.normalized();
    mutate_workspace_settings_doc(store, "merge_queue", move |cfg| {
        if !update.enabled {
            cfg.merge_queue = None;
            return Ok(());
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

        Ok(())
    })
    .await
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
    let setup_command = update
        .setup_command
        .as_ref()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    mutate_workspace_settings_doc(store, "worktree_bootstrap", move |cfg| {
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
        Ok(())
    })
    .await
}

#[derive(Debug, Clone)]
pub struct ExecutionConfigUpdate {
    pub environment: ExecutionEnvironment,
    pub network_mode: Option<ContainerNetworkMode>,
    pub allowlist: Option<Vec<String>>,
    pub image: Option<String>,
}

pub async fn update_execution_config(store: &Store, update: ExecutionConfigUpdate) -> Result<()> {
    let container = if matches!(update.environment, ExecutionEnvironment::Sandbox) {
        Some(WorkspaceContainerExecutionConfig {
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
    mutate_workspace_settings_doc(store, "execution", move |cfg| {
        cfg.execution = Some(WorkspaceExecutionConfig {
            environment: Some(update.environment),
            container,
        });
        Ok(())
    })
    .await
}

pub async fn update_agent_system_prompt_append(
    store: &Store,
    system_prompt_append: Option<String>,
) -> Result<()> {
    mutate_workspace_settings_doc(store, "agents", move |cfg| {
        match system_prompt_append {
            Some(value) => {
                let trimmed = value.trim().to_string();
                cfg.agents = Some(WorkspaceAgentsConfig {
                    system_prompt_append: Some(trimmed),
                });
            }
            None => cfg.agents = None,
        }
        Ok(())
    })
    .await
}

pub async fn update_subagent_system_prompt_append(
    store: &Store,
    system_prompt_append: Option<String>,
) -> Result<()> {
    mutate_workspace_settings_doc(store, "subagents", move |cfg| {
        match system_prompt_append {
            Some(value) => {
                let trimmed = value.trim().to_string();
                cfg.subagents = Some(WorkspaceSubagentsConfig {
                    system_prompt_append: Some(trimmed),
                });
            }
            None => cfg.subagents = None,
        }
        Ok(())
    })
    .await
}

async fn load_workspace_settings_doc(store: &Store) -> Result<WorkspaceRuntimeSettingsDoc> {
    let Some(doc) = store.get_runtime_settings_document().await? else {
        return Ok(WorkspaceRuntimeSettingsDoc::default());
    };

    let parsed = serde_json::from_str::<WorkspaceRuntimeSettingsDoc>(&doc.settings_json)
        .context("parsing workspace runtime settings document")?;
    Ok(parsed)
}

fn workspace_settings_write_lock() -> &'static AsyncMutex<()> {
    static LOCK: OnceLock<AsyncMutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| AsyncMutex::new(()))
}

async fn mutate_workspace_settings_doc(
    store: &Store,
    operation: &'static str,
    mutate: impl FnOnce(&mut WorkspaceRuntimeSettingsDoc) -> Result<()>,
) -> Result<()> {
    let _guard = workspace_settings_write_lock().lock().await;
    let mut cfg = load_workspace_settings_doc(store).await?;
    #[cfg(test)]
    pause_after_workspace_settings_load_for_tests(operation).await;
    mutate(&mut cfg)?;
    save_workspace_settings_doc(store, &cfg).await
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

fn normalize_provider_preference_key(value: &str) -> Option<String> {
    trimmed_nonempty(value)
}

fn deserialize_optional_string_map<'de, D>(
    deserializer: D,
) -> Result<Option<HashMap<String, String>>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = Option::<HashMap<String, serde_json::Value>>::deserialize(deserializer)?;
    Ok(raw.map(|entries| {
        entries
            .into_iter()
            .filter_map(|(key, value)| value.as_str().map(|model_id| (key, model_id.to_string())))
            .collect()
    }))
}

fn trimmed_nonempty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
type WorkspaceSettingsLoadedSignal = (
    tokio::sync::oneshot::Sender<()>,
    tokio::sync::oneshot::Receiver<()>,
);

#[cfg(test)]
type WorkspaceSettingsTestPauseHook =
    AsyncMutex<HashMap<&'static str, WorkspaceSettingsLoadedSignal>>;

#[cfg(test)]
fn workspace_settings_test_pause_hook() -> &'static WorkspaceSettingsTestPauseHook {
    static HOOK: OnceLock<WorkspaceSettingsTestPauseHook> = OnceLock::new();
    HOOK.get_or_init(|| AsyncMutex::new(HashMap::new()))
}

#[cfg(test)]
async fn pause_after_workspace_settings_load_for_tests(operation: &'static str) {
    let hook = workspace_settings_test_pause_hook()
        .lock()
        .await
        .remove(operation);
    if let Some((loaded_tx, resume_rx)) = hook {
        let _ = loaded_tx.send(());
        let _ = resume_rx.await;
    }
}

#[cfg(test)]
mod tests;
