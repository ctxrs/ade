use super::common::provider_account_route_error;
use super::*;

pub(crate) async fn list_copilot_accounts(
    State(providers): State<ProvidersHandle>,
) -> Result<Json<CopilotAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let response = providers
        .copilot_accounts_response()
        .await
        .map_err(provider_account_route_error)?;
    Ok(Json(response))
}

pub(crate) async fn upsert_copilot_account(
    State(providers): State<ProvidersHandle>,
    Json(req): Json<CopilotAccountUpsertReq>,
) -> Result<Json<CopilotAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let response = providers
        .add_copilot_account_response(req.label, req.token, req.email)
        .await
        .map_err(provider_account_route_error)?;
    Ok(Json(response))
}

pub(crate) async fn set_copilot_active_account(
    State(providers): State<ProvidersHandle>,
    Json(req): Json<CopilotActiveAccountReq>,
) -> Result<Json<CopilotAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let response = providers
        .set_active_copilot_account_response(req.account_id)
        .await
        .map_err(provider_account_route_error)?;
    Ok(Json(response))
}

pub(crate) async fn delete_copilot_account(
    State(providers): State<ProvidersHandle>,
    Path(id): Path<String>,
) -> Result<Json<CopilotAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let response = providers
        .delete_copilot_account_response(&id)
        .await
        .map_err(provider_account_route_error)?;
    Ok(Json(response))
}
