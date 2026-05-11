use super::*;
use crate::api::sessions;
use ctx_provider_install::InstallTarget;
use ctx_session_service::default_session::{
    resolve_default_session_model, select_default_provider_id,
};

pub(super) async fn resolve_default_session_target(
    state: &Arc<AppState>,
    store: &Store,
    workspace: &Workspace,
    execution_environment: ExecutionEnvironment,
) -> Result<(String, String, Option<String>), (StatusCode, Json<ApiErrorResp>)> {
    let install_target = crate::api::providers::install_target_for_workspace(state, workspace.id)
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&error.to_string()),
                }),
            )
        })?;
    let mut statuses =
        crate::api::providers::providers_statuses_response(state, install_target, true).await;
    let provider_id = match select_default_provider_id(&statuses) {
        Some(provider_id) => provider_id,
        None if install_target == InstallTarget::Host => {
            crate::daemon::installer::refresh_provider_statuses(state.as_ref())
                .await
                .map_err(|error| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiErrorResp {
                            error: logs::redact_sensitive(&error.to_string()),
                        }),
                    )
                })?;
            statuses =
                crate::api::providers::providers_statuses_response(state, install_target, true)
                    .await;
            select_default_provider_id(&statuses).ok_or((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "no provider available for default session".to_string(),
                }),
            ))?
        }
        None => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "no provider available for default session".to_string(),
                }),
            ));
        }
    };
    let provider_status = statuses
        .iter()
        .find(|status| status.provider_id == provider_id);
    let preferred_model_id =
        ctx_workspace_config::load_preferred_new_session_model_id(store, &provider_id)
            .await
            .map_err(|error| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&error.to_string()),
                    }),
                )
            })?;
    let catalog = sessions::load_provider_model_catalog_for_execution_environment(
        state,
        workspace,
        &provider_id,
        execution_environment,
    )
    .await
    .map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&error),
            }),
        )
    })?;
    let resolved_model = resolve_default_session_model(
        preferred_model_id.as_deref(),
        catalog.as_ref(),
        provider_status,
    )
    .map_err(|error| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: format!(
                    "failed to resolve default model for provider '{provider_id}': {error}"
                ),
            }),
        )
    })?;
    Ok((
        provider_id,
        resolved_model.model_id,
        resolved_model.reasoning_effort,
    ))
}
