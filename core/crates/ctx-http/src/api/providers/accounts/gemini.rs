use super::common::provider_account_route_error;
use super::*;

pub(crate) async fn list_gemini_accounts(
    State(providers): State<ProvidersHandle>,
) -> Result<Json<GeminiAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let response = providers
        .gemini_accounts_response()
        .await
        .map_err(provider_account_route_error)?;
    Ok(Json(response))
}

pub(crate) async fn upsert_gemini_account(
    State(providers): State<ProvidersHandle>,
    Json(req): Json<GeminiAccountUpsertReq>,
) -> Result<Json<GeminiAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let response = providers
        .add_gemini_account_response(
            req.label,
            req.oauth_creds_json,
            req.google_accounts_json,
            req.email,
        )
        .await
        .map_err(provider_account_route_error)?;
    Ok(Json(response))
}

pub(crate) async fn set_gemini_active_account(
    State(providers): State<ProvidersHandle>,
    Json(req): Json<GeminiActiveAccountReq>,
) -> Result<Json<GeminiAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let response = providers
        .set_active_gemini_account_response(req.account_id)
        .await
        .map_err(provider_account_route_error)?;
    Ok(Json(response))
}

pub(crate) async fn delete_gemini_account(
    State(providers): State<ProvidersHandle>,
    Path(id): Path<String>,
) -> Result<Json<GeminiAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let response = providers
        .delete_gemini_account_response(&id)
        .await
        .map_err(provider_account_route_error)?;
    Ok(Json(response))
}
