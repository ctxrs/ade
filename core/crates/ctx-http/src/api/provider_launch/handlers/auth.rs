use super::*;

mod verify;

pub(in crate::api) use verify::verify_provider_for_workspace;

pub(in crate::api) async fn authenticate_provider_for_workspace(
    State(state): State<Arc<AppState>>,
    Path((ws_id, provider_id)): Path<(String, String)>,
    req: Option<Json<AuthenticateProviderReq>>,
) -> Result<Json<ProviderAuthCheckResp>, (StatusCode, Json<serde_json::Value>)> {
    let ws_id = parse_workspace_id(&ws_id)?;
    let method_id = req.and_then(|value| value.0.method_id);
    let workspace = state
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
        ))?;

    let auth =
        authenticate_provider_for_workspace_runtime(&state, &workspace, &provider_id, method_id)
            .await
            .map_err(|error| match error {
                ProviderWorkspaceAuthenticationError::ExecutionSettings(error) => {
                    workspace_execution_settings_error_json(&error)
                }
                ProviderWorkspaceAuthenticationError::Verify(error) => (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": error,
                    })),
                ),
            })?;

    let resp = match auth.error_message {
        None => ProviderAuthCheckResp {
            provider_id: provider_id.clone(),
            workspace_id: ws_id.0.to_string(),
            status: "ok".to_string(),
            auth_required: Some(false),
            checked_at: Some(auth.checked_at.clone()),
            message: None,
        },
        Some(message) => {
            let (status, auth_required, _) = classify_probe_error(&message);
            ProviderAuthCheckResp {
                provider_id: provider_id.clone(),
                workspace_id: ws_id.0.to_string(),
                status: status.to_string(),
                auth_required,
                checked_at: Some(auth.checked_at.clone()),
                message: Some(message),
            }
        }
    };
    let verify_value =
        redact_json_value(serde_json::to_value(&resp).unwrap_or(serde_json::Value::Null));
    store_provider_verify_cache_value(
        &state,
        ws_id,
        auth.install_target,
        &provider_id,
        verify_value,
    )
    .await;

    Ok(Json(resp))
}
