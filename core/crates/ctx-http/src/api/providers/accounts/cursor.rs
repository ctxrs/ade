use super::common::{bad_request, internal_error, provider_account_delete_error, unknown_account};
use super::*;

pub(crate) async fn cursor_accounts_response(
    state: &Arc<AppState>,
) -> anyhow::Result<CursorAccountsResponse> {
    let registry = provider_accounts::load_cursor_registry(&state.core.data_root).await?;
    Ok(CursorAccountsResponse {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
    })
}

pub(crate) async fn list_cursor_accounts(
    State(state): State<Arc<AppState>>,
) -> Result<Json<CursorAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    Ok(Json(
        cursor_accounts_response(&state)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn upsert_cursor_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CursorAccountUpsertReq>,
) -> Result<Json<CursorAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::add_cursor_account(&state.core.data_root, req.label, req.token, req.email)
        .await
        .map_err(bad_request)?;
    restarts::restart_cursor_providers_for_auth_change(&state, "cursor auth updated").await;
    Ok(Json(
        cursor_accounts_response(&state)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn set_cursor_active_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CursorActiveAccountReq>,
) -> Result<Json<CursorAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    if let Some(ref account_id) = req.account_id {
        let registry = provider_accounts::load_cursor_registry(&state.core.data_root)
            .await
            .map_err(internal_error)?;
        if !registry.accounts.iter().any(|a| a.id == *account_id) {
            return Err(unknown_account());
        }
    }
    provider_accounts::set_active_cursor_account(&state.core.data_root, req.account_id)
        .await
        .map_err(bad_request)?;
    restarts::restart_cursor_providers_for_auth_change(&state, "cursor auth updated").await;
    Ok(Json(
        cursor_accounts_response(&state)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn delete_cursor_account(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<CursorAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::remove_cursor_account(&state.core.data_root, &id)
        .await
        .map_err(provider_account_delete_error)?;
    restarts::restart_cursor_providers_for_auth_change(&state, "cursor auth updated").await;
    Ok(Json(
        cursor_accounts_response(&state)
            .await
            .map_err(internal_error)?,
    ))
}
