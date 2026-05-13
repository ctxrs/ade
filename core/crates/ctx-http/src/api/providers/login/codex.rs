use super::*;

mod app_server;
mod completion;
mod process;

#[cfg(test)]
use crate::daemon::providers::persist_successful_codex_login;
pub(crate) use completion::complete_codex_login;
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
    let crate::daemon::providers::PreparedCodexLoginStart {
        account_id,
        label,
        account_dir,
        codex_bin,
    } = crate::daemon::providers::prepare_codex_login_start(&state, req.label)
        .await
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
    state
        .providers
        .with_codex_login_sessions(|map| {
            map.insert(account_id.clone(), status);
        })
        .await;
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
    let status = state
        .providers
        .with_codex_login_sessions(|map| map.get(&id).cloned())
        .await
        .ok_or_else(|| {
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
