use super::*;

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

    let environment = match effective.mode {
        ctx_settings_model::ExecutionMode::Host => "host",
        ctx_settings_model::ExecutionMode::Sandbox => "sandbox",
    }
    .to_string();
    let network_mode = match effective.container.network_mode {
        ctx_settings_model::ContainerNetworkMode::LlmOnly => "llm_only",
        ctx_settings_model::ContainerNetworkMode::Allowlist => "allowlist",
        ctx_settings_model::ContainerNetworkMode::All => "all",
    }
    .to_string();

    Ok(WorkspaceExecutionConfigResp {
        source,
        environment,
        network_mode: Some(network_mode),
        allowlist: Some(effective.container.allowlist.clone()),
    })
}

pub(in crate::api::workspaces::management) async fn update_workspace_execution_config(
    state: &Arc<AppState>,
    ctx: &WorkspaceRequestContext,
    req: UpdateExecutionConfigReq,
) -> WorkspaceApiResult<UpdateWorkspaceConfigResp> {
    let environment = match req.environment.trim() {
        "host" => ctx_workspace_config::ExecutionEnvironment::Host,
        "sandbox" => {
            #[cfg(target_os = "macos")]
            {
                if !ctx_harness_runtime::local_runtime_available(
                    &state.core.data_root,
                    &ctx_settings_model::ContainerRuntimeKind::SharedVmContainer,
                ) {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        Json(ApiErrorResp {
                            error: "AVF sandbox is unavailable on this macOS host. Install or launch through the desktop app so the AVF helper/runtime is present, then try again.".to_string(),
                        }),
                    ));
                }
            }
            ctx_workspace_config::ExecutionEnvironment::Sandbox
        }
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "invalid environment (expected host|sandbox)".to_string(),
                }),
            ));
        }
    };

    let network_mode = match req.network_mode.as_deref().map(|value| value.trim()) {
        None | Some("") => None,
        Some("llm_only") => Some(ctx_settings_model::ContainerNetworkMode::LlmOnly),
        Some("allowlist") => Some(ctx_settings_model::ContainerNetworkMode::Allowlist),
        Some("all") => Some(ctx_settings_model::ContainerNetworkMode::All),
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "invalid network_mode (expected llm_only|allowlist|all)".to_string(),
                }),
            ));
        }
    };

    let allowlist = req.allowlist.map(|values| {
        values
            .into_iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect::<Vec<String>>()
    });

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
    let requested_override = ctx_workspace_config::ExecutionSettingsOverride {
        mode: Some(match environment {
            ctx_workspace_config::ExecutionEnvironment::Host => {
                ctx_settings_model::ExecutionMode::Host
            }
            ctx_workspace_config::ExecutionEnvironment::Sandbox => {
                ctx_settings_model::ExecutionMode::Sandbox
            }
        }),
        container: ctx_workspace_config::ContainerExecutionSettingsOverride {
            network_mode: network_mode.clone(),
            allowlist: allowlist.clone(),
            image: None,
        },
    };
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
