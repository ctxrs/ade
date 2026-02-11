use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const SETTINGS_FILE_NAME: &str = "settings.json";

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

#[derive(Debug, Clone, Serialize)]
pub struct TitleGenerationSettings {
    pub mode: TitleGenerationMode,
    pub remote: TitleGenerationRemoteSettings,
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

#[derive(Debug, Clone, Deserialize)]
struct TitleGenerationSettingsLegacy {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    #[serde(default)]
    pub use_json: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct TitleGenerationSettingsSplit {
    #[serde(default)]
    pub mode: TitleGenerationMode,
    #[serde(default)]
    pub remote: TitleGenerationRemoteSettings,
    #[serde(default)]
    pub local: TitleGenerationLocalSettings,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum TitleGenerationSettingsWire {
    Legacy(TitleGenerationSettingsLegacy),
    Split(TitleGenerationSettingsSplit),
}

impl<'de> Deserialize<'de> for TitleGenerationSettings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = TitleGenerationSettingsWire::deserialize(deserializer)?;
        Ok(match wire {
            TitleGenerationSettingsWire::Legacy(legacy) => TitleGenerationSettings {
                mode: TitleGenerationMode::Remote,
                remote: TitleGenerationRemoteSettings {
                    base_url: legacy.base_url,
                    api_key: legacy.api_key,
                    model: legacy.model,
                    use_json: legacy.use_json,
                },
                local: TitleGenerationLocalSettings::default(),
            },
            TitleGenerationSettingsWire::Split(split) => TitleGenerationSettings {
                mode: split.mode,
                remote: split.remote,
                local: split.local,
            },
        })
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
    #[serde(alias = "auto")]
    Host,
    Container,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ContainerRuntimeKind {
    #[default]
    Podman,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ContainerMountMode {
    #[default]
    HostMounted,
    #[serde(alias = "sealed")]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerExecutionSettings {
    pub runtime: ContainerRuntimeKind,
    pub mount_mode: ContainerMountMode,
    pub network_mode: ContainerNetworkMode,
    #[serde(default)]
    pub allowlist: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
}

impl Default for ContainerExecutionSettings {
    fn default() -> Self {
        Self {
            runtime: ContainerRuntimeKind::Podman,
            mount_mode: ContainerMountMode::HostMounted,
            network_mode: ContainerNetworkMode::LlmOnly,
            allowlist: Vec::new(),
            image: None,
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
mod tests {
    use super::*;

    #[test]
    fn network_profiles_defaults_are_safe_for_system_tasks() {
        let settings = NetworkProfilesSettings::default();
        assert_eq!(settings.agent_default.mode, ContainerNetworkMode::LlmOnly);
        assert_eq!(settings.merge_queue.mode, ContainerNetworkMode::All);
        assert_eq!(settings.worktree_setup.mode, ContainerNetworkMode::All);
        assert_eq!(settings.user_shell.mode, ContainerNetworkMode::All);
        assert!(settings.merge_queue.allowlist.is_empty());
        assert!(settings.worktree_setup.allowlist.is_empty());
    }
}

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

#[derive(Debug, Clone, Serialize)]
pub struct PublicSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dictation: Option<PublicDictationSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telemetry: Option<PublicTelemetrySettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title_generation: Option<PublicTitleGenerationSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oracle: Option<PublicOracleSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_governance: Option<PublicResourceGovernanceSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_guard: Option<PublicProviderGuardSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_limits: Option<PublicToolLimitsSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_restart: Option<PublicProviderRestartSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subagents: Option<PublicSubagentSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sandboxing: Option<PublicSandboxingSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution: Option<PublicExecutionSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network_profiles: Option<PublicNetworkProfilesSettings>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicDictationSettings {
    pub enabled: bool,
    pub provider: DictationProvider,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub livekit: Option<PublicLiveKitDictationSettings>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicLiveKitDictationSettings {
    pub base_url: String,
    pub api_key: String,
    pub api_secret_set: bool,
    pub model: String,
    pub language: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicTelemetrySettings {
    pub enabled: bool,
    pub endpoint: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicTitleGenerationSettings {
    pub mode: TitleGenerationMode,
    pub remote: TitleGenerationRemoteSettings,
    pub local: TitleGenerationLocalSettings,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicOracleSettings {
    pub enabled: bool,
    pub base_url: String,
    pub api_key_set: bool,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicSandboxingSettings {
    pub provider_control_mode: ProviderControlMode,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicExecutionSettings {
    pub mode: ExecutionMode,
    pub container: PublicContainerExecutionSettings,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicNetworkProfilesSettings {
    pub agent_default: NetworkProfile,
    pub merge_queue: NetworkProfile,
    pub worktree_setup: NetworkProfile,
    pub user_shell: NetworkProfile,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicContainerExecutionSettings {
    pub runtime: ContainerRuntimeKind,
    pub mount_mode: ContainerMountMode,
    pub network_mode: ContainerNetworkMode,
    pub allowlist: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResourceGovernanceStatusState {
    Disabled,
    Applied,
    Pending,
    Unsupported,
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicResourceGovernanceStatus {
    pub state: ResourceGovernanceStatusState,
    pub can_apply_now: bool,
    pub requires_restart: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicResourceGovernanceLimits {
    pub cpu_quota_pct: u32,
    pub memory_high_mb: u32,
    pub memory_max_mb: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicResourceGovernanceSettings {
    pub enabled: bool,
    pub mode: ResourceGovernanceMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_quota_pct: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_high_mb: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_max_mb: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective: Option<PublicResourceGovernanceLimits>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<PublicResourceGovernanceStatus>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicProviderGuardSettings {
    pub enabled: bool,
    pub mode: ResourceGovernanceMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_high_mb: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_max_mb: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interval_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grace_period_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicToolLimitsLimits {
    pub memory_high_mb: u32,
    pub memory_max_mb: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicToolLimitsSettings {
    pub enabled: bool,
    pub mode: ResourceGovernanceMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_high_mb: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_max_mb: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective: Option<PublicToolLimitsLimits>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicProviderRestartSettings {
    pub enabled: bool,
    pub mode: ResourceGovernanceMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_high_mb: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_max_mb: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interval_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grace_period_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicSubagentSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_per_call: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateSettingsReq {
    #[serde(default)]
    pub dictation: Option<UpdateDictationSettingsReq>,
    #[serde(default)]
    pub telemetry: Option<UpdateTelemetrySettingsReq>,
    #[serde(default)]
    pub title_generation: Option<UpdateTitleGenerationSettingsReq>,
    #[serde(default)]
    pub oracle: Option<UpdateOracleSettingsReq>,
    #[serde(default)]
    pub resource_governance: Option<UpdateResourceGovernanceSettingsReq>,
    #[serde(default)]
    pub provider_guard: Option<UpdateProviderGuardSettingsReq>,
    #[serde(default)]
    pub tool_limits: Option<UpdateToolLimitsSettingsReq>,
    #[serde(default)]
    pub provider_restart: Option<UpdateProviderRestartSettingsReq>,
    #[serde(default)]
    pub subagents: Option<UpdateSubagentSettingsReq>,
    #[serde(default)]
    pub sandboxing: Option<UpdateSandboxingSettingsReq>,
    #[serde(default)]
    pub execution: Option<UpdateExecutionSettingsReq>,
    #[serde(default)]
    pub network_profiles: Option<NetworkProfilesSettings>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateDictationSettingsReq {
    pub enabled: bool,
    pub provider: DictationProvider,
    #[serde(default)]
    pub livekit: Option<UpdateLiveKitDictationSettingsReq>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateLiveKitDictationSettingsReq {
    pub base_url: String,
    pub api_key: String,
    pub api_secret: Option<String>,
    pub model: String,
    pub language: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateTelemetrySettingsReq {
    pub enabled: bool,
    pub endpoint: String,
}

#[derive(Debug, Clone, Deserialize)]
struct UpdateTitleGenerationSettingsLegacy {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    #[serde(default)]
    pub use_json: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct UpdateTitleGenerationSettingsSplit {
    #[serde(default)]
    pub mode: TitleGenerationMode,
    #[serde(default)]
    pub remote: TitleGenerationRemoteSettings,
    #[serde(default)]
    pub local: TitleGenerationLocalSettings,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum UpdateTitleGenerationSettingsWire {
    Legacy(UpdateTitleGenerationSettingsLegacy),
    Split(UpdateTitleGenerationSettingsSplit),
}

#[derive(Debug, Clone)]
pub struct UpdateTitleGenerationSettingsReq {
    pub mode: TitleGenerationMode,
    pub remote: TitleGenerationRemoteSettings,
    pub local: TitleGenerationLocalSettings,
}

impl<'de> Deserialize<'de> for UpdateTitleGenerationSettingsReq {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = UpdateTitleGenerationSettingsWire::deserialize(deserializer)?;
        Ok(match wire {
            UpdateTitleGenerationSettingsWire::Legacy(legacy) => UpdateTitleGenerationSettingsReq {
                mode: TitleGenerationMode::Remote,
                remote: TitleGenerationRemoteSettings {
                    base_url: legacy.base_url,
                    api_key: legacy.api_key,
                    model: legacy.model,
                    use_json: legacy.use_json,
                },
                local: TitleGenerationLocalSettings::default(),
            },
            UpdateTitleGenerationSettingsWire::Split(split) => UpdateTitleGenerationSettingsReq {
                mode: split.mode,
                remote: split.remote,
                local: split.local,
            },
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateOracleSettingsReq {
    pub enabled: bool,
    pub base_url: String,
    #[serde(default)]
    pub api_key: Option<String>,
    pub model: String,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateResourceGovernanceSettingsReq {
    pub enabled: bool,
    pub mode: ResourceGovernanceMode,
    #[serde(default)]
    pub cpu_quota_pct: Option<u32>,
    #[serde(default)]
    pub memory_high_mb: Option<u32>,
    #[serde(default)]
    pub memory_max_mb: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateProviderGuardSettingsReq {
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

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateToolLimitsSettingsReq {
    pub enabled: bool,
    pub mode: ResourceGovernanceMode,
    #[serde(default)]
    pub memory_high_mb: Option<u32>,
    #[serde(default)]
    pub memory_max_mb: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateProviderRestartSettingsReq {
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

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateSubagentSettingsReq {
    #[serde(default)]
    pub max_per_call: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateSandboxingSettingsReq {
    pub provider_control_mode: ProviderControlMode,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateExecutionSettingsReq {
    pub mode: ExecutionMode,
    #[serde(default)]
    pub container: ContainerExecutionSettings,
}

fn settings_path(data_root: &Path) -> PathBuf {
    data_root.join(SETTINGS_FILE_NAME)
}

pub async fn load_settings(data_root: &Path) -> Settings {
    let path = settings_path(data_root);
    let mut settings = match tokio::fs::read_to_string(&path).await {
        Ok(s) => serde_json::from_str::<Settings>(&s).unwrap_or_default(),
        Err(_) => Settings::default(),
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

    let parse_bool = |value: &str| match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    };
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

    settings
}

pub async fn save_settings(data_root: &Path, settings: &Settings) -> anyhow::Result<()> {
    let path = settings_path(data_root);
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(settings)?;
    tokio::fs::write(&tmp, bytes).await?;
    tokio::fs::rename(&tmp, &path).await?;
    Ok(())
}

pub fn to_public(settings: &Settings) -> PublicSettings {
    let dictation = settings
        .dictation
        .as_ref()
        .map(|d| PublicDictationSettings {
            enabled: d.enabled,
            provider: d.provider.clone(),
            livekit: d.livekit.as_ref().map(|lk| PublicLiveKitDictationSettings {
                base_url: lk.base_url.clone(),
                api_key: lk.api_key.clone(),
                api_secret_set: lk.api_secret.as_ref().is_some_and(|s| !s.trim().is_empty()),
                model: lk.model.clone(),
                language: lk.language.clone(),
            }),
        });
    let telemetry = Some(match settings.telemetry.as_ref() {
        Some(t) => PublicTelemetrySettings {
            enabled: t.enabled,
            endpoint: t.endpoint.clone(),
        },
        None => PublicTelemetrySettings {
            enabled: true,
            endpoint: crate::telemetry::default_telemetry_endpoint(),
        },
    });
    let title_generation =
        settings
            .title_generation
            .as_ref()
            .map(|t| PublicTitleGenerationSettings {
                mode: t.mode.clone(),
                remote: t.remote.clone(),
                local: t.local.clone(),
            });
    let oracle = settings.oracle.as_ref().map(|o| PublicOracleSettings {
        enabled: o.enabled,
        base_url: o.base_url.clone(),
        api_key_set: !o.api_key.trim().is_empty(),
        model: o.model.clone(),
        reasoning_effort: o.reasoning_effort.clone(),
        max_output_tokens: o.max_output_tokens,
        timeout_ms: o.timeout_ms,
    });
    let resource_governance =
        settings
            .resource_governance
            .as_ref()
            .map(|r| PublicResourceGovernanceSettings {
                enabled: r.enabled,
                mode: r.mode.clone(),
                cpu_quota_pct: r.cpu_quota_pct,
                memory_high_mb: r.memory_high_mb,
                memory_max_mb: r.memory_max_mb,
                effective: None,
                status: None,
            });
    let provider_guard = settings
        .provider_guard
        .as_ref()
        .map(|g| PublicProviderGuardSettings {
            enabled: g.enabled,
            mode: g.mode.clone(),
            memory_high_mb: g.memory_high_mb,
            memory_max_mb: g.memory_max_mb,
            interval_ms: g.interval_ms,
            grace_period_ms: g.grace_period_ms,
        });
    let tool_limits = settings
        .tool_limits
        .as_ref()
        .map(|t| PublicToolLimitsSettings {
            enabled: t.enabled,
            mode: t.mode.clone(),
            memory_high_mb: t.memory_high_mb,
            memory_max_mb: t.memory_max_mb,
            effective: None,
        });
    let provider_restart =
        settings
            .provider_restart
            .as_ref()
            .map(|p| PublicProviderRestartSettings {
                enabled: p.enabled,
                mode: p.mode.clone(),
                memory_high_mb: p.memory_high_mb,
                memory_max_mb: p.memory_max_mb,
                interval_ms: p.interval_ms,
                grace_period_ms: p.grace_period_ms,
            });
    let subagents = settings.subagents.as_ref().map(|s| PublicSubagentSettings {
        max_per_call: s.max_per_call,
    });
    let sandboxing = settings
        .sandboxing
        .as_ref()
        .map(|s| PublicSandboxingSettings {
            provider_control_mode: s.provider_control_mode.clone(),
        });
    let execution = settings
        .execution
        .as_ref()
        .map(|e| PublicExecutionSettings {
            mode: e.mode.clone(),
            container: PublicContainerExecutionSettings {
                runtime: e.container.runtime.clone(),
                mount_mode: e.container.mount_mode.clone(),
                network_mode: e.container.network_mode.clone(),
                allowlist: e.container.allowlist.clone(),
                image: e.container.image.clone(),
            },
        });
    let network_profiles =
        settings
            .network_profiles
            .as_ref()
            .map(|p| PublicNetworkProfilesSettings {
                agent_default: p.agent_default.clone(),
                merge_queue: p.merge_queue.clone(),
                worktree_setup: p.worktree_setup.clone(),
                user_shell: p.user_shell.clone(),
            });
    PublicSettings {
        dictation,
        telemetry,
        title_generation,
        oracle,
        resource_governance,
        provider_guard,
        tool_limits,
        provider_restart,
        subagents,
        sandboxing,
        execution,
        network_profiles,
    }
}

pub fn apply_update(mut current: Settings, req: UpdateSettingsReq) -> Settings {
    if let Some(d) = req.dictation {
        let mut next = current.dictation.unwrap_or_default();
        next.enabled = d.enabled;
        next.provider = d.provider;
        if let Some(lk) = d.livekit {
            let mut cur_lk = next.livekit.unwrap_or_default();
            cur_lk.base_url = lk.base_url;
            cur_lk.api_key = lk.api_key;
            if let Some(secret) = lk.api_secret {
                if !secret.trim().is_empty() {
                    cur_lk.api_secret = Some(secret);
                }
            }
            cur_lk.model = lk.model;
            cur_lk.language = lk.language;
            next.livekit = Some(cur_lk);
        }
        current.dictation = Some(next);
    }
    if let Some(t) = req.telemetry {
        let mut next = current.telemetry.unwrap_or_default();
        next.enabled = t.enabled;
        if !t.endpoint.trim().is_empty() {
            next.endpoint = t.endpoint;
        }
        current.telemetry = Some(next);
    }
    if let Some(t) = req.title_generation {
        let mut next = current.title_generation.unwrap_or_default();
        next.mode = t.mode;
        next.remote = t.remote;
        next.local = t.local;
        current.title_generation = Some(next);
    }
    if let Some(o) = req.oracle {
        let mut next = current.oracle.unwrap_or_default();
        next.enabled = o.enabled;
        next.base_url = o.base_url;
        if let Some(api_key) = o.api_key {
            next.api_key = api_key;
        }
        next.model = o.model;
        next.reasoning_effort = o.reasoning_effort;
        next.max_output_tokens = o.max_output_tokens;
        next.timeout_ms = o.timeout_ms;
        current.oracle = Some(next);
    }
    if let Some(r) = req.resource_governance {
        let mut next = current.resource_governance.unwrap_or_default();
        next.enabled = r.enabled;
        next.mode = r.mode;
        next.cpu_quota_pct = r.cpu_quota_pct;
        next.memory_high_mb = r.memory_high_mb;
        next.memory_max_mb = r.memory_max_mb;
        current.resource_governance = Some(next);
    }
    if let Some(g) = req.provider_guard {
        let mut next = current.provider_guard.unwrap_or_default();
        next.enabled = g.enabled;
        next.mode = g.mode;
        next.memory_high_mb = g.memory_high_mb;
        next.memory_max_mb = g.memory_max_mb;
        next.interval_ms = g.interval_ms;
        next.grace_period_ms = g.grace_period_ms;
        current.provider_guard = Some(next);
    }
    if let Some(t) = req.tool_limits {
        let mut next = current.tool_limits.unwrap_or_default();
        next.enabled = t.enabled;
        next.mode = t.mode;
        next.memory_high_mb = t.memory_high_mb;
        next.memory_max_mb = t.memory_max_mb;
        current.tool_limits = Some(next);
    }
    if let Some(r) = req.provider_restart {
        let mut next = current.provider_restart.unwrap_or_default();
        next.enabled = r.enabled;
        next.mode = r.mode;
        next.memory_high_mb = r.memory_high_mb;
        next.memory_max_mb = r.memory_max_mb;
        next.interval_ms = r.interval_ms;
        next.grace_period_ms = r.grace_period_ms;
        current.provider_restart = Some(next);
    }
    if let Some(s) = req.subagents {
        let mut next = current.subagents.unwrap_or_default();
        next.max_per_call = s.max_per_call;
        current.subagents = Some(next);
    }
    if let Some(s) = req.sandboxing {
        let mut next = current.sandboxing.unwrap_or_default();
        next.provider_control_mode = s.provider_control_mode;
        current.sandboxing = Some(next);
    }
    if let Some(e) = req.execution {
        current.execution = Some(ExecutionSettings {
            mode: e.mode,
            container: e.container,
        });
    }
    if let Some(p) = req.network_profiles {
        current.network_profiles = Some(p);
    }
    current
}
