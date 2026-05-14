use super::common::{internal_error, provider_account_mutation_error};
use super::*;

pub(crate) async fn claude_accounts_response(
    providers: &ProvidersHandle,
) -> anyhow::Result<ClaudeAccountsResponse> {
    let registry = providers.load_claude_account_registry().await?;
    Ok(ClaudeAccountsResponse {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
    })
}

pub(crate) async fn list_claude_accounts(
    State(providers): State<ProvidersHandle>,
) -> Result<Json<ClaudeAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    Ok(Json(
        claude_accounts_response(&providers)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn upsert_claude_account(
    State(providers): State<ProvidersHandle>,
    Json(req): Json<ClaudeAccountUpsertReq>,
) -> Result<Json<ClaudeAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    providers
        .add_claude_account(req.label, req.setup_token)
        .await
        .map_err(provider_account_mutation_error)?;
    Ok(Json(
        claude_accounts_response(&providers)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn set_claude_active_account(
    State(providers): State<ProvidersHandle>,
    Json(req): Json<ClaudeActiveAccountReq>,
) -> Result<Json<ClaudeAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    if let Some(ref account_id) = req.account_id {
        let registry = providers
            .load_claude_account_registry()
            .await
            .map_err(internal_error)?;
        if !registry.accounts.iter().any(|a| a.id == *account_id) {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "unknown account".to_string(),
                }),
            ));
        }
    }
    providers
        .set_active_claude_account(req.account_id)
        .await
        .map_err(provider_account_mutation_error)?;
    Ok(Json(
        claude_accounts_response(&providers)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn delete_claude_account(
    State(providers): State<ProvidersHandle>,
    Path(id): Path<String>,
) -> Result<Json<ClaudeAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    providers
        .remove_claude_account(&id)
        .await
        .map_err(provider_account_mutation_error)?;
    Ok(Json(
        claude_accounts_response(&providers)
            .await
            .map_err(internal_error)?,
    ))
}
