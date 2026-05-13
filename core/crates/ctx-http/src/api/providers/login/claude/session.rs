use super::*;

mod process;

use process::{monitor_claude_login, start_claude_login_process};

#[derive(Debug, Deserialize)]
pub(crate) struct ClaudeLoginStartReq {
    label: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ClaudeLoginStartResp {
    login_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    auth_url: Option<String>,
}

pub(crate) async fn start_claude_login(
    State(state): State<Arc<AppState>>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Json(req): Json<ClaudeLoginStartReq>,
) -> Result<Json<ClaudeLoginStartResp>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    let label = req.label;
    let login = start_claude_login_process(&state).await.map_err(|e| {
        let msg = format!("{e:#}");
        let status = if msg.contains("runtime_command_") {
            StatusCode::BAD_REQUEST
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };
        (status, Json(ApiErrorResp { error: msg }))
    })?;
    let auth_url = login.auth_url.clone();
    let login_session =
        crate::daemon::providers::start_claude_login_session(&state, auth_url).await;
    let state_clone = Arc::clone(&state);
    let login_id_for_task = login_session.login_id.clone();
    tokio::spawn(async move {
        monitor_claude_login(state_clone, login_id_for_task, label, login).await;
    });

    Ok(Json(ClaudeLoginStartResp {
        login_id: login_session.login_id,
        auth_url: login_session.auth_url,
    }))
}

pub(crate) async fn get_claude_login(
    State(state): State<Arc<AppState>>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Path(id): Path<String>,
) -> Result<Json<provider_accounts::ClaudeLoginStatus>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    let status = crate::daemon::providers::claude_login_status(&state, &id)
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
