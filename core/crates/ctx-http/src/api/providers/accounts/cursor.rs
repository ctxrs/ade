use super::common::{internal_error, provider_account_mutation_error, unknown_account};
use super::*;

pub(crate) async fn cursor_accounts_response(
    state: &Arc<AppState>,
) -> anyhow::Result<CursorAccountsResponse> {
    let registry = crate::daemon::providers::load_cursor_account_registry(state).await?;
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
    crate::daemon::providers::add_cursor_account(&state, req.label, req.token, req.email)
        .await
        .map_err(provider_account_mutation_error)?;
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
        let registry = crate::daemon::providers::load_cursor_account_registry(&state)
            .await
            .map_err(internal_error)?;
        if !registry.accounts.iter().any(|a| a.id == *account_id) {
            return Err(unknown_account());
        }
    }
    crate::daemon::providers::set_active_cursor_account(&state, req.account_id)
        .await
        .map_err(provider_account_mutation_error)?;
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
    crate::daemon::providers::remove_cursor_account(&state, &id)
        .await
        .map_err(provider_account_mutation_error)?;
    Ok(Json(
        cursor_accounts_response(&state)
            .await
            .map_err(internal_error)?,
    ))
}
