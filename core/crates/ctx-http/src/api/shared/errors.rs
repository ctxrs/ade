use axum::http::StatusCode;
use axum::Json;
use ctx_observability::logs;
use ctx_settings_service::EffectiveExecutionSettingsError;

use crate::api::errors::ApiErrorResp;

pub(crate) fn status_code_for_request_or_policy_error(err: &anyhow::Error) -> StatusCode {
    if ctx_settings_service::is_execution_policy_denial(err) {
        StatusCode::FORBIDDEN
    } else {
        StatusCode::BAD_REQUEST
    }
}

pub(crate) fn map_effective_execution_settings_error(
    err: EffectiveExecutionSettingsError,
) -> (StatusCode, Json<ApiErrorResp>) {
    let (status, error) = match err {
        EffectiveExecutionSettingsError::InvalidWorkspaceOverride(err) => {
            (status_code_for_request_or_policy_error(&err), err)
        }
        EffectiveExecutionSettingsError::Internal(err) => (StatusCode::INTERNAL_SERVER_ERROR, err),
    };
    (
        status,
        Json(ApiErrorResp {
            error: logs::redact_sensitive(&error.to_string()),
        }),
    )
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

    use super::*;

    #[test]
    fn status_code_for_request_or_policy_error_maps_policy_denials_to_forbidden() {
        let err = ctx_settings_service::HostExecutionPolicy::SandboxOnly
            .validate_execution_environment(ctx_core::models::ExecutionEnvironment::Host)
            .expect_err("host execution should be denied");
        assert_eq!(
            status_code_for_request_or_policy_error(&err),
            StatusCode::FORBIDDEN
        );
    }

    #[test]
    fn status_code_for_request_or_policy_error_preserves_bad_request_for_validation_errors() {
        let err = anyhow::anyhow!("invalid request");
        assert_eq!(
            status_code_for_request_or_policy_error(&err),
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn map_effective_execution_settings_error_maps_policy_denials_to_forbidden() {
        let err = ctx_settings_service::HostExecutionPolicy::SandboxOnly
            .validate_execution_environment(ctx_core::models::ExecutionEnvironment::Host)
            .expect_err("host execution should be denied");
        let (status, _) = map_effective_execution_settings_error(
            EffectiveExecutionSettingsError::InvalidWorkspaceOverride(err),
        );

        assert_eq!(status, StatusCode::FORBIDDEN);
    }
}
