use super::common::{internal_error, provider_account_mutation_error, unknown_account};
use super::*;

pub(crate) async fn copilot_accounts_response(
    providers: &ProvidersHandle,
) -> anyhow::Result<CopilotAccountsResponse> {
    let registry = providers.load_copilot_account_registry().await?;
    Ok(CopilotAccountsResponse {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
    })
}

pub(crate) async fn list_copilot_accounts(
    State(providers): State<ProvidersHandle>,
) -> Result<Json<CopilotAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    Ok(Json(
        copilot_accounts_response(&providers)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn upsert_copilot_account(
    State(providers): State<ProvidersHandle>,
    Json(req): Json<CopilotAccountUpsertReq>,
) -> Result<Json<CopilotAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    providers
        .add_copilot_account(req.label, req.token, req.email)
        .await
        .map_err(provider_account_mutation_error)?;
    Ok(Json(
        copilot_accounts_response(&providers)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn set_copilot_active_account(
    State(providers): State<ProvidersHandle>,
    Json(req): Json<CopilotActiveAccountReq>,
) -> Result<Json<CopilotAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    if let Some(ref account_id) = req.account_id {
        let registry = providers
            .load_copilot_account_registry()
            .await
            .map_err(internal_error)?;
        if !registry.accounts.iter().any(|a| a.id == *account_id) {
            return Err(unknown_account());
        }
    }
    providers
        .set_active_copilot_account(req.account_id)
        .await
        .map_err(provider_account_mutation_error)?;
    Ok(Json(
        copilot_accounts_response(&providers)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn delete_copilot_account(
    State(providers): State<ProvidersHandle>,
    Path(id): Path<String>,
) -> Result<Json<CopilotAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    providers
        .remove_copilot_account(&id)
        .await
        .map_err(provider_account_mutation_error)?;
    Ok(Json(
        copilot_accounts_response(&providers)
            .await
            .map_err(internal_error)?,
    ))
}
