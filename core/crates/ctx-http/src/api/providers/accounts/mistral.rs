use super::common::{internal_error, provider_account_mutation_error, unknown_account};
use super::*;

pub(crate) async fn mistral_accounts_response(
    state: &Arc<AppState>,
) -> anyhow::Result<MistralAccountsResponse> {
    let registry = crate::daemon::providers::load_mistral_account_registry(state).await?;
    Ok(MistralAccountsResponse {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
    })
}

pub(crate) async fn list_mistral_accounts(
    State(state): State<Arc<AppState>>,
) -> Result<Json<MistralAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    Ok(Json(
        mistral_accounts_response(&state)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn upsert_mistral_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<MistralAccountUpsertReq>,
) -> Result<Json<MistralAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    crate::daemon::providers::upsert_mistral_account(&state, req.label, req.email)
        .await
        .map_err(provider_account_mutation_error)?;
    Ok(Json(
        mistral_accounts_response(&state)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn set_mistral_active_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<MistralActiveAccountReq>,
) -> Result<Json<MistralAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    if let Some(ref account_id) = req.account_id {
        let registry = crate::daemon::providers::load_mistral_account_registry(&state)
            .await
            .map_err(internal_error)?;
        if !registry.accounts.iter().any(|a| a.id == *account_id) {
            return Err(unknown_account());
        }
    }
    crate::daemon::providers::set_active_mistral_account(&state, req.account_id)
        .await
        .map_err(provider_account_mutation_error)?;
    Ok(Json(
        mistral_accounts_response(&state)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn delete_mistral_account(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<MistralAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    crate::daemon::providers::remove_mistral_account(&state, &id)
        .await
        .map_err(provider_account_mutation_error)?;
    Ok(Json(
        mistral_accounts_response(&state)
            .await
            .map_err(internal_error)?,
    ))
}
