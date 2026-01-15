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
    pub resource_governance: Option<ResourceGovernanceSettings>,
    #[serde(default)]
    pub provider_guard: Option<ProviderGuardSettings>,
    #[serde(default)]
    pub subagents: Option<SubagentSettings>,
    #[serde(default)]
    pub sandboxing: Option<SandboxingSettings>,
    #[serde(default)]
    pub github: Option<GithubSettings>,
    #[serde(default)]
    pub cloud_workers: Option<CloudWorkersSettings>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TitleGenerationSettings {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    #[serde(default)]
    pub use_json: bool,
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GithubSettings {
    #[serde(default)]
    pub token: Option<String>,
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
    #[serde(default)]
    pub gateway_token: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicCloudGatewaySettings {
    pub provider: String,
    pub gateway_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_ip: Option<String>,
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
    pub artifact_storage_account: Option<String>,
    #[serde(default)]
    pub artifact_container: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicAwsCloudWorkersSettings {
    pub access_key_id: String,
    pub secret_access_key_set: bool,
    pub region: String,
    pub gateway_instance_type: String,
    pub worker_instance_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subnet_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security_group_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssh_key_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worker_ami_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gateway_ami_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssh_user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_bucket: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicGcpCloudWorkersSettings {
    pub project_id: String,
    pub zone: String,
    pub machine_type: String,
    pub image: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subnetwork: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_account: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scopes: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disk_size_gb: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disk_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssh_user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delete_disk_on_pause: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_bucket: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicAzureCloudWorkersSettings {
    pub subscription_id: String,
    pub resource_group: String,
    pub location: String,
    pub vm_size: String,
    pub image: String,
    pub vnet: String,
    pub subnet: String,
    pub admin_username: String,
    pub ssh_public_key: String,
    pub disk_size_gb: i32,
    pub disk_sku: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_storage_account: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_container: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicCloudWorkersSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gateway: Option<PublicCloudGatewaySettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aws: Option<PublicAwsCloudWorkersSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gcp: Option<PublicGcpCloudWorkersSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub azure: Option<PublicAzureCloudWorkersSettings>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubagentSettings {
    #[serde(default)]
    pub max_per_call: Option<u32>,
}

impl Default for ResourceGovernanceSettings {
    fn default() -> Self {
        Self {
            enabled: true,
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
            enabled: true,
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
    pub github: Option<PublicGithubSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cloud_workers: Option<PublicCloudWorkersSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_governance: Option<PublicResourceGovernanceSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_guard: Option<PublicProviderGuardSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subagents: Option<PublicSubagentSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sandboxing: Option<PublicSandboxingSettings>,
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
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub use_json: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicGithubSettings {
    pub token_set: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicSandboxingSettings {
    pub provider_control_mode: ProviderControlMode,
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
    pub github: Option<UpdateGithubSettingsReq>,
    #[serde(default)]
    pub cloud_workers: Option<UpdateCloudWorkersSettingsReq>,
    #[serde(default)]
    pub resource_governance: Option<UpdateResourceGovernanceSettingsReq>,
    #[serde(default)]
    pub provider_guard: Option<UpdateProviderGuardSettingsReq>,
    #[serde(default)]
    pub subagents: Option<UpdateSubagentSettingsReq>,
    #[serde(default)]
    pub sandboxing: Option<UpdateSandboxingSettingsReq>,
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
pub struct UpdateTitleGenerationSettingsReq {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    #[serde(default)]
    pub use_json: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateGithubSettingsReq {
    pub token: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateCloudWorkersSettingsReq {
    #[serde(default)]
    pub aws: Option<UpdateAwsCloudWorkersSettingsReq>,
    #[serde(default)]
    pub gcp: Option<UpdateGcpCloudWorkersSettingsReq>,
    #[serde(default)]
    pub azure: Option<UpdateAzureCloudWorkersSettingsReq>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateAwsCloudWorkersSettingsReq {
    #[serde(default)]
    pub access_key_id: Option<String>,
    #[serde(default)]
    pub secret_access_key: Option<String>,
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub gateway_instance_type: Option<String>,
    #[serde(default)]
    pub worker_instance_type: Option<String>,
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

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateGcpCloudWorkersSettingsReq {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub zone: Option<String>,
    #[serde(default)]
    pub machine_type: Option<String>,
    #[serde(default)]
    pub image: Option<String>,
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

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateAzureCloudWorkersSettingsReq {
    #[serde(default)]
    pub subscription_id: Option<String>,
    #[serde(default)]
    pub resource_group: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub vm_size: Option<String>,
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub vnet: Option<String>,
    #[serde(default)]
    pub subnet: Option<String>,
    #[serde(default)]
    pub admin_username: Option<String>,
    #[serde(default)]
    pub ssh_public_key: Option<String>,
    #[serde(default)]
    pub disk_size_gb: Option<i32>,
    #[serde(default)]
    pub disk_sku: Option<String>,
    #[serde(default)]
    pub artifact_storage_account: Option<String>,
    #[serde(default)]
    pub artifact_container: Option<String>,
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
pub struct UpdateSubagentSettingsReq {
    #[serde(default)]
    pub max_per_call: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateSandboxingSettingsReq {
    pub provider_control_mode: ProviderControlMode,
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
    if settings.sandboxing.is_none() {
        settings.sandboxing = Some(SandboxingSettings::default());
    }

    // Environment overrides (optional) for easy local bring-up.
    // These are intentionally "best-effort" and do not persist.
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
                base_url: t.base_url.clone(),
                api_key: t.api_key.clone(),
                model: t.model.clone(),
                use_json: t.use_json,
            });
    let github = settings.github.as_ref().map(|g| PublicGithubSettings {
        token_set: g
            .token
            .as_ref()
            .is_some_and(|token| !token.trim().is_empty()),
    });
    let cloud_workers = settings
        .cloud_workers
        .as_ref()
        .map(|cw| PublicCloudWorkersSettings {
            gateway: cw
                .gateway
                .as_ref()
                .map(|gateway| PublicCloudGatewaySettings {
                    provider: gateway.provider.clone(),
                    gateway_url: gateway.gateway_url.clone(),
                    instance_id: gateway.instance_id.clone(),
                    region: gateway.region.clone(),
                    public_ip: gateway.public_ip.clone(),
                }),
            aws: cw.aws.as_ref().map(|aws| PublicAwsCloudWorkersSettings {
                access_key_id: aws.access_key_id.clone(),
                secret_access_key_set: !aws.secret_access_key.trim().is_empty(),
                region: aws.region.clone(),
                gateway_instance_type: aws.gateway_instance_type.clone(),
                worker_instance_type: aws.worker_instance_type.clone(),
                subnet_id: aws.subnet_id.clone(),
                security_group_id: aws.security_group_id.clone(),
                ssh_key_name: aws.ssh_key_name.clone(),
                worker_ami_id: aws.worker_ami_id.clone(),
                gateway_ami_id: aws.gateway_ami_id.clone(),
                ssh_user: aws.ssh_user.clone(),
                artifact_bucket: aws.artifact_bucket.clone(),
            }),
            gcp: cw.gcp.as_ref().map(|gcp| PublicGcpCloudWorkersSettings {
                project_id: gcp.project_id.clone(),
                zone: gcp.zone.clone(),
                machine_type: gcp.machine_type.clone(),
                image: gcp.image.clone(),
                network: gcp.network.clone(),
                subnetwork: gcp.subnetwork.clone(),
                service_account: gcp.service_account.clone(),
                scopes: gcp.scopes.clone(),
                disk_size_gb: gcp.disk_size_gb,
                disk_type: gcp.disk_type.clone(),
                ssh_user: gcp.ssh_user.clone(),
                delete_disk_on_pause: gcp.delete_disk_on_pause,
                artifact_bucket: gcp.artifact_bucket.clone(),
            }),
            azure: cw
                .azure
                .as_ref()
                .map(|azure| PublicAzureCloudWorkersSettings {
                    subscription_id: azure.subscription_id.clone(),
                    resource_group: azure.resource_group.clone(),
                    location: azure.location.clone(),
                    vm_size: azure.vm_size.clone(),
                    image: azure.image.clone(),
                    vnet: azure.vnet.clone(),
                    subnet: azure.subnet.clone(),
                    admin_username: azure.admin_username.clone(),
                    ssh_public_key: azure.ssh_public_key.clone(),
                    disk_size_gb: azure.disk_size_gb,
                    disk_sku: azure.disk_sku.clone(),
                    artifact_storage_account: azure.artifact_storage_account.clone(),
                    artifact_container: azure.artifact_container.clone(),
                }),
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
    let subagents = settings.subagents.as_ref().map(|s| PublicSubagentSettings {
        max_per_call: s.max_per_call,
    });
    let sandboxing = settings
        .sandboxing
        .as_ref()
        .map(|s| PublicSandboxingSettings {
            provider_control_mode: s.provider_control_mode.clone(),
        });
    PublicSettings {
        dictation,
        telemetry,
        title_generation,
        github,
        cloud_workers,
        resource_governance,
        provider_guard,
        subagents,
        sandboxing,
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
        let mut next = current.title_generation.unwrap_or(TitleGenerationSettings {
            base_url: String::new(),
            api_key: String::new(),
            model: String::new(),
            use_json: false,
        });
        next.base_url = t.base_url;
        next.api_key = t.api_key;
        next.model = t.model;
        next.use_json = t.use_json;
        current.title_generation = Some(next);
    }
    if let Some(g) = req.github {
        let mut next = current.github.unwrap_or_default();
        if let Some(token) = g.token {
            let trimmed = token.trim();
            if trimmed.is_empty() {
                next.token = None;
            } else {
                next.token = Some(trimmed.to_string());
            }
        }
        current.github = Some(next);
    }
    if let Some(cw) = req.cloud_workers {
        let mut next = current.cloud_workers.unwrap_or_default();
        if let Some(aws_req) = cw.aws {
            let mut aws = next.aws.unwrap_or_default();
            let mut set_string = |target: &mut String, value: Option<String>| {
                if let Some(value) = value {
                    let trimmed = value.trim();
                    if trimmed.is_empty() {
                        target.clear();
                    } else {
                        *target = trimmed.to_string();
                    }
                }
            };
            let mut set_optional = |target: &mut Option<String>, value: Option<String>| {
                if let Some(value) = value {
                    let trimmed = value.trim();
                    if trimmed.is_empty() {
                        *target = None;
                    } else {
                        *target = Some(trimmed.to_string());
                    }
                }
            };
            set_string(&mut aws.access_key_id, aws_req.access_key_id);
            set_string(&mut aws.secret_access_key, aws_req.secret_access_key);
            set_string(&mut aws.region, aws_req.region);
            set_string(
                &mut aws.gateway_instance_type,
                aws_req.gateway_instance_type,
            );
            set_string(&mut aws.worker_instance_type, aws_req.worker_instance_type);
            set_optional(&mut aws.subnet_id, aws_req.subnet_id);
            set_optional(&mut aws.security_group_id, aws_req.security_group_id);
            set_optional(&mut aws.ssh_key_name, aws_req.ssh_key_name);
            set_optional(&mut aws.worker_ami_id, aws_req.worker_ami_id);
            set_optional(&mut aws.gateway_ami_id, aws_req.gateway_ami_id);
            set_optional(&mut aws.ssh_user, aws_req.ssh_user);
            set_optional(&mut aws.artifact_bucket, aws_req.artifact_bucket);
            next.aws = Some(aws);
        }
        if let Some(gcp_req) = cw.gcp {
            let mut gcp = next.gcp.unwrap_or_default();
            let mut set_string = |target: &mut String, value: Option<String>| {
                if let Some(value) = value {
                    let trimmed = value.trim();
                    if trimmed.is_empty() {
                        target.clear();
                    } else {
                        *target = trimmed.to_string();
                    }
                }
            };
            let mut set_optional = |target: &mut Option<String>, value: Option<String>| {
                if let Some(value) = value {
                    let trimmed = value.trim();
                    if trimmed.is_empty() {
                        *target = None;
                    } else {
                        *target = Some(trimmed.to_string());
                    }
                }
            };
            set_string(&mut gcp.project_id, gcp_req.project_id);
            set_string(&mut gcp.zone, gcp_req.zone);
            set_string(&mut gcp.machine_type, gcp_req.machine_type);
            set_string(&mut gcp.image, gcp_req.image);
            set_optional(&mut gcp.network, gcp_req.network);
            set_optional(&mut gcp.subnetwork, gcp_req.subnetwork);
            set_optional(&mut gcp.service_account, gcp_req.service_account);
            if let Some(scopes) = gcp_req.scopes {
                let scopes = scopes
                    .into_iter()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
                    .collect::<Vec<_>>();
                gcp.scopes = if scopes.is_empty() {
                    None
                } else {
                    Some(scopes)
                };
            }
            if let Some(size) = gcp_req.disk_size_gb {
                gcp.disk_size_gb = if size > 0 { Some(size) } else { None };
            }
            set_optional(&mut gcp.disk_type, gcp_req.disk_type);
            set_optional(&mut gcp.ssh_user, gcp_req.ssh_user);
            if let Some(delete) = gcp_req.delete_disk_on_pause {
                gcp.delete_disk_on_pause = Some(delete);
            }
            set_optional(&mut gcp.artifact_bucket, gcp_req.artifact_bucket);
            next.gcp = Some(gcp);
        }
        if let Some(azure_req) = cw.azure {
            let mut azure = next.azure.unwrap_or_default();
            let mut set_string = |target: &mut String, value: Option<String>| {
                if let Some(value) = value {
                    let trimmed = value.trim();
                    if trimmed.is_empty() {
                        target.clear();
                    } else {
                        *target = trimmed.to_string();
                    }
                }
            };
            let mut set_optional = |target: &mut Option<String>, value: Option<String>| {
                if let Some(value) = value {
                    let trimmed = value.trim();
                    if trimmed.is_empty() {
                        *target = None;
                    } else {
                        *target = Some(trimmed.to_string());
                    }
                }
            };
            set_string(&mut azure.subscription_id, azure_req.subscription_id);
            set_string(&mut azure.resource_group, azure_req.resource_group);
            set_string(&mut azure.location, azure_req.location);
            set_string(&mut azure.vm_size, azure_req.vm_size);
            set_string(&mut azure.image, azure_req.image);
            set_string(&mut azure.vnet, azure_req.vnet);
            set_string(&mut azure.subnet, azure_req.subnet);
            set_string(&mut azure.admin_username, azure_req.admin_username);
            set_string(&mut azure.ssh_public_key, azure_req.ssh_public_key);
            if let Some(size) = azure_req.disk_size_gb {
                azure.disk_size_gb = if size > 0 { size } else { 0 };
            }
            set_string(&mut azure.disk_sku, azure_req.disk_sku);
            set_optional(
                &mut azure.artifact_storage_account,
                azure_req.artifact_storage_account,
            );
            set_optional(&mut azure.artifact_container, azure_req.artifact_container);
            next.azure = Some(azure);
        }
        current.cloud_workers = Some(next);
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
    current
}
