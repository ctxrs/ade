use super::common::{internal_error, provider_account_mutation_error, unknown_account};
use super::*;

pub(crate) async fn qwen_accounts_response(
    state: &Arc<AppState>,
) -> anyhow::Result<QwenAccountsResponse> {
    let registry = crate::daemon::providers::load_qwen_account_registry(state).await?;
    Ok(QwenAccountsResponse {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
    })
}

pub(crate) async fn list_qwen_accounts(
    State(state): State<Arc<AppState>>,
) -> Result<Json<QwenAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    Ok(Json(
        qwen_accounts_response(&state)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn upsert_qwen_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<QwenAccountUpsertReq>,
) -> Result<Json<QwenAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    crate::daemon::providers::add_qwen_account(&state, req.label, req.oauth_creds_json, req.email)
        .await
        .map_err(provider_account_mutation_error)?;
    Ok(Json(
        qwen_accounts_response(&state)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn set_qwen_active_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<QwenActiveAccountReq>,
) -> Result<Json<QwenAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    if let Some(ref account_id) = req.account_id {
        let registry = crate::daemon::providers::load_qwen_account_registry(&state)
            .await
            .map_err(internal_error)?;
        if !registry.accounts.iter().any(|a| a.id == *account_id) {
            return Err(unknown_account());
        }
    }
    crate::daemon::providers::set_active_qwen_account(&state, req.account_id)
        .await
        .map_err(provider_account_mutation_error)?;
    Ok(Json(
        qwen_accounts_response(&state)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn delete_qwen_account(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<QwenAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    crate::daemon::providers::remove_qwen_account(&state, &id)
        .await
        .map_err(provider_account_mutation_error)?;
    Ok(Json(
        qwen_accounts_response(&state)
            .await
            .map_err(internal_error)?,
    ))
}
