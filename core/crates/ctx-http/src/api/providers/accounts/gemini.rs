use super::common::{internal_error, provider_account_mutation_error, unknown_account};
use super::*;

pub(crate) async fn gemini_accounts_response(
    providers: &ProvidersHandle,
) -> anyhow::Result<GeminiAccountsResponse> {
    let registry = providers.load_gemini_account_registry().await?;
    Ok(GeminiAccountsResponse {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
    })
}

pub(crate) async fn list_gemini_accounts(
    State(providers): State<ProvidersHandle>,
) -> Result<Json<GeminiAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    Ok(Json(
        gemini_accounts_response(&providers)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn upsert_gemini_account(
    State(providers): State<ProvidersHandle>,
    Json(req): Json<GeminiAccountUpsertReq>,
) -> Result<Json<GeminiAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    providers
        .add_gemini_account(
            req.label,
            req.oauth_creds_json,
            req.google_accounts_json,
            req.email,
        )
        .await
        .map_err(provider_account_mutation_error)?;
    Ok(Json(
        gemini_accounts_response(&providers)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn set_gemini_active_account(
    State(providers): State<ProvidersHandle>,
    Json(req): Json<GeminiActiveAccountReq>,
) -> Result<Json<GeminiAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    if let Some(ref account_id) = req.account_id {
        let registry = providers
            .load_gemini_account_registry()
            .await
            .map_err(internal_error)?;
        if !registry.accounts.iter().any(|a| a.id == *account_id) {
            return Err(unknown_account());
        }
    }
    providers
        .set_active_gemini_account(req.account_id)
        .await
        .map_err(provider_account_mutation_error)?;
    Ok(Json(
        gemini_accounts_response(&providers)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn delete_gemini_account(
    State(providers): State<ProvidersHandle>,
    Path(id): Path<String>,
) -> Result<Json<GeminiAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    providers
        .remove_gemini_account(&id)
        .await
        .map_err(provider_account_mutation_error)?;
    Ok(Json(
        gemini_accounts_response(&providers)
            .await
            .map_err(internal_error)?,
    ))
}
