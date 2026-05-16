use super::*;

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
    State(providers): State<ProvidersHandle>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Json(req): Json<CodexLoginStartReq>,
) -> Result<Json<CodexLoginStartResp>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    let started_login = providers
        .start_codex_app_server_login(req.label)
        .await
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: err.route_safe_message().to_string(),
                }),
            )
        })?;

    Ok(Json(CodexLoginStartResp {
        account_id: started_login.account_id,
        auth_url: started_login.auth_url,
        expected_callback_url: started_login.expected_callback_url,
        completion_token: started_login.completion_token,
    }))
}

pub(crate) async fn get_codex_login(
    State(providers): State<ProvidersHandle>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Path(id): Path<String>,
) -> Result<Json<provider_accounts::CodexLoginStatus>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    let status = providers.codex_login_status(&id).await.ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "login not found".to_string(),
            }),
        )
    })?;
    Ok(Json(status))
}

pub(crate) async fn complete_codex_login(
    State(providers): State<ProvidersHandle>,
    Path(id): Path<String>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Json(req): Json<CodexLoginCompleteReq>,
) -> Result<Json<CodexLoginCompleteResp>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    let response = providers
        .complete_codex_app_server_login(&id, req.callback_url, &req.completion_token)
        .await
        .map_err(codex_login_complete_error)?;
    Ok(Json(CodexLoginCompleteResp {
        accepted: response.accepted,
        status_code: response.status_code,
    }))
}

fn codex_login_complete_error(
    err: ctx_daemon::daemon::providers::CodexLoginCompleteError,
) -> (StatusCode, Json<ApiErrorResp>) {
    use ctx_daemon::daemon::providers::CodexLoginCompleteErrorKind;

    let status = match err.kind() {
        CodexLoginCompleteErrorKind::BadRequest => StatusCode::BAD_REQUEST,
        CodexLoginCompleteErrorKind::NotFound => StatusCode::NOT_FOUND,
        CodexLoginCompleteErrorKind::Conflict => StatusCode::CONFLICT,
        CodexLoginCompleteErrorKind::Unauthorized => StatusCode::UNAUTHORIZED,
        CodexLoginCompleteErrorKind::BadGateway => StatusCode::BAD_GATEWAY,
        CodexLoginCompleteErrorKind::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (
        status,
        Json(ApiErrorResp {
            error: err.route_safe_message().to_string(),
        }),
    )
}

#[cfg(test)]
mod tests;
