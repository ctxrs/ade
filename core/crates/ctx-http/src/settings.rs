use anyhow::Context;
use ctx_store::Store;
use serde::{Deserialize, Deserializer, Serialize};

mod public;
pub(crate) mod update;
pub use public::*;
pub(crate) use update::UpdateSettingsReq;

const SETTINGS_SCHEMA_VERSION: i64 = 1;

pub(crate) fn to_public(settings: &Settings) -> PublicSettings {
    public::to_public(settings)
}

pub(crate) fn apply_update(current: Settings, req: UpdateSettingsReq) -> Settings {
    update::apply_update(current, req)
}

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    #[default]
    Host,
    Container,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ContainerRuntimeKind {
    #[default]
    Podman,
    AvfLinuxVm,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ContainerMountMode {
    #[default]
    HostMounted,
    DiskIsolated,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ContainerNetworkMode {
    #[default]
    LlmOnly,
    Allowlist,
    All,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ContainerMachineMemoryProfile {
    #[default]
    Economy,
    Balanced,
    Performance,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContainerMachineSettings {
    #[serde(default)]
    pub memory_profile: ContainerMachineMemoryProfile,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_memory_mb: Option<u32>,
    #[serde(
        default = "default_container_machine_idle_shutdown_seconds",
        deserialize_with = "deserialize_container_machine_idle_shutdown_seconds"
    )]
    pub idle_shutdown_seconds: u64,
    #[serde(default = "default_container_machine_host_pressure_swap_threshold_mb")]
    pub host_pressure_swap_threshold_mb: u32,
}

impl Default for ContainerMachineSettings {
    fn default() -> Self {
        Self {
            memory_profile: ContainerMachineMemoryProfile::Economy,
            custom_memory_mb: None,
            idle_shutdown_seconds: default_container_machine_idle_shutdown_seconds(),
            host_pressure_swap_threshold_mb:
                default_container_machine_host_pressure_swap_threshold_mb(),
        }
    }
}

pub const MIN_CONTAINER_MACHINE_IDLE_SHUTDOWN_SECONDS: u64 = 60;

pub const fn default_container_machine_idle_shutdown_seconds() -> u64 {
    60 * 60
}

fn deserialize_container_machine_idle_shutdown_seconds<'de, D>(
    deserializer: D,
) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    let value = u64::deserialize(deserializer)?;
    Ok(normalize_container_machine_idle_shutdown_seconds(value))
}

pub const fn default_container_machine_host_pressure_swap_threshold_mb() -> u32 {
    1024
}

pub const fn normalize_container_machine_idle_shutdown_seconds(value: u64) -> u64 {
    if value < MIN_CONTAINER_MACHINE_IDLE_SHUTDOWN_SECONDS {
        MIN_CONTAINER_MACHINE_IDLE_SHUTDOWN_SECONDS
    } else {
        value
    }
}

pub fn normalize_container_machine_settings(machine: &mut ContainerMachineSettings) {
    machine.idle_shutdown_seconds =
        normalize_container_machine_idle_shutdown_seconds(machine.idle_shutdown_seconds);
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerExecutionSettings {
    pub runtime: ContainerRuntimeKind,
    pub mount_mode: ContainerMountMode,
    pub network_mode: ContainerNetworkMode,
    #[serde(default)]
    pub allowlist: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(default)]
    pub machine: ContainerMachineSettings,
}

fn default_container_runtime_kind() -> ContainerRuntimeKind {
    #[cfg(target_os = "macos")]
    {
        ContainerRuntimeKind::AvfLinuxVm
    }
    #[cfg(not(target_os = "macos"))]
    {
        ContainerRuntimeKind::Podman
    }
}

impl Default for ContainerExecutionSettings {
    fn default() -> Self {
        Self {
            runtime: default_container_runtime_kind(),
            mount_mode: ContainerMountMode::HostMounted,
            network_mode: ContainerNetworkMode::LlmOnly,
            allowlist: Vec::new(),
            image: None,
            machine: ContainerMachineSettings::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionSettings {
    pub mode: ExecutionMode,
    #[serde(default)]
    pub container: ContainerExecutionSettings,
}

impl Default for ExecutionSettings {
    fn default() -> Self {
        Self {
            mode: ExecutionMode::Host,
            container: ContainerExecutionSettings::default(),
        }
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

pub async fn load_settings(store: &Store) -> anyhow::Result<Settings> {
    let mut settings = match store.get_runtime_settings_document().await? {
        Some(doc) => serde_json::from_str::<Settings>(&doc.settings_json)
            .context("parsing runtime settings document")?,
        None => Settings::default(),
    };
    if settings.resource_governance.is_none() {
        settings.resource_governance = Some(ResourceGovernanceSettings::default());
    }
    if settings.provider_guard.is_none() {
        settings.provider_guard = Some(ProviderGuardSettings::default());
    }
    if settings.tool_limits.is_none() {
        settings.tool_limits = Some(ToolLimitsSettings::default());
    }
    if settings.provider_restart.is_none() {
        settings.provider_restart = Some(ProviderRestartSettings::default());
    }
    if settings.sandboxing.is_none() {
        settings.sandboxing = Some(SandboxingSettings::default());
    }
    if settings.storage.is_none() {
        settings.storage = Some(StorageSettings::default());
    }
    if settings.execution.is_none() {
        settings.execution = Some(ExecutionSettings::default());
    }
    if settings.network_profiles.is_none() {
        settings.network_profiles = Some(NetworkProfilesSettings::default());
    }

    // Environment overrides (optional) for easy local bring-up.
    // These are intentionally "best-effort" and do not persist.
    if let Ok(mode) = std::env::var("CTX_EXECUTION_MODE") {
        let normalized = mode.trim().to_lowercase();
        let mode = match normalized.as_str() {
            "host" => Some(ExecutionMode::Host),
            "container" => Some(ExecutionMode::Container),
            _ => None,
        };
        if let Some(mode) = mode {
            let execution = settings
                .execution
                .get_or_insert_with(ExecutionSettings::default);
            execution.mode = mode;
        }
    }
    if let Ok(v) = std::env::var("CTX_DICTATION_PROVIDER") {
        if v.trim().eq_ignore_ascii_case("disabled") {
            settings.dictation = Some(DictationSettings {
                enabled: false,
                provider: DictationProvider::Disabled,
                livekit: None,
            });
        }
    }

    if let Ok(api_key) = std::env::var("LIVEKIT_API_KEY") {
        let d = settings
            .dictation
            .get_or_insert_with(DictationSettings::default);
        if d.livekit.is_none() {
            d.livekit = Some(LiveKitDictationSettings::default());
        }
        if let Some(lk) = d.livekit.as_mut() {
            if lk.api_key.trim().is_empty() {
                lk.api_key = api_key;
            }
        }
    }
    if let Ok(api_secret) = std::env::var("LIVEKIT_API_SECRET") {
        let d = settings
            .dictation
            .get_or_insert_with(DictationSettings::default);
        if d.livekit.is_none() {
            d.livekit = Some(LiveKitDictationSettings::default());
        }
        if let Some(lk) = d.livekit.as_mut() {
            if lk
                .api_secret
                .as_ref()
                .map(|s| s.trim().is_empty())
                .unwrap_or(true)
            {
                lk.api_secret = Some(api_secret);
            }
        }
    }
    if let Ok(base_url) = std::env::var("CTX_LIVEKIT_INFERENCE_BASE_URL") {
        let d = settings
            .dictation
            .get_or_insert_with(DictationSettings::default);
        if d.livekit.is_none() {
            d.livekit = Some(LiveKitDictationSettings::default());
        }
        if let Some(lk) = d.livekit.as_mut() {
            if lk.base_url.trim() == "https://agent-gateway.livekit.cloud/v1" {
                lk.base_url = base_url;
            }
        }
    }

    if let Ok(api_key) = std::env::var("CTX_ORACLE_API_KEY") {
        let oracle = settings.oracle.get_or_insert_with(OracleSettings::default);
        if oracle.api_key.trim().is_empty() {
            oracle.api_key = api_key;
        }
        if !oracle.api_key.trim().is_empty() {
            oracle.enabled = true;
        }
    }
    if let Ok(base_url) = std::env::var("CTX_ORACLE_BASE_URL") {
        let oracle = settings.oracle.get_or_insert_with(OracleSettings::default);
        if oracle.base_url.trim() == "https://api.openai.com/v1" {
            oracle.base_url = base_url;
        }
    }
    if let Ok(model) = std::env::var("CTX_ORACLE_MODEL") {
        let oracle = settings.oracle.get_or_insert_with(OracleSettings::default);
        if oracle.model.trim() == "gpt-5.2-pro" {
            oracle.model = model;
        }
    }

    let parse_bool = ctx_core::boolish::parse_boolish;
    let parse_mode = |value: &str| match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(ResourceGovernanceMode::Auto),
        "custom" => Some(ResourceGovernanceMode::Custom),
        _ => None,
    };

    if let Ok(value) = std::env::var("CTX_TOOL_LIMITS_ENABLED") {
        if let Some(enabled) = parse_bool(&value) {
            let tool_limits = settings
                .tool_limits
                .get_or_insert_with(ToolLimitsSettings::default);
            tool_limits.enabled = enabled;
        }
    }
    if let Ok(value) = std::env::var("CTX_TOOL_LIMITS_MODE") {
        if let Some(mode) = parse_mode(&value) {
            let tool_limits = settings
                .tool_limits
                .get_or_insert_with(ToolLimitsSettings::default);
            tool_limits.mode = mode;
        }
    }
    if let Ok(value) = std::env::var("CTX_TOOL_LIMITS_MEMORY_HIGH_MB") {
        if let Ok(parsed) = value.trim().parse::<u32>() {
            let tool_limits = settings
                .tool_limits
                .get_or_insert_with(ToolLimitsSettings::default);
            tool_limits.memory_high_mb = Some(parsed);
        }
    }
    if let Ok(value) = std::env::var("CTX_TOOL_LIMITS_MEMORY_MAX_MB") {
        if let Ok(parsed) = value.trim().parse::<u32>() {
            let tool_limits = settings
                .tool_limits
                .get_or_insert_with(ToolLimitsSettings::default);
            tool_limits.memory_max_mb = Some(parsed);
        }
    }

    if let Ok(value) = std::env::var("CTX_PROVIDER_RESTART_ENABLED") {
        if let Some(enabled) = parse_bool(&value) {
            let restart = settings
                .provider_restart
                .get_or_insert_with(ProviderRestartSettings::default);
            restart.enabled = enabled;
        }
    }
    if let Ok(value) = std::env::var("CTX_PROVIDER_RESTART_MODE") {
        if let Some(mode) = parse_mode(&value) {
            let restart = settings
                .provider_restart
                .get_or_insert_with(ProviderRestartSettings::default);
            restart.mode = mode;
        }
    }
    if let Ok(value) = std::env::var("CTX_PROVIDER_RESTART_MEMORY_HIGH_MB") {
        if let Ok(parsed) = value.trim().parse::<u32>() {
            let restart = settings
                .provider_restart
                .get_or_insert_with(ProviderRestartSettings::default);
            restart.memory_high_mb = Some(parsed);
        }
    }
    if let Ok(value) = std::env::var("CTX_PROVIDER_RESTART_MEMORY_MAX_MB") {
        if let Ok(parsed) = value.trim().parse::<u32>() {
            let restart = settings
                .provider_restart
                .get_or_insert_with(ProviderRestartSettings::default);
            restart.memory_max_mb = Some(parsed);
        }
    }
    if let Ok(value) = std::env::var("CTX_PROVIDER_RESTART_INTERVAL_MS") {
        if let Ok(parsed) = value.trim().parse::<u64>() {
            let restart = settings
                .provider_restart
                .get_or_insert_with(ProviderRestartSettings::default);
            restart.interval_ms = Some(parsed);
        }
    }
    if let Ok(value) = std::env::var("CTX_PROVIDER_RESTART_GRACE_PERIOD_MS") {
        if let Ok(parsed) = value.trim().parse::<u64>() {
            let restart = settings
                .provider_restart
                .get_or_insert_with(ProviderRestartSettings::default);
            restart.grace_period_ms = Some(parsed);
        }
    }

    Ok(settings)
}

pub async fn save_settings(store: &Store, settings: &Settings) -> anyhow::Result<()> {
    let settings_json = serde_json::to_string_pretty(settings)?;
    store
        .upsert_runtime_settings_document(SETTINGS_SCHEMA_VERSION, &settings_json)
        .await?;
    Ok(())
}
