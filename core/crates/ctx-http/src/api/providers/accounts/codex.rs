use super::common::{bad_request, internal_error, provider_account_delete_error};
use super::*;

#[path = "codex/usage.rs"]
mod usage;

pub(crate) use usage::get_codex_accounts_usage;

pub(crate) async fn codex_accounts_response(
    state: &Arc<AppState>,
) -> anyhow::Result<CodexAccountsResponse> {
    let registry = provider_accounts::load_codex_registry(&state.core.data_root).await?;
    let logins = state
        .providers
        .with_codex_login_sessions(|map| map.values().cloned().collect::<Vec<_>>())
        .await;
    Ok(CodexAccountsResponse {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
        logins,
    })
}

pub(crate) async fn list_codex_accounts(
    State(state): State<Arc<AppState>>,
) -> Result<Json<CodexAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    Ok(Json(
        codex_accounts_response(&state)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn probe_host_codex_import(
    State(_state): State<Arc<AppState>>,
) -> Result<Json<provider_accounts::CodexHostImportProbe>, (StatusCode, Json<ApiErrorResp>)> {
    Ok(Json(
        provider_accounts::probe_host_codex_auth_candidate().await,
    ))
}

pub(crate) async fn import_host_codex_auth(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CodexHostImportReq>,
) -> Result<Json<CodexAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::import_host_codex_auth_to_secret_store(&state.core.data_root, req.label)
        .await
        .map_err(bad_request)?;
    restarts::restart_codex_providers_for_auth_change(&state, "codex auth updated")
        .await
        .map_err(internal_error)?;
    Ok(Json(
        codex_accounts_response(&state)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn set_codex_active_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CodexActiveAccountReq>,
) -> Result<Json<CodexAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    if let Some(ref account_id) = req.account_id {
        let registry = provider_accounts::load_codex_registry(&state.core.data_root)
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
    let registry =
        provider_accounts::set_active_codex_account(&state.core.data_root, req.account_id)
            .await
            .map_err(|e| {
                let msg = e.to_string();
                let status = if msg.contains("api_shape=openai_responses")
                    || msg.contains("auth_type=bearer")
                    || msg.contains("unknown account")
                {
                    StatusCode::BAD_REQUEST
                } else {
                    StatusCode::INTERNAL_SERVER_ERROR
                };
                (status, Json(ApiErrorResp { error: msg }))
            })?;
    restarts::restart_codex_providers_for_auth_change(&state, "codex auth updated")
        .await
        .map_err(internal_error)?;
    let logins = state
        .providers
        .with_codex_login_sessions(|map| map.values().cloned().collect::<Vec<_>>())
        .await;
    Ok(Json(CodexAccountsResponse {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
        logins,
    }))
}

pub(crate) async fn delete_codex_account(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<CodexAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let registry = provider_accounts::remove_codex_account(&state.core.data_root, &id)
        .await
        .map_err(provider_account_delete_error)?;
    restarts::restart_codex_providers_for_auth_change(&state, "codex auth updated")
        .await
        .map_err(internal_error)?;
    let logins = state
        .providers
        .with_codex_login_sessions(|map| {
            map.remove(&id);
            map.values().cloned().collect::<Vec<_>>()
        })
        .await;
    Ok(Json(CodexAccountsResponse {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
        logins,
    }))
}
