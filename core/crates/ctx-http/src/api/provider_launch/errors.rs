use axum::http::StatusCode;
use axum::Json;
use ctx_observability::logs;

use crate::daemon::providers::{ProviderLaunchConfigError, StartProviderInstallError};

pub(in crate::api::provider_launch) fn workspace_execution_settings_error_json(
    error: &anyhow::Error,
) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({
            "error": format!("failed to load workspace execution settings: {error:#}"),
        })),
    )
}

pub(in crate::api::provider_launch) fn provider_install_error_response(
    error: StartProviderInstallError,
) -> (StatusCode, Json<serde_json::Value>) {
    let status = if error.code.as_deref() == Some("install_target_disabled") {
        StatusCode::FORBIDDEN
    } else {
        StatusCode::BAD_REQUEST
    };
    (
        status,
        Json(serde_json::json!({
            "error": logs::redact_sensitive(&error.message),
            "code": error.code,
        })),
    )
}

pub(in crate::api::provider_launch) fn provider_launch_config_error_response(
    error: ProviderLaunchConfigError,
) -> (StatusCode, Json<serde_json::Value>) {
    match error {
        ProviderLaunchConfigError::UnsupportedProvider { provider_id } => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!("unsupported provider id: {provider_id}"),
            })),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::provider_install_error_response;
    use crate::daemon::providers::StartProviderInstallError;
    use axum::http::StatusCode;

    #[test]
    fn provider_install_error_response_maps_disabled_install_targets_to_forbidden() {
        let (status, body) = provider_install_error_response(StartProviderInstallError {
            message: "host provider installs are disabled by daemon policy".to_string(),
            code: Some("install_target_disabled".to_string()),
        });

        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body.0["code"], "install_target_disabled");
    }
}
