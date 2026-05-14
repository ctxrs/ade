use super::common::{internal_error, provider_account_mutation_error, unknown_account};
use super::*;

pub(crate) async fn amp_accounts_response(
    providers: &ProvidersHandle,
) -> anyhow::Result<AmpAccountsResponse> {
    let registry = providers
        .ensure_amp_account_registry_from_runtime_auth()
        .await?;
    Ok(AmpAccountsResponse {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
    })
}

pub(crate) async fn list_amp_accounts(
    State(providers): State<ProvidersHandle>,
) -> Result<Json<AmpAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let response = amp_accounts_response(&providers)
        .await
        .map_err(internal_error)?;
    Ok(Json(response))
}

pub(crate) async fn upsert_amp_account(
    State(providers): State<ProvidersHandle>,
    Json(req): Json<AmpAccountUpsertReq>,
) -> Result<Json<AmpAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    providers
        .upsert_amp_account(req.label, req.email)
        .await
        .map_err(provider_account_mutation_error)?;
    Ok(Json(
        amp_accounts_response(&providers)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn set_amp_active_account(
    State(providers): State<ProvidersHandle>,
    Json(req): Json<AmpActiveAccountReq>,
) -> Result<Json<AmpAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    if let Some(ref account_id) = req.account_id {
        let registry = providers
            .load_amp_account_registry()
            .await
            .map_err(internal_error)?;
        if !registry.accounts.iter().any(|a| a.id == *account_id) {
            return Err(unknown_account());
        }
    }
    providers
        .set_active_amp_account(req.account_id)
        .await
        .map_err(provider_account_mutation_error)?;
    let response = amp_accounts_response(&providers)
        .await
        .map_err(internal_error)?;
    Ok(Json(response))
}

pub(crate) async fn delete_amp_account(
    State(providers): State<ProvidersHandle>,
    Path(id): Path<String>,
) -> Result<Json<AmpAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    providers
        .remove_amp_account(&id)
        .await
        .map_err(provider_account_mutation_error)?;
    let response = amp_accounts_response(&providers)
        .await
        .map_err(internal_error)?;
    Ok(Json(response))
}
