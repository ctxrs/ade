use super::*;

#[derive(Debug, Deserialize)]
pub(in crate::api) struct UpdateWorkspaceProviderModelPreferenceReq {
    #[serde(default)]
    preferred_model_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub(in crate::api) struct WorkspaceProviderModelPreferenceResp {
    provider_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    preferred_model_id: Option<String>,
}

async fn require_known_provider(
    state: &Arc<AppState>,
    provider_id: &str,
) -> Result<(), (StatusCode, Json<ApiErrorResp>)> {
    let provider_id = provider_id.trim();
    if provider_id.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "provider_id is required".to_string(),
            }),
        ));
    }
    let matrix = state
        .providers
        .load_provider_matrix(&state.core.data_root)
        .await;
    let known = ctx_provider_matrix::get_entry(&matrix, provider_id).is_some()
        || state.providers.has_provider_status(provider_id).await
        || state.providers.has_provider_adapter(provider_id).await;
    if known {
        return Ok(());
    }
    Err((
        StatusCode::NOT_FOUND,
        Json(ApiErrorResp {
            error: format!("provider not found: {provider_id}"),
        }),
    ))
}

async fn load_effective_preferred_model_id(
    state: &Arc<AppState>,
    workspace: &ctx_core::models::Workspace,
    provider_id: &str,
) -> Result<Option<String>, (StatusCode, Json<ApiErrorResp>)> {
    let options = crate::api::provider_launch::get_provider_options(
        State(Arc::clone(state)),
        Path((workspace.id.0.to_string(), provider_id.to_string())),
    )
    .await
    .map_err(|(status, body)| {
        (
            status,
            Json(ApiErrorResp {
                error: body
                    .0
                    .get("error")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("provider options unavailable")
                    .to_string(),
            }),
        )
    })?;
    Ok(options
        .0
        .get("preferred_model_id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string))
}

pub(in crate::api) async fn get_workspace_provider_model_preference(
    State(state): State<Arc<AppState>>,
    Path((id, provider_id)): Path<(String, String)>,
) -> Result<Json<WorkspaceProviderModelPreferenceResp>, (StatusCode, Json<ApiErrorResp>)> {
    let workspace_id = parse_workspace_id(&id)?;
    let workspace = require_workspace(&state, workspace_id).await?;
    require_known_provider(&state, &provider_id).await?;
    let provider_id = provider_id.trim().to_string();
    let preferred_model_id =
        load_effective_preferred_model_id(&state, &workspace, &provider_id).await?;
    Ok(Json(WorkspaceProviderModelPreferenceResp {
        provider_id,
        preferred_model_id,
    }))
}

pub(in crate::api) async fn update_workspace_provider_model_preference(
    State(state): State<Arc<AppState>>,
    Path((id, provider_id)): Path<(String, String)>,
    Json(req): Json<UpdateWorkspaceProviderModelPreferenceReq>,
) -> Result<Json<WorkspaceProviderModelPreferenceResp>, (StatusCode, Json<ApiErrorResp>)> {
    let workspace_id = parse_workspace_id(&id)?;
    let workspace = require_workspace(&state, workspace_id).await?;
    require_known_provider(&state, &provider_id).await?;
    let provider_id = provider_id.trim().to_string();
    crate::api::workspace_provider_model_preferences::update_workspace_provider_preferred_model_id(
        &state,
        workspace_id,
        &provider_id,
        req.preferred_model_id,
    )
    .await
    .map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&error.to_string()),
            }),
        )
    })?;
    let preferred_model_id =
        load_effective_preferred_model_id(&state, &workspace, &provider_id).await?;
    Ok(Json(WorkspaceProviderModelPreferenceResp {
        provider_id,
        preferred_model_id,
    }))
}
