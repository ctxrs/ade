use super::common::{internal_error, provider_account_route_error};
use super::*;

#[path = "codex/usage.rs"]
mod usage;

pub(crate) use usage::get_codex_accounts_usage;

pub(crate) async fn list_codex_accounts(
    State(providers): State<ProvidersHandle>,
) -> Result<Json<CodexAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let response = providers
        .codex_accounts_response()
        .await
        .map_err(provider_account_route_error)?;
    Ok(Json(response))
}

pub(crate) async fn probe_host_codex_import(
    State(_providers): State<ProvidersHandle>,
) -> Result<Json<provider_accounts::CodexHostImportProbe>, (StatusCode, Json<ApiErrorResp>)> {
    Ok(Json(
        ctx_daemon::daemon::providers::probe_host_codex_auth_candidate().await,
    ))
}

pub(crate) async fn import_host_codex_auth(
    State(providers): State<ProvidersHandle>,
    Json(req): Json<CodexHostImportReq>,
) -> Result<Json<CodexAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let response = providers
        .import_host_codex_auth_response(req.label)
        .await
        .map_err(provider_account_route_error)?;
    Ok(Json(response))
}

pub(crate) async fn set_codex_active_account(
    State(providers): State<ProvidersHandle>,
    Json(req): Json<CodexActiveAccountReq>,
) -> Result<Json<CodexAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let response = providers
        .set_active_codex_account_response(req.account_id)
        .await
        .map_err(provider_account_route_error)?;
    Ok(Json(response))
}

pub(crate) async fn delete_codex_account(
    State(providers): State<ProvidersHandle>,
    Path(id): Path<String>,
) -> Result<Json<CodexAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let response = providers
        .delete_codex_account_response(&id)
        .await
        .map_err(provider_account_route_error)?;
    Ok(Json(response))
}
