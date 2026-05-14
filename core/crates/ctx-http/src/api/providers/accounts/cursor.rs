use super::common::{internal_error, provider_account_mutation_error, unknown_account};
use super::*;

pub(crate) async fn cursor_accounts_response(
    providers: &ProvidersHandle,
) -> anyhow::Result<CursorAccountsResponse> {
    let registry = providers.load_cursor_account_registry().await?;
    Ok(CursorAccountsResponse {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
    })
}

pub(crate) async fn list_cursor_accounts(
    State(providers): State<ProvidersHandle>,
) -> Result<Json<CursorAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    Ok(Json(
        cursor_accounts_response(&providers)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn upsert_cursor_account(
    State(providers): State<ProvidersHandle>,
    Json(req): Json<CursorAccountUpsertReq>,
) -> Result<Json<CursorAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    providers
        .add_cursor_account(req.label, req.token, req.email)
        .await
        .map_err(provider_account_mutation_error)?;
    Ok(Json(
        cursor_accounts_response(&providers)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn set_cursor_active_account(
    State(providers): State<ProvidersHandle>,
    Json(req): Json<CursorActiveAccountReq>,
) -> Result<Json<CursorAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    if let Some(ref account_id) = req.account_id {
        let registry = providers
            .load_cursor_account_registry()
            .await
            .map_err(internal_error)?;
        if !registry.accounts.iter().any(|a| a.id == *account_id) {
            return Err(unknown_account());
        }
    }
    providers
        .set_active_cursor_account(req.account_id)
        .await
        .map_err(provider_account_mutation_error)?;
    Ok(Json(
        cursor_accounts_response(&providers)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn delete_cursor_account(
    State(providers): State<ProvidersHandle>,
    Path(id): Path<String>,
) -> Result<Json<CursorAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    providers
        .remove_cursor_account(&id)
        .await
        .map_err(provider_account_mutation_error)?;
    Ok(Json(
        cursor_accounts_response(&providers)
            .await
            .map_err(internal_error)?,
    ))
}
