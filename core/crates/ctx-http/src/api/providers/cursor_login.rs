use super::login::reject_mobile_auth;
use super::*;
use crate::api::MobileAuthContext;
use axum::Extension;

mod capture;
mod output;
mod runtime;
mod session;
#[cfg(test)]
mod tests;

#[cfg(test)]
use capture::{
    cursor_login_home, ensure_private_dir, initialize_cursor_capture_file,
    write_cursor_capture_hook,
};
use runtime::resolve_cursor_login_runtime;
#[cfg(test)]
use runtime::resolve_cursor_login_runtime_from_config;
use session::monitor_cursor_login;

#[derive(Debug, Deserialize)]
pub(crate) struct CursorLoginStartReq {
    label: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct CursorLoginStartResp {
    login_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    auth_url: Option<String>,
}

pub(crate) async fn start_cursor_login(
    State(state): State<Arc<AppState>>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Json(req): Json<CursorLoginStartReq>,
) -> Result<Json<CursorLoginStartResp>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    let _ = resolve_cursor_login_runtime(&state).await.map_err(|e| {
        let msg = e.to_string();
        let status = if msg.contains("runtime_command_") {
            StatusCode::BAD_REQUEST
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };
        (status, Json(ApiErrorResp { error: msg }))
    })?;

    let login_id = uuid::Uuid::new_v4().to_string();
    {
        let mut map = state.providers.cursor_login_sessions.lock().await;
        map.insert(
            login_id.clone(),
            provider_accounts::CursorLoginStatus {
                login_id: login_id.clone(),
                auth_url: None,
                status: "pending".to_string(),
                account_id: None,
                error: None,
            },
        );
    }

    let state_clone = Arc::clone(&state);
    let login_id_for_task = login_id.clone();
    tokio::spawn(async move {
        monitor_cursor_login(state_clone, login_id_for_task, req.label).await;
    });

    Ok(Json(CursorLoginStartResp {
        login_id,
        auth_url: None,
    }))
}

pub(crate) async fn get_cursor_login(
    State(state): State<Arc<AppState>>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Path(id): Path<String>,
) -> Result<Json<provider_accounts::CursorLoginStatus>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    let map = state.providers.cursor_login_sessions.lock().await;
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
