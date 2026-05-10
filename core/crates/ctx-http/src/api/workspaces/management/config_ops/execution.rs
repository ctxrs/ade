use super::*;

#[path = "execution/codec.rs"]
mod codec;

use codec::{
    build_workspace_execution_config_override, normalize_execution_allowlist,
    parse_execution_environment, parse_execution_network_mode, project_workspace_execution_config,
};

pub(in crate::api::workspaces::management) async fn load_workspace_execution_config(
    state: &Arc<AppState>,
    ctx: &WorkspaceRequestContext,
) -> WorkspaceApiResult<WorkspaceExecutionConfigResp> {
    let settings = ctx_settings_service::load_settings(state.global_store())
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&error.to_string()),
                }),
            )
        })?;
    let mut effective = settings.execution.clone().unwrap_or_default();
    let mut source = "daemon_default".to_string();
    match workspace_config::load_execution_settings_override(&ctx.store).await {
        Ok(Some(override_config)) => {
            execution_effective::apply_workspace_execution_settings_override(
                &mut effective,
                &override_config,
            )
            .map_err(|error| {
                (
                    crate::api::shared::status_code_for_request_or_policy_error(&error),
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&error.to_string()),
                    }),
                )
            })?;
            source = "workspace".to_string();
        }
        Ok(None) => {}
        Err(error) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&error.to_string()),
                }),
            ));
        }
    }

    Ok(project_workspace_execution_config(source, &effective))
}

pub(in crate::api::workspaces::management) async fn update_workspace_execution_config(
    state: &Arc<AppState>,
    ctx: &WorkspaceRequestContext,
    req: UpdateExecutionConfigReq,
) -> WorkspaceApiResult<UpdateWorkspaceConfigResp> {
    let environment = parse_execution_environment(state, req.environment.trim())?;
    let network_mode = parse_execution_network_mode(req.network_mode.as_deref())?;
    let allowlist = req.allowlist.map(normalize_execution_allowlist);

    let settings = ctx_settings_service::load_settings(state.global_store())
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&error.to_string()),
                }),
            )
        })?;
    let effective = settings.execution.clone().unwrap_or_default();
    let requested_override = build_workspace_execution_config_override(
        environment,
        network_mode.clone(),
        allowlist.clone(),
    );
    execution_effective::validate_workspace_execution_settings_override(
        &effective,
        &requested_override,
    )
    .map_err(|error| {
        (
            crate::api::shared::status_code_for_request_or_policy_error(&error),
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&error.to_string()),
            }),
        )
    })?;

    workspace_config::update_execution_config(
        &ctx.store,
        workspace_config::ExecutionConfigUpdate {
            environment,
            network_mode,
            allowlist,
            image: None,
        },
    )
    .await
    .map_err(|error| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&error.to_string()),
            }),
        )
    })?;

    Ok(UpdateWorkspaceConfigResp { ok: true })
}
