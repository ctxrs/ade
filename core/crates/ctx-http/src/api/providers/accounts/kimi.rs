use super::common::provider_account_route_error;
use super::*;

pub(crate) async fn list_kimi_accounts(
    State(providers): State<ProvidersHandle>,
) -> Result<Json<KimiAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let response = providers
        .kimi_accounts_response()
        .await
        .map_err(provider_account_route_error)?;
    Ok(Json(response))
}

pub(crate) async fn upsert_kimi_account(
    State(providers): State<ProvidersHandle>,
    Json(req): Json<KimiAccountUpsertReq>,
) -> Result<Json<KimiAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let response = providers
        .add_kimi_account_response(
            req.label,
            req.provider,
            req.credentials_json,
            req.config_toml,
            req.email,
        )
        .await
        .map_err(provider_account_route_error)?;
    Ok(Json(response))
}

pub(crate) async fn set_kimi_active_account(
    State(providers): State<ProvidersHandle>,
    Json(req): Json<KimiActiveAccountReq>,
) -> Result<Json<KimiAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let response = providers
        .set_active_kimi_account_response(req.account_id)
        .await
        .map_err(provider_account_route_error)?;
    Ok(Json(response))
}

pub(crate) async fn delete_kimi_account(
    State(providers): State<ProvidersHandle>,
    Path(id): Path<String>,
) -> Result<Json<KimiAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let response = providers
        .delete_kimi_account_response(&id)
        .await
        .map_err(provider_account_route_error)?;
    Ok(Json(response))
}
