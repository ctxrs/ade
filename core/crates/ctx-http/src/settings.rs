use anyhow::Context;
use ctx_store::Store;
use serde::{Deserialize, Serialize};

mod defaults;
mod overrides;
mod public;
pub(crate) mod update;
pub use public::*;
pub(crate) use update::UpdateSettingsReq;

const SETTINGS_SCHEMA_VERSION: i64 = 1;
const RUNTIME_SETTINGS_SECRET_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RuntimeSettingsSecretEnvelope {
    version: u32,
    dictation_livekit_api_key: String,
    #[serde(default)]
    dictation_livekit_api_secret: Option<String>,
    title_generation_remote_api_key: String,
    oracle_api_key: String,
    aws_cloud_workers_access_key_id: String,
    aws_cloud_workers_secret_access_key: String,
}

pub(crate) fn to_public(settings: &Settings) -> PublicSettings {
    public::to_public(settings)
}

pub(crate) fn apply_update(current: Settings, req: UpdateSettingsReq) -> Settings {
    let mut next = update::apply_update(current, req);
    normalize_settings_in_place(&mut next);
    next
}

pub use ctx_sandbox_contract::{
    default_container_machine_host_pressure_swap_threshold_mb,
    default_container_machine_idle_shutdown_seconds, default_container_mount_mode_for_runtime,
    default_container_runtime_kind, normalize_container_execution_settings,
    normalize_container_machine_idle_shutdown_seconds, normalize_container_machine_settings,
    ContainerExecutionSettings, ContainerMachineMemoryProfile, ContainerMachineSettings,
    ContainerMountMode, ContainerNetworkMode, ContainerRuntimeKind, ExecutionMode,
    ExecutionSettings, MIN_CONTAINER_MACHINE_IDLE_SHUTDOWN_SECONDS,
};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Settings {
    #[serde(default)]
    pub dictation: Option<DictationSettings>,
    #[serde(default)]
    pub telemetry: Option<TelemetrySettings>,
    #[serde(default)]
    pub title_generation: Option<TitleGenerationSettings>,
    #[serde(default)]
    pub oracle: Option<OracleSettings>,
    #[serde(default)]
    pub resource_governance: Option<ResourceGovernanceSettings>,
    #[serde(default)]
    pub provider_guard: Option<ProviderGuardSettings>,
    #[serde(default)]
    pub tool_limits: Option<ToolLimitsSettings>,
    #[serde(default)]
    pub provider_restart: Option<ProviderRestartSettings>,
    #[serde(default)]
    pub subagents: Option<SubagentSettings>,
    #[serde(default)]
    pub sandboxing: Option<SandboxingSettings>,
    #[serde(default)]
    pub cloud_workers: Option<CloudWorkersSettings>,
    #[serde(default)]
    pub storage: Option<StorageSettings>,
    #[serde(default)]
    pub execution: Option<ExecutionSettings>,
    #[serde(default)]
    pub network_profiles: Option<NetworkProfilesSettings>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StorageSettings {
    #[serde(default)]
    pub max_connections: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DictationSettings {
    pub enabled: bool,
    pub provider: DictationProvider,
    #[serde(default)]
    pub livekit: Option<LiveKitDictationSettings>,
}

impl Default for DictationSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            provider: DictationProvider::LiveKitInference,
            livekit: Some(LiveKitDictationSettings::default()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DictationProvider {
    Disabled,
    #[serde(rename = "livekit_inference")]
    LiveKitInference,
    #[serde(rename = "tauri_stt")]
    TauriStt,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveKitDictationSettings {
    pub base_url: String,
    pub api_key: String,
    #[serde(default)]
    pub api_secret: Option<String>,
    pub model: String,
    pub language: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TitleGenerationMode {
    #[default]
    Remote,
    Local,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TitleGenerationRemoteSettings {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    #[serde(default)]
    pub use_json: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TitleGenerationLocalSettings {
    pub model_id: String,
    #[serde(default)]
    pub use_json: bool,
}

impl Default for TitleGenerationLocalSettings {
    fn default() -> Self {
        Self {
            model_id: crate::title_generation_local::LOCAL_MODEL_ID.to_string(),
            use_json: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TitleGenerationSettings {
    pub mode: TitleGenerationMode,
    #[serde(default)]
    pub remote: TitleGenerationRemoteSettings,
    #[serde(default)]
    pub local: TitleGenerationLocalSettings,
}

impl Default for TitleGenerationSettings {
    fn default() -> Self {
        Self {
            mode: TitleGenerationMode::Remote,
            remote: TitleGenerationRemoteSettings::default(),
            local: TitleGenerationLocalSettings::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OracleSettings {
    pub enabled: bool,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

impl Default for OracleSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            base_url: "https://api.openai.com/v1".to_string(),
            api_key: String::new(),
            model: "gpt-5.2-pro".to_string(),
            reasoning_effort: Some("high".to_string()),
            max_output_tokens: None,
            timeout_ms: Some(10 * 60 * 1000),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetrySettings {
    pub enabled: bool,
    #[serde(default = "crate::telemetry::default_telemetry_endpoint")]
    pub endpoint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProviderControlMode {
    #[default]
    Full,
    HarnessNative,
    CtxEnforced,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxingSettings {
    pub provider_control_mode: ProviderControlMode,
}

impl Default for SandboxingSettings {
    fn default() -> Self {
        Self {
            provider_control_mode: ProviderControlMode::Full,
        }
    }
}

pub(crate) fn normalize_settings_in_place(settings: &mut Settings) {
    if let Some(sandboxing) = settings.sandboxing.as_mut() {
        // Only `full` is currently a real product setting. The other variants
        // stay in the schema as reserved values for future work, but are not
        // meaningfully supported or user-facing today.
        sandboxing.provider_control_mode = ProviderControlMode::Full;
    }
    if let Some(execution) = settings.execution.as_mut() {
        normalize_container_execution_settings(&mut execution.container);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum NetworkContext {
    AgentDefault,
    MergeQueue,
    WorktreeSetup,
    UserShell,
}

impl NetworkContext {
    pub fn as_str(&self) -> &'static str {
        match self {
            NetworkContext::AgentDefault => "agent_default",
            NetworkContext::MergeQueue => "merge_queue",
            NetworkContext::WorktreeSetup => "worktree_setup",
            NetworkContext::UserShell => "user_shell",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkProfile {
    pub mode: ContainerNetworkMode,
    #[serde(default)]
    pub allowlist: Vec<String>,
}

impl Default for NetworkProfile {
    fn default() -> Self {
        Self {
            mode: ContainerNetworkMode::LlmOnly,
            allowlist: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkProfilesSettings {
    pub agent_default: NetworkProfile,
    pub merge_queue: NetworkProfile,
    pub worktree_setup: NetworkProfile,
    pub user_shell: NetworkProfile,
}

impl Default for NetworkProfilesSettings {
    fn default() -> Self {
        Self {
            agent_default: NetworkProfile::default(),
            merge_queue: NetworkProfile {
                mode: ContainerNetworkMode::All,
                allowlist: Vec::new(),
            },
            worktree_setup: NetworkProfile {
                mode: ContainerNetworkMode::All,
                allowlist: Vec::new(),
            },
            user_shell: NetworkProfile {
                mode: ContainerNetworkMode::All,
                allowlist: Vec::new(),
            },
        }
    }
}

impl NetworkProfilesSettings {
    pub fn profile(&self, context: NetworkContext) -> NetworkProfile {
        match context {
            NetworkContext::AgentDefault => self.agent_default.clone(),
            NetworkContext::MergeQueue => self.merge_queue.clone(),
            NetworkContext::WorktreeSetup => self.worktree_setup.clone(),
            NetworkContext::UserShell => self.user_shell.clone(),
        }
    }
}

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceGovernanceMode {
    Auto,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceGovernanceSettings {
    pub enabled: bool,
    pub mode: ResourceGovernanceMode,
    #[serde(default)]
    pub cpu_quota_pct: Option<u32>,
    #[serde(default)]
    pub memory_high_mb: Option<u32>,
    #[serde(default)]
    pub memory_max_mb: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderGuardSettings {
    pub enabled: bool,
    pub mode: ResourceGovernanceMode,
    #[serde(default)]
    pub memory_high_mb: Option<u32>,
    #[serde(default)]
    pub memory_max_mb: Option<u32>,
    #[serde(default)]
    pub interval_ms: Option<u64>,
    #[serde(default)]
    pub grace_period_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolLimitsSettings {
    pub enabled: bool,
    pub mode: ResourceGovernanceMode,
    #[serde(default)]
    pub memory_high_mb: Option<u32>,
    #[serde(default)]
    pub memory_max_mb: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderRestartSettings {
    pub enabled: bool,
    pub mode: ResourceGovernanceMode,
    #[serde(default)]
    pub memory_high_mb: Option<u32>,
    #[serde(default)]
    pub memory_max_mb: Option<u32>,
    #[serde(default)]
    pub interval_ms: Option<u64>,
    #[serde(default)]
    pub grace_period_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CloudWorkersSettings {
    #[serde(default)]
    pub gateway: Option<CloudGatewaySettings>,
    #[serde(default)]
    pub aws: Option<AwsCloudWorkersSettings>,
    #[serde(default)]
    pub gcp: Option<GcpCloudWorkersSettings>,
    #[serde(default)]
    pub azure: Option<AzureCloudWorkersSettings>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CloudGatewaySettings {
    pub provider: String,
    pub gateway_url: String,
    #[serde(default)]
    pub instance_id: Option<String>,
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub public_ip: Option<String>,
    #[serde(default)]
    pub gateway_ca_pem: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AwsCloudWorkersSettings {
    #[serde(default)]
    pub access_key_id: String,
    #[serde(default)]
    pub secret_access_key: String,
    #[serde(default)]
    pub region: String,
    #[serde(default)]
    pub gateway_instance_type: String,
    #[serde(default)]
    pub worker_instance_type: String,
    #[serde(default)]
    pub subnet_id: Option<String>,
    #[serde(default)]
    pub security_group_id: Option<String>,
    #[serde(default)]
    pub ssh_key_name: Option<String>,
    #[serde(default)]
    pub worker_ami_id: Option<String>,
    #[serde(default)]
    pub gateway_ami_id: Option<String>,
    #[serde(default)]
    pub ssh_user: Option<String>,
    #[serde(default)]
    pub artifact_bucket: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GcpCloudWorkersSettings {
    #[serde(default)]
    pub project_id: String,
    #[serde(default)]
    pub zone: String,
    #[serde(default)]
    pub machine_type: String,
    #[serde(default)]
    pub image: String,
    #[serde(default)]
    pub network: Option<String>,
    #[serde(default)]
    pub subnetwork: Option<String>,
    #[serde(default)]
    pub service_account: Option<String>,
    #[serde(default)]
    pub scopes: Option<Vec<String>>,
    #[serde(default)]
    pub disk_size_gb: Option<i64>,
    #[serde(default)]
    pub disk_type: Option<String>,
    #[serde(default)]
    pub ssh_user: Option<String>,
    #[serde(default)]
    pub delete_disk_on_pause: Option<bool>,
    #[serde(default)]
    pub artifact_bucket: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AzureCloudWorkersSettings {
    #[serde(default)]
    pub subscription_id: String,
    #[serde(default)]
    pub resource_group: String,
    #[serde(default)]
    pub location: String,
    #[serde(default)]
    pub vm_size: String,
    #[serde(default)]
    pub image: String,
    #[serde(default)]
    pub vnet: String,
    #[serde(default)]
    pub subnet: String,
    #[serde(default)]
    pub admin_username: String,
    #[serde(default)]
    pub ssh_public_key: String,
    #[serde(default)]
    pub disk_size_gb: i32,
    #[serde(default)]
    pub disk_sku: String,
    #[serde(default)]
    pub delete_disk_on_pause: bool,
    #[serde(default)]
    pub use_public_ip: bool,
    #[serde(default)]
    pub artifact_storage_account: Option<String>,
    #[serde(default)]
    pub artifact_container: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubagentSettings {
    #[serde(default)]
    pub max_per_call: Option<u32>,
}

impl Default for ResourceGovernanceSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: ResourceGovernanceMode::Auto,
            cpu_quota_pct: None,
            memory_high_mb: None,
            memory_max_mb: None,
        }
    }
}

impl Default for ProviderGuardSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: ResourceGovernanceMode::Auto,
            memory_high_mb: None,
            memory_max_mb: None,
            interval_ms: None,
            grace_period_ms: None,
        }
    }
}

impl Default for ToolLimitsSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: ResourceGovernanceMode::Auto,
            memory_high_mb: None,
            memory_max_mb: None,
        }
    }
}

impl Default for ProviderRestartSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: ResourceGovernanceMode::Auto,
            memory_high_mb: None,
            memory_max_mb: None,
            interval_ms: None,
            grace_period_ms: None,
        }
    }
}

impl Default for TelemetrySettings {
    fn default() -> Self {
        Self {
            enabled: true,
            endpoint: crate::telemetry::default_telemetry_endpoint(),
        }
    }
}

impl Default for LiveKitDictationSettings {
    fn default() -> Self {
        Self {
            base_url: "https://agent-gateway.livekit.cloud/v1".to_string(),
            api_key: String::new(),
            api_secret: None,
            model: "auto".to_string(),
            language: "en".to_string(),
        }
    }
}

fn runtime_settings_secrets_from_settings(settings: &Settings) -> RuntimeSettingsSecretEnvelope {
    RuntimeSettingsSecretEnvelope {
        version: RUNTIME_SETTINGS_SECRET_VERSION,
        dictation_livekit_api_key: settings
            .dictation
            .as_ref()
            .and_then(|dictation| dictation.livekit.as_ref())
            .map(|livekit| livekit.api_key.clone())
            .unwrap_or_default(),
        dictation_livekit_api_secret: settings
            .dictation
            .as_ref()
            .and_then(|dictation| dictation.livekit.as_ref())
            .and_then(|livekit| livekit.api_secret.clone()),
        title_generation_remote_api_key: settings
            .title_generation
            .as_ref()
            .map(|title_generation| title_generation.remote.api_key.clone())
            .unwrap_or_default(),
        oracle_api_key: settings
            .oracle
            .as_ref()
            .map(|oracle| oracle.api_key.clone())
            .unwrap_or_default(),
        aws_cloud_workers_access_key_id: settings
            .cloud_workers
            .as_ref()
            .and_then(|cloud_workers| cloud_workers.aws.as_ref())
            .map(|aws| aws.access_key_id.clone())
            .unwrap_or_default(),
        aws_cloud_workers_secret_access_key: settings
            .cloud_workers
            .as_ref()
            .and_then(|cloud_workers| cloud_workers.aws.as_ref())
            .map(|aws| aws.secret_access_key.clone())
            .unwrap_or_default(),
    }
}

fn apply_runtime_settings_secrets(
    settings: &mut Settings,
    secrets: &RuntimeSettingsSecretEnvelope,
) {
    if let Some(livekit) = settings
        .dictation
        .as_mut()
        .and_then(|dictation| dictation.livekit.as_mut())
    {
        livekit.api_key = secrets.dictation_livekit_api_key.clone();
        livekit.api_secret = secrets.dictation_livekit_api_secret.clone();
    }
    if let Some(title_generation) = settings.title_generation.as_mut() {
        title_generation.remote.api_key = secrets.title_generation_remote_api_key.clone();
    }
    if let Some(oracle) = settings.oracle.as_mut() {
        oracle.api_key = secrets.oracle_api_key.clone();
    }
    if let Some(aws) = settings
        .cloud_workers
        .as_mut()
        .and_then(|cloud_workers| cloud_workers.aws.as_mut())
    {
        aws.access_key_id = secrets.aws_cloud_workers_access_key_id.clone();
        aws.secret_access_key = secrets.aws_cloud_workers_secret_access_key.clone();
    }
}

fn strip_runtime_settings_secrets(settings: &mut Settings) {
    if let Some(livekit) = settings
        .dictation
        .as_mut()
        .and_then(|dictation| dictation.livekit.as_mut())
    {
        livekit.api_key.clear();
        livekit.api_secret = None;
    }
    if let Some(title_generation) = settings.title_generation.as_mut() {
        title_generation.remote.api_key.clear();
    }
    if let Some(oracle) = settings.oracle.as_mut() {
        oracle.api_key.clear();
    }
    if let Some(aws) = settings
        .cloud_workers
        .as_mut()
        .and_then(|cloud_workers| cloud_workers.aws.as_mut())
    {
        aws.access_key_id.clear();
        aws.secret_access_key.clear();
    }
}

fn settings_contain_runtime_secrets(settings: &Settings) -> bool {
    settings
        .dictation
        .as_ref()
        .and_then(|dictation| dictation.livekit.as_ref())
        .is_some_and(|livekit| {
            !livekit.api_key.trim().is_empty()
                || livekit
                    .api_secret
                    .as_deref()
                    .is_some_and(|secret| !secret.trim().is_empty())
        })
        || settings
            .title_generation
            .as_ref()
            .is_some_and(|title_generation| !title_generation.remote.api_key.trim().is_empty())
        || settings
            .oracle
            .as_ref()
            .is_some_and(|oracle| !oracle.api_key.trim().is_empty())
        || settings
            .cloud_workers
            .as_ref()
            .and_then(|cloud_workers| cloud_workers.aws.as_ref())
            .is_some_and(|aws| {
                !aws.access_key_id.trim().is_empty() || !aws.secret_access_key.trim().is_empty()
            })
}

async fn load_runtime_settings_secret_envelope(
    store: &Store,
    secret_ref: &str,
) -> anyhow::Result<RuntimeSettingsSecretEnvelope> {
    let payload = store
        .read_runtime_settings_secrets_if_present(secret_ref)
        .await?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "runtime settings secrets are missing for settings document (secret_ref={secret_ref})"
            )
        })?;
    let envelope = serde_json::from_str::<RuntimeSettingsSecretEnvelope>(&payload)
        .context("parsing runtime settings secret envelope")?;
    if envelope.version != RUNTIME_SETTINGS_SECRET_VERSION {
        anyhow::bail!(
            "unsupported runtime settings secret version {}",
            envelope.version
        );
    }
    Ok(envelope)
}

pub async fn load_settings(store: &Store) -> anyhow::Result<Settings> {
    let mut settings = match store.get_runtime_settings_document().await? {
        Some(doc) => {
            let mut settings = serde_json::from_str::<Settings>(&doc.settings_json)
                .context("parsing runtime settings document")?;
            let legacy_secrets_present = settings_contain_runtime_secrets(&settings);
            match doc.secret_ref.as_deref() {
                Some(secret_ref) => {
                    let secrets = load_runtime_settings_secret_envelope(store, secret_ref).await?;
                    apply_runtime_settings_secrets(&mut settings, &secrets);
                    if legacy_secrets_present {
                        save_settings(store, &settings).await?;
                        store.checkpoint_wal_truncate().await?;
                    }
                }
                None => {
                    if legacy_secrets_present {
                        save_settings(store, &settings).await?;
                        store.checkpoint_wal_truncate().await?;
                    }
                }
            }
            settings
        }
        None => Settings::default(),
    };
    defaults::ensure_settings_defaults(&mut settings);
    normalize_settings_in_place(&mut settings);

    overrides::apply_env_overrides(&mut settings);

    Ok(settings)
}

pub async fn save_settings(store: &Store, settings: &Settings) -> anyhow::Result<()> {
    let mut normalized = settings.clone();
    normalize_settings_in_place(&mut normalized);
    if settings_contain_runtime_secrets(&normalized) {
        let secrets_json =
            serde_json::to_string_pretty(&runtime_settings_secrets_from_settings(&normalized))?;
        strip_runtime_settings_secrets(&mut normalized);
        let settings_json = serde_json::to_string_pretty(&normalized)?;
        store
            .upsert_runtime_settings_document_with_secrets(
                SETTINGS_SCHEMA_VERSION,
                &settings_json,
                &secrets_json,
            )
            .await?;
    } else {
        let settings_json = serde_json::to_string_pretty(&normalized)?;
        store
            .upsert_runtime_settings_document(SETTINGS_SCHEMA_VERSION, &settings_json)
            .await?;
    }
    Ok(())
}
