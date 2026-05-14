use super::common::{internal_error, provider_account_mutation_error, unknown_account};
use super::*;

pub(crate) async fn mistral_accounts_response(
    providers: &ProvidersHandle,
) -> anyhow::Result<MistralAccountsResponse> {
    let registry = providers.load_mistral_account_registry().await?;
    Ok(MistralAccountsResponse {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
    })
}

pub(crate) async fn list_mistral_accounts(
    State(providers): State<ProvidersHandle>,
) -> Result<Json<MistralAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    Ok(Json(
        mistral_accounts_response(&providers)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn upsert_mistral_account(
    State(providers): State<ProvidersHandle>,
    Json(req): Json<MistralAccountUpsertReq>,
) -> Result<Json<MistralAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    providers
        .upsert_mistral_account(req.label, req.email)
        .await
        .map_err(provider_account_mutation_error)?;
    Ok(Json(
        mistral_accounts_response(&providers)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn set_mistral_active_account(
    State(providers): State<ProvidersHandle>,
    Json(req): Json<MistralActiveAccountReq>,
) -> Result<Json<MistralAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    if let Some(ref account_id) = req.account_id {
        let registry = providers
            .load_mistral_account_registry()
            .await
            .map_err(internal_error)?;
        if !registry.accounts.iter().any(|a| a.id == *account_id) {
            return Err(unknown_account());
        }
    }
    providers
        .set_active_mistral_account(req.account_id)
        .await
        .map_err(provider_account_mutation_error)?;
    Ok(Json(
        mistral_accounts_response(&providers)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn delete_mistral_account(
    State(providers): State<ProvidersHandle>,
    Path(id): Path<String>,
) -> Result<Json<MistralAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    providers
        .remove_mistral_account(&id)
        .await
        .map_err(provider_account_mutation_error)?;
    Ok(Json(
        mistral_accounts_response(&providers)
            .await
            .map_err(internal_error)?,
    ))
}
