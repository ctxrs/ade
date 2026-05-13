use super::*;

#[path = "mistral/monitor.rs"]
mod monitor;

#[derive(Debug, Deserialize)]
pub(crate) struct MistralLoginStartReq {
    label: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct MistralLoginStartResp {
    login_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    auth_url: Option<String>,
}

pub(crate) async fn start_mistral_login(
    State(state): State<Arc<AppState>>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Json(req): Json<MistralLoginStartReq>,
) -> Result<Json<MistralLoginStartResp>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    let label = req.label;
    let login_session = crate::daemon::providers::start_mistral_login_session(&state).await;

    let state_clone = Arc::clone(&state);
    let login_id_for_task = login_session.login_id.clone();
    tokio::spawn(async move {
        monitor::monitor_mistral_login(state_clone, login_id_for_task, label).await;
    });

    Ok(Json(MistralLoginStartResp {
        login_id: login_session.login_id,
        auth_url: login_session.auth_url,
    }))
}

pub(crate) async fn get_mistral_login(
    State(state): State<Arc<AppState>>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Path(id): Path<String>,
) -> Result<Json<provider_accounts::MistralLoginStatus>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    let status = crate::daemon::providers::mistral_login_status(&state, &id)
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
