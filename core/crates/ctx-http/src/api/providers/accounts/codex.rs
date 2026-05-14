use super::common::{internal_error, provider_account_mutation_error};
use super::*;

#[path = "codex/usage.rs"]
mod usage;

pub(crate) use usage::get_codex_accounts_usage;

pub(crate) async fn codex_accounts_response(
    providers: &ProvidersHandle,
) -> anyhow::Result<CodexAccountsResponse> {
    providers
        .load_codex_accounts_snapshot()
        .await
        .map(codex_accounts_response_from_snapshot)
}

fn codex_accounts_response_from_snapshot(
    snapshot: ctx_daemon::daemon::providers::CodexAccountsSnapshot,
) -> CodexAccountsResponse {
    CodexAccountsResponse {
        active_account_id: snapshot.active_account_id,
        accounts: snapshot.accounts,
        logins: snapshot.logins,
    }
}

pub(crate) async fn list_codex_accounts(
    State(providers): State<ProvidersHandle>,
) -> Result<Json<CodexAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    Ok(Json(
        codex_accounts_response(&providers)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn probe_host_codex_import(
    State(_providers): State<ProvidersHandle>,
) -> Result<Json<provider_accounts::CodexHostImportProbe>, (StatusCode, Json<ApiErrorResp>)> {
    Ok(Json(
        ctx_daemon::daemon::providers::probe_host_codex_auth_candidate().await,
    ))
}

pub(crate) async fn import_host_codex_auth(
    State(providers): State<ProvidersHandle>,
    Json(req): Json<CodexHostImportReq>,
) -> Result<Json<CodexAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    providers
        .import_host_codex_auth(req.label)
        .await
        .map_err(provider_account_mutation_error)?;
    Ok(Json(
        codex_accounts_response(&providers)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn set_codex_active_account(
    State(providers): State<ProvidersHandle>,
    Json(req): Json<CodexActiveAccountReq>,
) -> Result<Json<CodexAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    if let Some(ref account_id) = req.account_id {
        let registry = providers
            .load_codex_account_registry()
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
    let snapshot = providers
        .set_active_codex_account(req.account_id)
        .await
        .map_err(codex_account_set_active_error)?;
    Ok(Json(codex_accounts_response_from_snapshot(snapshot)))
}

pub(crate) async fn delete_codex_account(
    State(providers): State<ProvidersHandle>,
    Path(id): Path<String>,
) -> Result<Json<CodexAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let snapshot = providers
        .remove_codex_account(&id)
        .await
        .map_err(provider_account_mutation_error)?;
    Ok(Json(codex_accounts_response_from_snapshot(snapshot)))
}

fn codex_account_set_active_error(
    error: ctx_daemon::daemon::providers::ProviderAccountMutationError,
) -> (StatusCode, Json<ApiErrorResp>) {
    match error {
        ctx_daemon::daemon::providers::ProviderAccountMutationError::BadRequest(error) => {
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
        ctx_daemon::daemon::providers::ProviderAccountMutationError::Delete(error)
        | ctx_daemon::daemon::providers::ProviderAccountMutationError::Internal(error) => {
            internal_error(error)
        }
    }
}
