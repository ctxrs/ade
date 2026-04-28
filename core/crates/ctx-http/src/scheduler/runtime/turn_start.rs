use std::time::Duration;

use crate::settings::ProviderControlMode;
use ctx_core::provider_ids::CODEX_PROVIDER_ID;
use ctx_core::provider_policy::{CTX_CRP_LAUNCH_POLICY_ENV, CTX_CRP_LAUNCH_POLICY_FULL};

pub(super) fn provider_mode_id_for(
    provider_id: &str,
    control_mode: &ProviderControlMode,
) -> Option<&'static str> {
    match control_mode {
        ProviderControlMode::Full => match provider_id {
            CODEX_PROVIDER_ID => Some("full-access"),
            "claude-crp" => Some("bypassPermissions"),
            "droid" => Some("auto_high"),
            _ => None,
        },
        ProviderControlMode::HarnessNative | ProviderControlMode::CtxEnforced => None,
    }
}

pub(super) fn apply_crp_launch_policy_env_for_control_mode(
    provider_env: &mut std::collections::HashMap<String, String>,
    control_mode: &ProviderControlMode,
) {
    provider_env.remove(CTX_CRP_LAUNCH_POLICY_ENV);
    match control_mode {
        ProviderControlMode::Full => {
            provider_env.insert(
                CTX_CRP_LAUNCH_POLICY_ENV.to_string(),
                CTX_CRP_LAUNCH_POLICY_FULL.to_string(),
            );
        }
        ProviderControlMode::HarnessNative | ProviderControlMode::CtxEnforced => {}
    }
}

const DEFAULT_TURN_START_DEADLINE: Duration = Duration::from_secs(60);

pub(super) fn turn_start_deadline() -> Duration {
    std::env::var("CTX_TURN_START_DEADLINE_MS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .map(Duration::from_millis)
        .unwrap_or(DEFAULT_TURN_START_DEADLINE)
}
