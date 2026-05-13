use super::*;

#[path = "kimi/monitor.rs"]
mod monitor;
#[path = "kimi/oauth.rs"]
mod oauth;

#[derive(Debug, Deserialize)]
pub(crate) struct KimiLoginStartReq {
    label: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct KimiLoginStartResp {
    login_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    auth_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    device_code: Option<String>,
}

pub(crate) async fn start_kimi_login(
    State(state): State<Arc<AppState>>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Json(req): Json<KimiLoginStartReq>,
) -> Result<Json<KimiLoginStartResp>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    let auth = oauth::request_kimi_device_authorization()
        .await
        .map_err(|err| {
            (
                StatusCode::BAD_GATEWAY,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&err.to_string()),
                }),
            )
        })?;
    let auth_url = auth
        .verification_uri_complete
        .clone()
        .or(auth.verification_uri.clone());
    let device_code = Some(auth.user_code.clone());
    let login_session =
        crate::daemon::providers::start_kimi_login_session(&state, auth_url, device_code).await;

    let state_clone = Arc::clone(&state);
    let login_id_for_task = login_session.login_id.clone();
    let poll_interval = oauth::poll_interval_for_authorization(&auth);
    let timeout = oauth::timeout_for_authorization(&auth);
    tokio::spawn(async move {
        monitor::monitor_kimi_login(
            state_clone,
            login_id_for_task,
            req.label,
            auth.device_code,
            poll_interval,
            timeout,
        )
        .await;
    });

    Ok(Json(KimiLoginStartResp {
        login_id: login_session.login_id,
        auth_url: login_session.auth_url,
        device_code: login_session.device_code,
    }))
}

pub(crate) async fn get_kimi_login(
    State(state): State<Arc<AppState>>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Path(id): Path<String>,
) -> Result<Json<provider_accounts::KimiLoginStatus>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    let status = crate::daemon::providers::kimi_login_status(&state, &id)
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
