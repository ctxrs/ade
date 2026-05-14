use super::*;

#[path = "completion/replay.rs"]
mod replay;

use replay::replay_codex_callback;

async fn restore_completion_token(providers: &ProvidersHandle, id: &str, completion_token: &str) {
    providers
        .restore_codex_login_completion_token(id, completion_token)
        .await;
}

fn claim_error_response(
    err: ctx_daemon::daemon::providers::CodexLoginCallbackClaimError,
) -> (StatusCode, Json<ApiErrorResp>) {
    match err {
        ctx_daemon::daemon::providers::CodexLoginCallbackClaimError::NotFound => (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "login not found".to_string(),
            }),
        ),
        ctx_daemon::daemon::providers::CodexLoginCallbackClaimError::NotPending => (
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "login is not pending".to_string(),
            }),
        ),
        ctx_daemon::daemon::providers::CodexLoginCallbackClaimError::InvalidCompletionToken => (
            StatusCode::UNAUTHORIZED,
            Json(ApiErrorResp {
                error: "invalid completion token".to_string(),
            }),
        ),
        ctx_daemon::daemon::providers::CodexLoginCallbackClaimError::MissingExpectedCallback => (
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "login is missing expected callback metadata".to_string(),
            }),
        ),
    }
}

pub(crate) async fn complete_codex_login(
    State(providers): State<ProvidersHandle>,
    Path(id): Path<String>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Json(req): Json<CodexLoginCompleteReq>,
) -> Result<Json<CodexLoginCompleteResp>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    let expected_callback = providers
        .claim_codex_login_callback(&id, &req.completion_token)
        .await
        .map_err(claim_error_response)?;

    if let Err(err) = validate_callback_url(&req.callback_url, Some(expected_callback.as_str())) {
        restore_completion_token(&providers, &id, &req.completion_token).await;
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
                restore_completion_token(&providers, &id, &req.completion_token).await;
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
