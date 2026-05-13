use axum::http::StatusCode;
use axum::Json;
use ctx_observability::logs;
use ctx_settings_service::EffectiveExecutionSettingsError;
use ctx_storage_admission::is_storage_exhaustion_error;

use crate::api::errors::ApiErrorResp;

pub(crate) fn status_code_for_internal_error(err: &anyhow::Error) -> StatusCode {
    if ctx_settings_service::is_execution_policy_denial(err) {
        StatusCode::FORBIDDEN
    } else if err
        .chain()
        .any(|cause| is_storage_exhaustion_error(&cause.to_string()))
    {
        StatusCode::INSUFFICIENT_STORAGE
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

pub(crate) fn status_code_for_request_or_policy_error(err: &anyhow::Error) -> StatusCode {
    if ctx_settings_service::is_execution_policy_denial(err) {
        StatusCode::FORBIDDEN
    } else {
        StatusCode::BAD_REQUEST
    }
}

fn internal_api_error_message(err: &anyhow::Error) -> String {
    if let Some(storage_message) = err
        .chain()
        .map(ToString::to_string)
        .find(|message| is_storage_exhaustion_error(message))
    {
        storage_message
    } else {
        format!("{err:#}")
    }
}

pub(crate) fn map_internal_api_error(err: &anyhow::Error) -> (StatusCode, Json<ApiErrorResp>) {
    (
        status_code_for_internal_error(err),
        Json(ApiErrorResp {
            error: logs::redact_sensitive(&internal_api_error_message(err)),
        }),
    )
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
    fn status_code_for_internal_error_maps_storage_failures_to_insufficient_storage() {
        let err =
            anyhow::anyhow!("Insufficient storage capacity for creating an isolated task worktree");
        assert_eq!(
            status_code_for_internal_error(&err),
            StatusCode::INSUFFICIENT_STORAGE
        );
    }

    #[test]
    fn status_code_for_internal_error_preserves_generic_internal_errors() {
        let err = anyhow::anyhow!("plain internal failure");
        assert_eq!(
            status_code_for_internal_error(&err),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn status_code_for_internal_error_maps_execution_policy_denials_to_forbidden() {
        let err = ctx_settings_service::HostExecutionPolicy::SandboxOnly
            .validate_execution_environment(ctx_core::models::ExecutionEnvironment::Host)
            .expect_err("host execution should be denied");
        assert_eq!(status_code_for_internal_error(&err), StatusCode::FORBIDDEN);
    }

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
    fn map_internal_api_error_preserves_storage_guidance() {
        let err = anyhow::anyhow!("wrapper")
            .context("Insufficient storage capacity for creating an isolated task worktree");
        let (status, body) = map_internal_api_error(&err);
        assert_eq!(status, StatusCode::INSUFFICIENT_STORAGE);
        assert_eq!(
            body.0.error,
            "Insufficient storage capacity for creating an isolated task worktree"
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
