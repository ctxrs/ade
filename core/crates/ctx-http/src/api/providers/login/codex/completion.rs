use super::*;

#[path = "completion/replay.rs"]
mod replay;

use replay::replay_codex_callback;

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

    let status_code = match replay_codex_callback(&req.callback_url).await {
        Ok(status_code) => status_code,
        Err(err) => {
            if err.should_restore_completion_token() {
                restore_completion_token(&state, &id, &req.completion_token).await;
            }
            let status = err.status_code();
            let error = err.into_message();
            return Err((status, Json(ApiErrorResp { error })));
        }
    };

    Ok(Json(CodexLoginCompleteResp {
        accepted: true,
        status_code,
    }))
}
