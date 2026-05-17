use ctx_provider_runtime::provider_workers::ProviderAdapterRestartResult;
use ctx_providers::adapters::ProviderRestartMode;
use serde::{Deserialize, Serialize};

use crate::daemon::ProvidersHandle;

use super::inventory::{refresh_provider_inventory, ProviderMatrixRefreshSummary};

#[derive(Debug, Clone, Serialize)]
pub struct ProviderMatrixRefreshRouteResponse {
    provider_count: usize,
    generated_at: Option<String>,
    source: String,
    degraded: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_error: Option<String>,
}

impl From<ProviderMatrixRefreshSummary> for ProviderMatrixRefreshRouteResponse {
    fn from(summary: ProviderMatrixRefreshSummary) -> Self {
        Self {
            provider_count: summary.provider_count,
            generated_at: summary.generated_at,
            source: summary.source,
            degraded: summary.degraded,
            last_error: summary.last_error,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ProviderDevRestartRouteRequest {
    mode: String,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderDevRestartRouteResponse {
    mode: String,
    results: Vec<ProviderDevRestartRouteResult>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderDevRestartRouteResult {
    provider_id: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

impl From<ProviderAdapterRestartResult> for ProviderDevRestartRouteResult {
    fn from(result: ProviderAdapterRestartResult) -> Self {
        Self {
            provider_id: result.provider_id,
            status: result.status.as_str().to_string(),
            message: result.message,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderAdminRouteErrorKind {
    NotFound,
    BadRequest,
    Internal,
}

#[derive(Debug)]
pub struct ProviderAdminRouteError {
    kind: ProviderAdminRouteErrorKind,
    message: String,
}

impl ProviderAdminRouteError {
    pub fn kind(&self) -> ProviderAdminRouteErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            kind: ProviderAdminRouteErrorKind::NotFound,
            message: message.into(),
        }
    }

    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            kind: ProviderAdminRouteErrorKind::BadRequest,
            message: message.into(),
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self {
            kind: ProviderAdminRouteErrorKind::Internal,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ProviderAdminRouteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

#[derive(Debug)]
struct ProviderDevRestartRoutePlan {
    mode: ProviderRestartMode,
    reason: String,
}

impl ProvidersHandle {
    pub async fn refresh_provider_matrix_for_route(
        &self,
    ) -> Result<ProviderMatrixRefreshRouteResponse, ProviderAdminRouteError> {
        refresh_provider_inventory(self.state.as_ref())
            .await
            .map(Into::into)
            .map_err(matrix_refresh_route_error)
    }

    pub async fn dev_restart_providers_for_route(
        &self,
        request: ProviderDevRestartRouteRequest,
    ) -> Result<ProviderDevRestartRouteResponse, ProviderAdminRouteError> {
        let plan = dev_restart_route_plan(dev_tools_enabled(), request)?;
        let results = self
            .state
            .providers
            .restart_all_provider_adapters(&plan.reason, plan.mode)
            .await;
        Ok(dev_restart_route_response(plan.mode, results))
    }
}

fn matrix_refresh_route_error(error: anyhow::Error) -> ProviderAdminRouteError {
    ProviderAdminRouteError::internal(format!("failed to refresh provider statuses: {error:#}"))
}

fn dev_tools_enabled() -> bool {
    std::env::var("CTX_DEV_MODE")
        .ok()
        .as_deref()
        .and_then(ctx_core::boolish::parse_boolish)
        .unwrap_or(false)
}

fn parse_restart_mode(value: &str) -> Option<ProviderRestartMode> {
    match value.trim().to_lowercase().as_str() {
        "immediate" => Some(ProviderRestartMode::Immediate),
        "drain" => Some(ProviderRestartMode::Drain),
        _ => None,
    }
}

fn dev_restart_route_plan(
    enabled: bool,
    request: ProviderDevRestartRouteRequest,
) -> Result<ProviderDevRestartRoutePlan, ProviderAdminRouteError> {
    if !enabled {
        return Err(ProviderAdminRouteError::not_found("dev tools are disabled"));
    }

    let Some(mode) = parse_restart_mode(&request.mode) else {
        return Err(ProviderAdminRouteError::bad_request(
            "mode must be 'immediate' or 'drain'",
        ));
    };
    let reason = request
        .reason
        .unwrap_or_else(|| format!("dev restart ({})", mode.as_str()));
    Ok(ProviderDevRestartRoutePlan { mode, reason })
}

fn dev_restart_route_response(
    mode: ProviderRestartMode,
    results: Vec<ProviderAdapterRestartResult>,
) -> ProviderDevRestartRouteResponse {
    ProviderDevRestartRouteResponse {
        mode: mode.as_str().to_string(),
        results: results.into_iter().map(Into::into).collect(),
    }
}

#[cfg(test)]
mod route_tests {
    use std::sync::{Mutex, OnceLock};

    use ctx_provider_runtime::provider_workers::{
        ProviderAdapterRestartResult, ProviderAdapterRestartStatus,
    };

    use super::*;

    struct EnvVarGuard {
        key: &'static str,
        prev: Option<String>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let prev = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, prev }
        }

        fn remove(key: &'static str) -> Self {
            let prev = std::env::var(key).ok();
            std::env::remove_var(key);
            Self { key, prev }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(prev) = self.prev.take() {
                std::env::set_var(self.key, prev);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    fn dev_mode_env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn provider_admin_route_error_preserves_kind_and_message() {
        let error = ProviderAdminRouteError::bad_request("bad route");

        assert_eq!(error.kind(), ProviderAdminRouteErrorKind::BadRequest);
        assert_eq!(error.message(), "bad route");
        assert_eq!(error.to_string(), "bad route");
    }

    #[test]
    fn matrix_refresh_route_error_preserves_existing_prefix() {
        let error = matrix_refresh_route_error(anyhow::anyhow!("parsing agent server config"));

        assert_eq!(error.kind(), ProviderAdminRouteErrorKind::Internal);
        assert_eq!(
            error.message(),
            "failed to refresh provider statuses: parsing agent server config"
        );
    }

    #[test]
    fn dev_restart_route_plan_rejects_disabled_dev_tools() {
        let error = dev_restart_route_plan(
            false,
            ProviderDevRestartRouteRequest {
                mode: "immediate".to_string(),
                reason: None,
            },
        )
        .unwrap_err();

        assert_eq!(error.kind(), ProviderAdminRouteErrorKind::NotFound);
        assert_eq!(error.message(), "dev tools are disabled");
    }

    #[test]
    fn dev_restart_route_plan_rejects_unknown_mode() {
        let error = dev_restart_route_plan(
            true,
            ProviderDevRestartRouteRequest {
                mode: "later".to_string(),
                reason: None,
            },
        )
        .unwrap_err();

        assert_eq!(error.kind(), ProviderAdminRouteErrorKind::BadRequest);
        assert_eq!(error.message(), "mode must be 'immediate' or 'drain'");
    }

    #[test]
    fn dev_restart_route_plan_defaults_reason_from_mode() {
        let plan = dev_restart_route_plan(
            true,
            ProviderDevRestartRouteRequest {
                mode: " DRAIN ".to_string(),
                reason: None,
            },
        )
        .unwrap();

        assert_eq!(plan.mode, ProviderRestartMode::Drain);
        assert_eq!(plan.reason, "dev restart (drain)");
    }

    #[test]
    fn dev_restart_route_plan_preserves_custom_reason() {
        let plan = dev_restart_route_plan(
            true,
            ProviderDevRestartRouteRequest {
                mode: "immediate".to_string(),
                reason: Some("operator request".to_string()),
            },
        )
        .unwrap();

        assert_eq!(plan.mode, ProviderRestartMode::Immediate);
        assert_eq!(plan.reason, "operator request");
    }

    #[test]
    fn dev_restart_response_preserves_json_shape_and_skips_empty_message() {
        let response = dev_restart_route_response(
            ProviderRestartMode::Immediate,
            vec![
                ProviderAdapterRestartResult {
                    provider_id: "ok-provider".to_string(),
                    status: ProviderAdapterRestartStatus::Ok,
                    message: None,
                },
                ProviderAdapterRestartResult {
                    provider_id: "bad-provider".to_string(),
                    status: ProviderAdapterRestartStatus::Error,
                    message: Some("restart failed".to_string()),
                },
            ],
        );
        let payload = serde_json::to_value(response).unwrap();

        assert_eq!(payload["mode"].as_str(), Some("immediate"));
        assert_eq!(
            payload["results"][0]["provider_id"].as_str(),
            Some("ok-provider")
        );
        assert_eq!(payload["results"][0]["status"].as_str(), Some("ok"));
        assert!(payload["results"][0].get("message").is_none());
        assert_eq!(
            payload["results"][1]["message"].as_str(),
            Some("restart failed")
        );
    }

    #[test]
    fn dev_tools_enabled_reads_boolish_env() {
        let _serial = dev_mode_env_lock().lock().expect("dev mode env lock");
        let _guard = EnvVarGuard::set("CTX_DEV_MODE", "yes");

        assert!(dev_tools_enabled());
    }

    #[test]
    fn dev_tools_enabled_defaults_false_when_env_missing() {
        let _serial = dev_mode_env_lock().lock().expect("dev mode env lock");
        let _guard = EnvVarGuard::remove("CTX_DEV_MODE");

        assert!(!dev_tools_enabled());
    }
}
