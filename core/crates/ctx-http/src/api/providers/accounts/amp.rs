use super::common::provider_account_route_error;
use super::*;

pub(crate) async fn list_amp_accounts(
    State(providers): State<ProvidersHandle>,
) -> Result<Json<AmpAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let response = providers
        .amp_accounts_response()
        .await
        .map_err(provider_account_route_error)?;
    Ok(Json(response))
}

pub(crate) async fn upsert_amp_account(
    State(providers): State<ProvidersHandle>,
    Json(req): Json<AmpAccountUpsertReq>,
) -> Result<Json<AmpAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let response = providers
        .upsert_amp_account_response(req.label, req.email)
        .await
        .map_err(provider_account_route_error)?;
    Ok(Json(response))
}

pub(crate) async fn set_amp_active_account(
    State(providers): State<ProvidersHandle>,
    Json(req): Json<AmpActiveAccountReq>,
) -> Result<Json<AmpAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let response = providers
        .set_active_amp_account_response(req.account_id)
        .await
        .map_err(provider_account_route_error)?;
    Ok(Json(response))
}

pub(crate) async fn delete_amp_account(
    State(providers): State<ProvidersHandle>,
    Path(id): Path<String>,
) -> Result<Json<AmpAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let response = providers
        .delete_amp_account_response(&id)
        .await
        .map_err(provider_account_route_error)?;
    Ok(Json(response))
}
