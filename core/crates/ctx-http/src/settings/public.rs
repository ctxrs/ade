use serde::Serialize;

use super::{
    ContainerMachineSettings, ContainerMountMode, ContainerNetworkMode, ContainerRuntimeKind,
    DictationProvider, ExecutionMode, NetworkProfile, ProviderControlMode, ResourceGovernanceMode,
    Settings, TitleGenerationLocalSettings, TitleGenerationMode,
};

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
    pub api_key_set: bool,
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
    pub remote: PublicTitleGenerationRemoteSettings,
    pub local: TitleGenerationLocalSettings,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicTitleGenerationRemoteSettings {
    pub base_url: String,
    pub api_key_set: bool,
    pub model: String,
    pub use_json: bool,
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
    pub machine: ContainerMachineSettings,
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

pub(super) fn to_public(settings: &Settings) -> PublicSettings {
    let dictation = settings
        .dictation
        .as_ref()
        .map(|d| PublicDictationSettings {
            enabled: d.enabled,
            provider: d.provider.clone(),
            livekit: d.livekit.as_ref().map(|lk| PublicLiveKitDictationSettings {
                base_url: lk.base_url.clone(),
                api_key_set: !lk.api_key.trim().is_empty(),
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
                remote: PublicTitleGenerationRemoteSettings {
                    base_url: t.remote.base_url.clone(),
                    api_key_set: !t.remote.api_key.trim().is_empty(),
                    model: t.remote.model.clone(),
                    use_json: t.remote.use_json,
                },
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
                machine: e.container.machine.clone(),
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
