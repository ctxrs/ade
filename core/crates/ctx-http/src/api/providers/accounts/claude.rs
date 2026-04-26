use super::common::{bad_request, internal_error, provider_account_delete_error};
use super::*;

pub(crate) async fn claude_accounts_response(
    state: &Arc<AppState>,
) -> anyhow::Result<ClaudeAccountsResponse> {
    let registry = provider_accounts::load_claude_registry(&state.core.data_root).await?;
    Ok(ClaudeAccountsResponse {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
    })
}

pub(crate) async fn list_claude_accounts(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ClaudeAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    Ok(Json(
        claude_accounts_response(&state)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn upsert_claude_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ClaudeAccountUpsertReq>,
) -> Result<Json<ClaudeAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::add_claude_account(&state.core.data_root, req.label, req.setup_token)
        .await
        .map_err(bad_request)?;
    restarts::restart_claude_providers_for_auth_change(&state, "claude auth updated").await;
    Ok(Json(
        claude_accounts_response(&state)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn set_claude_active_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ClaudeActiveAccountReq>,
) -> Result<Json<ClaudeAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    if let Some(ref account_id) = req.account_id {
        let registry = provider_accounts::load_claude_registry(&state.core.data_root)
            .await
            .map_err(internal_error)?;
        if !registry.accounts.iter().any(|a| a.id == *account_id) {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "unknown account".to_string(),
                }),
            ));
        }
    }
    provider_accounts::set_active_claude_account(&state.core.data_root, req.account_id)
        .await
        .map_err(bad_request)?;
    restarts::restart_claude_providers_for_auth_change(&state, "claude auth updated").await;
    Ok(Json(
        claude_accounts_response(&state)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn delete_claude_account(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<ClaudeAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::remove_claude_account(&state.core.data_root, &id)
        .await
        .map_err(provider_account_delete_error)?;
    restarts::restart_claude_providers_for_auth_change(&state, "claude auth updated").await;
    Ok(Json(
        claude_accounts_response(&state)
            .await
            .map_err(internal_error)?,
    ))
}
