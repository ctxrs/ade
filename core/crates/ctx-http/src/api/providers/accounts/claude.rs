use super::common::{internal_error, provider_account_mutation_error};
use super::*;

pub(crate) async fn claude_accounts_response(
    state: &Arc<AppState>,
) -> anyhow::Result<ClaudeAccountsResponse> {
    let registry = crate::daemon::providers::load_claude_account_registry(state).await?;
    Ok(ClaudeAccountsResponse {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
    })
}

pub(crate) async fn list_claude_accounts(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ClaudeAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    Ok(Json(
        claude_accounts_response(&state)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn upsert_claude_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ClaudeAccountUpsertReq>,
) -> Result<Json<ClaudeAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    crate::daemon::providers::add_claude_account(&state, req.label, req.setup_token)
        .await
        .map_err(provider_account_mutation_error)?;
    Ok(Json(
        claude_accounts_response(&state)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn set_claude_active_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ClaudeActiveAccountReq>,
) -> Result<Json<ClaudeAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    if let Some(ref account_id) = req.account_id {
        let registry = crate::daemon::providers::load_claude_account_registry(&state)
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
    crate::daemon::providers::set_active_claude_account(&state, req.account_id)
        .await
        .map_err(provider_account_mutation_error)?;
    Ok(Json(
        claude_accounts_response(&state)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn delete_claude_account(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<ClaudeAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    crate::daemon::providers::remove_claude_account(&state, &id)
        .await
        .map_err(provider_account_mutation_error)?;
    Ok(Json(
        claude_accounts_response(&state)
            .await
            .map_err(internal_error)?,
    ))
}
