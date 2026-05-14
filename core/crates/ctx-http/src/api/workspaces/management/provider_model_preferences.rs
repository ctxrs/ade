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

impl From<crate::daemon::workspaces::WorkspaceProviderModelPreference>
    for WorkspaceProviderModelPreferenceResp
{
    fn from(value: crate::daemon::workspaces::WorkspaceProviderModelPreference) -> Self {
        Self {
            provider_id: value.provider_id,
            preferred_model_id: value.preferred_model_id,
        }
    }
}

fn provider_model_preference_error_response(
    error: crate::daemon::workspaces::WorkspaceProviderModelPreferenceError,
) -> (StatusCode, Json<ApiErrorResp>) {
    match error {
        crate::daemon::workspaces::WorkspaceProviderModelPreferenceError::ProviderIdRequired => (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "provider_id is required".to_string(),
            }),
        ),
        crate::daemon::workspaces::WorkspaceProviderModelPreferenceError::ProviderNotFound {
            provider_id,
        } => (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: format!("provider not found: {provider_id}"),
            }),
        ),
        crate::daemon::workspaces::WorkspaceProviderModelPreferenceError::WorkspaceNotFound => (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "workspace not found".to_string(),
            }),
        ),
        crate::daemon::workspaces::WorkspaceProviderModelPreferenceError::StoreUnavailable(
            error,
        ) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&error.to_string()),
            }),
        ),
        crate::daemon::workspaces::WorkspaceProviderModelPreferenceError::ExecutionSettings(
            error,
        ) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: format!("failed to load workspace execution settings: {error:#}"),
            }),
        ),
    }
}

pub(in crate::api) async fn get_workspace_provider_model_preference(
    State(workspaces): State<WorkspacesHandle>,
    Path((id, provider_id)): Path<(String, String)>,
) -> Result<Json<WorkspaceProviderModelPreferenceResp>, (StatusCode, Json<ApiErrorResp>)> {
    let workspace_id = parse_workspace_id(&id)?;
    workspaces
        .get_workspace_provider_model_preference(workspace_id, &provider_id)
        .await
        .map(WorkspaceProviderModelPreferenceResp::from)
        .map(Json)
        .map_err(provider_model_preference_error_response)
}

pub(in crate::api) async fn update_workspace_provider_model_preference(
    State(workspaces): State<WorkspacesHandle>,
    Path((id, provider_id)): Path<(String, String)>,
    Json(req): Json<UpdateWorkspaceProviderModelPreferenceReq>,
) -> Result<Json<WorkspaceProviderModelPreferenceResp>, (StatusCode, Json<ApiErrorResp>)> {
    let workspace_id = parse_workspace_id(&id)?;
    workspaces
        .set_workspace_provider_model_preference(workspace_id, &provider_id, req.preferred_model_id)
        .await
        .map(WorkspaceProviderModelPreferenceResp::from)
        .map(Json)
        .map_err(provider_model_preference_error_response)
}
