use super::common::{internal_error, provider_account_mutation_error, unknown_account};
use super::*;

pub(crate) async fn qwen_accounts_response(
    providers: &ProvidersHandle,
) -> anyhow::Result<QwenAccountsResponse> {
    let registry = providers.load_qwen_account_registry().await?;
    Ok(QwenAccountsResponse {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
    })
}

pub(crate) async fn list_qwen_accounts(
    State(providers): State<ProvidersHandle>,
) -> Result<Json<QwenAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    Ok(Json(
        qwen_accounts_response(&providers)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn upsert_qwen_account(
    State(providers): State<ProvidersHandle>,
    Json(req): Json<QwenAccountUpsertReq>,
) -> Result<Json<QwenAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    providers
        .add_qwen_account(req.label, req.oauth_creds_json, req.email)
        .await
        .map_err(provider_account_mutation_error)?;
    Ok(Json(
        qwen_accounts_response(&providers)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn set_qwen_active_account(
    State(providers): State<ProvidersHandle>,
    Json(req): Json<QwenActiveAccountReq>,
) -> Result<Json<QwenAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    if let Some(ref account_id) = req.account_id {
        let registry = providers
            .load_qwen_account_registry()
            .await
            .map_err(internal_error)?;
        if !registry.accounts.iter().any(|a| a.id == *account_id) {
            return Err(unknown_account());
        }
    }
    providers
        .set_active_qwen_account(req.account_id)
        .await
        .map_err(provider_account_mutation_error)?;
    Ok(Json(
        qwen_accounts_response(&providers)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn delete_qwen_account(
    State(providers): State<ProvidersHandle>,
    Path(id): Path<String>,
) -> Result<Json<QwenAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    providers
        .remove_qwen_account(&id)
        .await
        .map_err(provider_account_mutation_error)?;
    Ok(Json(
        qwen_accounts_response(&providers)
            .await
            .map_err(internal_error)?,
    ))
}
