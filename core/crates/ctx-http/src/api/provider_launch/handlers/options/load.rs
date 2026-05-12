use std::time::Duration;

use ctx_core::ids::WorkspaceId;
use ctx_core::models::Workspace;
use ctx_harness_sources::HarnessEndpointRecord;
use ctx_provider_install::install_state::InstallTarget;

use super::*;

pub(super) enum ProviderOptionsLoadOutcome {
    Cached(serde_json::Value),
    Ready(Box<ProviderOptionsInputs>),
}

pub(super) struct ProviderOptionsInputs {
    pub(super) workspace_id: WorkspaceId,
    pub(super) install_target: InstallTarget,
    pub(super) managed: crate::daemon::installer::AgentServerConfigFile,
    pub(super) managed_config_error: Option<String>,
    pub(super) matrix: ctx_provider_matrix::ProviderMatrix,
    pub(super) source_config: Option<harness_sources::HarnessProviderSourceConfig>,
    pub(super) source_config_error: Option<String>,
    pub(super) cache: ProviderOptionsCacheSnapshot,
    pub(super) workspace: Workspace,
    pub(super) preferred_model_id: Option<String>,
    pub(super) selected_endpoint: Option<HarnessEndpointRecord>,
}

pub(super) async fn load_provider_options_inputs(
    state: &Arc<AppState>,
    ws_id: &str,
    provider_id: &str,
    cache_ttl: Duration,
    verify_ttl: Duration,
) -> Result<ProviderOptionsLoadOutcome, (StatusCode, Json<serde_json::Value>)> {
    let workspace_id = parse_workspace_id(ws_id)?;
    let install_target = install_target_for_workspace(state, workspace_id)
        .await
        .map_err(|error| workspace_execution_settings_error_json(&error))?;
    let (managed, managed_config_error) =
        load_managed_agent_server_config_with_error(&state.core.data_root).await;
    let matrix = ctx_provider_matrix::load_matrix_cached(
        &state.core.data_root,
        &state.providers.matrix_cache,
    )
    .await;
    let known = provider_is_known(state, &matrix, provider_id).await;
    let (source_config, source_config_error) =
        load_provider_source_config_with_error(&state.core.data_root, provider_id).await;
    let skip_cached_config_surfaces =
        managed_config_error.is_some() || source_config_error.is_some();
    let cache = ProviderOptionsCacheSnapshot::load(
        state,
        workspace_id,
        install_target,
        provider_id,
        skip_cached_config_surfaces,
    )
    .await;

    if let Some(out) = cache.fresh_authoritative_response(cache_ttl, verify_ttl) {
        return Ok(ProviderOptionsLoadOutcome::Cached(out));
    }
    ensure_known_provider(provider_id, known)?;

    let workspace = load_workspace(state, workspace_id).await?;
    let preferred_model_id =
        load_workspace_preferred_model_id(state, workspace_id, provider_id).await?;
    let selected_endpoint = selected_endpoint_record_from_harness_config(source_config.as_ref());

    Ok(ProviderOptionsLoadOutcome::Ready(Box::new(
        ProviderOptionsInputs {
            workspace_id,
            install_target,
            managed,
            managed_config_error,
            matrix,
            source_config,
            source_config_error,
            cache,
            workspace,
            preferred_model_id,
            selected_endpoint,
        },
    )))
}

async fn provider_is_known(
    state: &Arc<AppState>,
    matrix: &ctx_provider_matrix::ProviderMatrix,
    provider_id: &str,
) -> bool {
    state
        .providers
        .with_provider_statuses(|map| {
            map.contains_key(provider_id)
                || ctx_provider_matrix::get_entry(matrix, provider_id).is_some()
        })
        .await
}

fn ensure_known_provider(
    provider_id: &str,
    known: bool,
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    if known {
        return Ok(());
    }

    Err((
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({
            "error": format!("unsupported provider id: {provider_id}"),
        })),
    ))
}

async fn load_workspace(
    state: &Arc<AppState>,
    ws_id: WorkspaceId,
) -> Result<Workspace, (StatusCode, Json<serde_json::Value>)> {
    state
        .global_store()
        .get_workspace(ws_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "failed to load workspace",
                })),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "workspace not found",
            })),
        ))
}
