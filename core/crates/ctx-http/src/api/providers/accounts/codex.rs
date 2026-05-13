use super::common::{internal_error, provider_account_mutation_error};
use super::*;

#[path = "codex/usage.rs"]
mod usage;

pub(crate) use usage::get_codex_accounts_usage;

pub(crate) async fn codex_accounts_response(
    state: &Arc<AppState>,
) -> anyhow::Result<CodexAccountsResponse> {
    crate::daemon::providers::load_codex_accounts_snapshot(state)
        .await
        .map(codex_accounts_response_from_snapshot)
}

fn codex_accounts_response_from_snapshot(
    snapshot: crate::daemon::providers::CodexAccountsSnapshot,
) -> CodexAccountsResponse {
    CodexAccountsResponse {
        active_account_id: snapshot.active_account_id,
        accounts: snapshot.accounts,
        logins: snapshot.logins,
    }
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
        crate::daemon::providers::probe_host_codex_auth_candidate().await,
    ))
}

pub(crate) async fn import_host_codex_auth(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CodexHostImportReq>,
) -> Result<Json<CodexAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    crate::daemon::providers::import_host_codex_auth(&state, req.label)
        .await
        .map_err(provider_account_mutation_error)?;
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
        let registry = crate::daemon::providers::load_codex_account_registry(&state)
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
    let snapshot = crate::daemon::providers::set_active_codex_account(&state, req.account_id)
        .await
        .map_err(codex_account_set_active_error)?;
    Ok(Json(codex_accounts_response_from_snapshot(snapshot)))
}

pub(crate) async fn delete_codex_account(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<CodexAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let snapshot = crate::daemon::providers::remove_codex_account(&state, &id)
        .await
        .map_err(provider_account_mutation_error)?;
    Ok(Json(codex_accounts_response_from_snapshot(snapshot)))
}

fn codex_account_set_active_error(
    error: crate::daemon::providers::ProviderAccountMutationError,
) -> (StatusCode, Json<ApiErrorResp>) {
    match error {
        crate::daemon::providers::ProviderAccountMutationError::BadRequest(error) => {
            let msg = error.to_string();
            let status = if msg.contains("api_shape=openai_responses")
                || msg.contains("auth_type=bearer")
                || msg.contains("unknown account")
            {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            (status, Json(ApiErrorResp { error: msg }))
        }
        crate::daemon::providers::ProviderAccountMutationError::Delete(error)
        | crate::daemon::providers::ProviderAccountMutationError::Internal(error) => {
            internal_error(error)
        }
    }
}
