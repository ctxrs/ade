use super::common::provider_account_route_error;
use super::*;

pub(crate) async fn list_cursor_accounts(
    State(providers): State<ProvidersHandle>,
) -> Result<Json<CursorAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let response = providers
        .cursor_accounts_response()
        .await
        .map_err(provider_account_route_error)?;
    Ok(Json(response))
}

pub(crate) async fn upsert_cursor_account(
    State(providers): State<ProvidersHandle>,
    Json(req): Json<CursorAccountUpsertReq>,
) -> Result<Json<CursorAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let response = providers
        .add_cursor_account_response(req.label, req.token, req.email)
        .await
        .map_err(provider_account_route_error)?;
    Ok(Json(response))
}

pub(crate) async fn set_cursor_active_account(
    State(providers): State<ProvidersHandle>,
    Json(req): Json<CursorActiveAccountReq>,
) -> Result<Json<CursorAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let response = providers
        .set_active_cursor_account_response(req.account_id)
        .await
        .map_err(provider_account_route_error)?;
    Ok(Json(response))
}

pub(crate) async fn delete_cursor_account(
    State(providers): State<ProvidersHandle>,
    Path(id): Path<String>,
) -> Result<Json<CursorAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let response = providers
        .delete_cursor_account_response(&id)
        .await
        .map_err(provider_account_route_error)?;
    Ok(Json(response))
}
