use super::common::provider_account_route_error;
use super::*;

pub(crate) async fn list_claude_accounts(
    State(providers): State<ProvidersHandle>,
) -> Result<Json<ClaudeAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let response = providers
        .claude_accounts_response()
        .await
        .map_err(provider_account_route_error)?;
    Ok(Json(response))
}

pub(crate) async fn upsert_claude_account(
    State(providers): State<ProvidersHandle>,
    Json(req): Json<ClaudeAccountUpsertReq>,
) -> Result<Json<ClaudeAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let response = providers
        .add_claude_account_response(req.label, req.setup_token)
        .await
        .map_err(provider_account_route_error)?;
    Ok(Json(response))
}

pub(crate) async fn set_claude_active_account(
    State(providers): State<ProvidersHandle>,
    Json(req): Json<ClaudeActiveAccountReq>,
) -> Result<Json<ClaudeAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let response = providers
        .set_active_claude_account_response(req.account_id)
        .await
        .map_err(provider_account_route_error)?;
    Ok(Json(response))
}

pub(crate) async fn delete_claude_account(
    State(providers): State<ProvidersHandle>,
    Path(id): Path<String>,
) -> Result<Json<ClaudeAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let response = providers
        .delete_claude_account_response(&id)
        .await
        .map_err(provider_account_route_error)?;
    Ok(Json(response))
}
