use super::common::{bad_request, internal_error, provider_account_delete_error, unknown_account};
use super::*;

pub(crate) async fn copilot_accounts_response(
    state: &Arc<AppState>,
) -> anyhow::Result<CopilotAccountsResponse> {
    let registry = provider_accounts::load_copilot_registry(&state.core.data_root).await?;
    Ok(CopilotAccountsResponse {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
    })
}

pub(crate) async fn list_copilot_accounts(
    State(state): State<Arc<AppState>>,
) -> Result<Json<CopilotAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    Ok(Json(
        copilot_accounts_response(&state)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn upsert_copilot_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CopilotAccountUpsertReq>,
) -> Result<Json<CopilotAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::add_copilot_account(&state.core.data_root, req.label, req.token, req.email)
        .await
        .map_err(bad_request)?;
    restarts::restart_copilot_providers_for_auth_change(&state, "copilot auth updated")
        .await
        .map_err(internal_error)?;
    Ok(Json(
        copilot_accounts_response(&state)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn set_copilot_active_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CopilotActiveAccountReq>,
) -> Result<Json<CopilotAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    if let Some(ref account_id) = req.account_id {
        let registry = provider_accounts::load_copilot_registry(&state.core.data_root)
            .await
            .map_err(internal_error)?;
        if !registry.accounts.iter().any(|a| a.id == *account_id) {
            return Err(unknown_account());
        }
    }
    provider_accounts::set_active_copilot_account(&state.core.data_root, req.account_id)
        .await
        .map_err(bad_request)?;
    restarts::restart_copilot_providers_for_auth_change(&state, "copilot auth updated")
        .await
        .map_err(internal_error)?;
    Ok(Json(
        copilot_accounts_response(&state)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn delete_copilot_account(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<CopilotAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::remove_copilot_account(&state.core.data_root, &id)
        .await
        .map_err(provider_account_delete_error)?;
    restarts::restart_copilot_providers_for_auth_change(&state, "copilot auth updated")
        .await
        .map_err(internal_error)?;
    Ok(Json(
        copilot_accounts_response(&state)
            .await
            .map_err(internal_error)?,
    ))
}
