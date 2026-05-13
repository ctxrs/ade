use super::*;

mod monitor;

use monitor::monitor_qwen_login;

#[derive(Debug, Deserialize)]
pub(crate) struct QwenLoginStartReq {
    label: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct QwenLoginStartResp {
    login_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    auth_url: Option<String>,
}

pub(crate) async fn start_qwen_login(
    State(state): State<Arc<AppState>>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Json(req): Json<QwenLoginStartReq>,
) -> Result<Json<QwenLoginStartResp>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    let login_session = crate::daemon::providers::start_qwen_login_session(&state).await;

    let state_clone = Arc::clone(&state);
    let login_id_for_task = login_session.login_id.clone();
    tokio::spawn(async move {
        monitor_qwen_login(state_clone, login_id_for_task, req.label).await;
    });

    Ok(Json(QwenLoginStartResp {
        login_id: login_session.login_id,
        auth_url: login_session.auth_url,
    }))
}

pub(crate) async fn get_qwen_login(
    State(state): State<Arc<AppState>>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Path(id): Path<String>,
) -> Result<Json<provider_accounts::QwenLoginStatus>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    let status = crate::daemon::providers::qwen_login_status(&state, &id)
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
