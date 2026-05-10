use super::*;

pub(super) fn project_workspace_execution_config(
    source: String,
    effective: &ctx_settings_model::ExecutionSettings,
) -> WorkspaceExecutionConfigResp {
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

    WorkspaceExecutionConfigResp {
        source,
        environment,
        network_mode: Some(network_mode),
        allowlist: Some(effective.container.allowlist.clone()),
    }
}

pub(super) fn parse_execution_environment(
    state: &Arc<AppState>,
    environment: &str,
) -> WorkspaceApiResult<ctx_workspace_config::ExecutionEnvironment> {
    match environment {
        "host" => Ok(ctx_workspace_config::ExecutionEnvironment::Host),
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
            Ok(ctx_workspace_config::ExecutionEnvironment::Sandbox)
        }
        _ => Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid environment (expected host|sandbox)".to_string(),
            }),
        )),
    }
}

pub(super) fn parse_execution_network_mode(
    network_mode: Option<&str>,
) -> WorkspaceApiResult<Option<ctx_settings_model::ContainerNetworkMode>> {
    match network_mode.map(|value| value.trim()) {
        None | Some("") => Ok(None),
        Some("llm_only") => Ok(Some(ctx_settings_model::ContainerNetworkMode::LlmOnly)),
        Some("allowlist") => Ok(Some(ctx_settings_model::ContainerNetworkMode::Allowlist)),
        Some("all") => Ok(Some(ctx_settings_model::ContainerNetworkMode::All)),
        _ => Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid network_mode (expected llm_only|allowlist|all)".to_string(),
            }),
        )),
    }
}

pub(super) fn normalize_execution_allowlist(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect()
}

pub(super) fn build_workspace_execution_config_override(
    environment: ctx_workspace_config::ExecutionEnvironment,
    network_mode: Option<ctx_settings_model::ContainerNetworkMode>,
    allowlist: Option<Vec<String>>,
) -> ctx_workspace_config::ExecutionSettingsOverride {
    ctx_workspace_config::ExecutionSettingsOverride {
        mode: Some(match environment {
            ctx_workspace_config::ExecutionEnvironment::Host => {
                ctx_settings_model::ExecutionMode::Host
            }
            ctx_workspace_config::ExecutionEnvironment::Sandbox => {
                ctx_settings_model::ExecutionMode::Sandbox
            }
        }),
        container: ctx_workspace_config::ContainerExecutionSettingsOverride {
            network_mode,
            allowlist,
            image: None,
        },
    }
}
