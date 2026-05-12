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
    let login_id = uuid::Uuid::new_v4().to_string();
    state
        .providers
        .with_qwen_login_sessions(|map| {
            map.insert(
                login_id.clone(),
                provider_accounts::QwenLoginStatus {
                    login_id: login_id.clone(),
                    auth_url: None,
                    status: "pending".to_string(),
                    account_id: None,
                    error: None,
                },
            );
        })
        .await;

    let state_clone = Arc::clone(&state);
    let login_id_for_task = login_id.clone();
    tokio::spawn(async move {
        monitor_qwen_login(state_clone, login_id_for_task, req.label).await;
    });

    Ok(Json(QwenLoginStartResp {
        login_id,
        auth_url: None,
    }))
}

pub(crate) async fn get_qwen_login(
    State(state): State<Arc<AppState>>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Path(id): Path<String>,
) -> Result<Json<provider_accounts::QwenLoginStatus>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    let status = state
        .providers
        .with_qwen_login_sessions(|map| map.get(&id).cloned())
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
