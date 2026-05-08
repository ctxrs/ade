use super::*;

mod app_server;
mod process;

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

async fn restore_completion_token(state: &Arc<AppState>, id: &str, completion_token: &str) {
    let mut map = state.providers.codex_login_sessions.lock().await;
    if let Some(status) = map.get_mut(id) {
        if status.status == "pending" && status.completion_token.is_none() {
            status.completion_token = Some(completion_token.to_string());
        }
    }
}

pub(crate) async fn complete_codex_login(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Json(req): Json<CodexLoginCompleteReq>,
) -> Result<Json<CodexLoginCompleteResp>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    let expected_callback = {
        let mut map = state.providers.codex_login_sessions.lock().await;
        let Some(status) = map.get_mut(&id) else {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "login not found".to_string(),
                }),
            ));
        };
        if status.status != "pending" {
            return Err((
                StatusCode::CONFLICT,
                Json(ApiErrorResp {
                    error: "login is not pending".to_string(),
                }),
            ));
        }
        if status.completion_token.as_deref() != Some(req.completion_token.as_str()) {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(ApiErrorResp {
                    error: "invalid completion token".to_string(),
                }),
            ));
        }
        let Some(expected_callback) = status.expected_callback_url.clone() else {
            return Err((
                StatusCode::CONFLICT,
                Json(ApiErrorResp {
                    error: "login is missing expected callback metadata".to_string(),
                }),
            ));
        };
        status.completion_token = None;
        expected_callback
    };

    if let Err(err) = validate_callback_url(&req.callback_url, Some(expected_callback.as_str())) {
        restore_completion_token(&state, &id, &req.completion_token).await;
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: err.to_string(),
            }),
        ));
    }

    let parsed_callback = Url::parse(&req.callback_url).map_err(|err| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: format!("invalid callback_url: {err}"),
            }),
        )
    })?;
    let callback_host = parsed_callback
        .host_str()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let client = if callback_host == "localhost" {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .resolve(
                "localhost",
                std::net::SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0),
            )
            .build()
            .map_err(|err| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: format!("failed to build callback replay client: {err}"),
                    }),
                )
            })?
    } else {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|err| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: format!("failed to build callback replay client: {err}"),
                    }),
                )
            })?
    };

    let response = match client
        .get(&req.callback_url)
        .timeout(Duration::from_secs(20))
        .send()
        .await
    {
        Ok(response) => response,
        Err(err) => {
            restore_completion_token(&state, &id, &req.completion_token).await;
            return Err((
                StatusCode::BAD_GATEWAY,
                Json(ApiErrorResp {
                    error: format!("failed to replay callback: {err}"),
                }),
            ));
        }
    };
    let status = response.status();
    if !status.is_success() {
        restore_completion_token(&state, &id, &req.completion_token).await;
        return Err((
            StatusCode::BAD_GATEWAY,
            Json(ApiErrorResp {
                error: format!("callback replay returned {status}"),
            }),
        ));
    }
    let status_code = status.as_u16();

    Ok(Json(CodexLoginCompleteResp {
        accepted: true,
        status_code,
    }))
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
    let codex_bin = crate::installer::require_codex_cli_command_path_for_target(
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
