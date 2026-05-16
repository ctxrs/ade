use super::common::provider_account_route_error;
use super::*;

pub(crate) async fn list_mistral_accounts(
    State(providers): State<ProvidersHandle>,
) -> Result<Json<MistralAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let response = providers
        .mistral_accounts_response()
        .await
        .map_err(provider_account_route_error)?;
    Ok(Json(response))
}

pub(crate) async fn upsert_mistral_account(
    State(providers): State<ProvidersHandle>,
    Json(req): Json<MistralAccountUpsertReq>,
) -> Result<Json<MistralAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let response = providers
        .upsert_mistral_account_response(req.label, req.email)
        .await
        .map_err(provider_account_route_error)?;
    Ok(Json(response))
}

pub(crate) async fn set_mistral_active_account(
    State(providers): State<ProvidersHandle>,
    Json(req): Json<MistralActiveAccountReq>,
) -> Result<Json<MistralAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let response = providers
        .set_active_mistral_account_response(req.account_id)
        .await
        .map_err(provider_account_route_error)?;
    Ok(Json(response))
}

pub(crate) async fn delete_mistral_account(
    State(providers): State<ProvidersHandle>,
    Path(id): Path<String>,
) -> Result<Json<MistralAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let response = providers
        .delete_mistral_account_response(&id)
        .await
        .map_err(provider_account_route_error)?;
    Ok(Json(response))
}
