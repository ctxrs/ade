use super::*;

mod app_server;
mod completion;
mod process;

pub(crate) use completion::complete_codex_login;
#[cfg(test)]
use process::persist_successful_codex_login;
use process::{monitor_codex_login, start_codex_login_process};

#[derive(Debug, Deserialize)]
pub(crate) struct CodexLoginStartReq {
    label: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct CodexLoginStartResp {
    account_id: String,
    auth_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected_callback_url: Option<String>,
    completion_token: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CodexLoginCompleteReq {
    callback_url: String,
    completion_token: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct CodexLoginCompleteResp {
    accepted: bool,
    status_code: u16,
}

pub(crate) async fn start_codex_login(
    State(state): State<Arc<AppState>>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Json(req): Json<CodexLoginStartReq>,
) -> Result<Json<CodexLoginStartResp>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    let account_id = uuid::Uuid::new_v4().to_string();
    let label = provider_accounts::normalize_label(req.label, &account_id);
    let account_dir =
        provider_accounts::ensure_codex_account_dir(&state.core.data_root, &account_id)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: e.to_string(),
                    }),
                )
            })?;
    let (cfg, managed_config_error) =
        crate::api::provider_launch::load_managed_agent_server_config_with_error(
            &state.core.data_root,
        )
        .await;
    if let Some(error) = managed_config_error {
        let _ = tokio::fs::remove_dir_all(&account_dir).await;
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp { error }),
        ));
    }
    let codex_bin = crate::daemon::installer::require_codex_cli_command_path_for_target(
        &cfg,
        Some(ctx_provider_install::install_state::InstallTarget::Host),
    )
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: e.to_string(),
            }),
        )
    })?;
    let login = match start_codex_login_process(&account_dir, &codex_bin).await {
        Ok(login) => login,
        Err(e) => {
            let _ = tokio::fs::remove_dir_all(&account_dir).await;
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            ));
        }
    };
    let expected_callback_url = expected_callback_from_auth_url(&login.auth_url);
    let completion_token = uuid::Uuid::new_v4().to_string();
    let auth_url = login.auth_url.clone();
    let status = provider_accounts::CodexLoginStatus {
        account_id: account_id.clone(),
        auth_url: auth_url.clone(),
        expected_callback_url: expected_callback_url.clone(),
        completion_token: Some(completion_token.clone()),
        status: "pending".to_string(),
        error: None,
    };
    {
        let mut map = state.providers.codex_login_sessions.lock().await;
        map.insert(account_id.clone(), status);
    }
    let state_clone = Arc::clone(&state);
    let account_id_for_task = account_id.clone();
    tokio::spawn(async move {
        monitor_codex_login(state_clone, account_id_for_task, label, login).await;
    });

    Ok(Json(CodexLoginStartResp {
        account_id,
        auth_url,
        expected_callback_url,
        completion_token,
    }))
}

pub(crate) async fn get_codex_login(
    State(state): State<Arc<AppState>>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Path(id): Path<String>,
) -> Result<Json<provider_accounts::CodexLoginStatus>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    let map = state.providers.codex_login_sessions.lock().await;
    let status = map.get(&id).cloned().ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "login not found".to_string(),
            }),
        )
    })?;
    Ok(Json(status))
}

#[cfg(test)]
mod tests;
