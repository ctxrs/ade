use super::common::provider_account_route_error;
use super::*;

pub(crate) async fn list_qwen_accounts(
    State(providers): State<ProvidersHandle>,
) -> Result<Json<QwenAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let response = providers
        .qwen_accounts_response()
        .await
        .map_err(provider_account_route_error)?;
    Ok(Json(response))
}

pub(crate) async fn upsert_qwen_account(
    State(providers): State<ProvidersHandle>,
    Json(req): Json<QwenAccountUpsertReq>,
) -> Result<Json<QwenAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let response = providers
        .add_qwen_account_response(req.label, req.oauth_creds_json, req.email)
        .await
        .map_err(provider_account_route_error)?;
    Ok(Json(response))
}

pub(crate) async fn set_qwen_active_account(
    State(providers): State<ProvidersHandle>,
    Json(req): Json<QwenActiveAccountReq>,
) -> Result<Json<QwenAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let response = providers
        .set_active_qwen_account_response(req.account_id)
        .await
        .map_err(provider_account_route_error)?;
    Ok(Json(response))
}

pub(crate) async fn delete_qwen_account(
    State(providers): State<ProvidersHandle>,
    Path(id): Path<String>,
) -> Result<Json<QwenAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let response = providers
        .delete_qwen_account_response(&id)
        .await
        .map_err(provider_account_route_error)?;
    Ok(Json(response))
}
