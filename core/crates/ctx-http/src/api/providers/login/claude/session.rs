use super::*;

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
    State(providers): State<ProvidersHandle>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Json(req): Json<ClaudeLoginStartReq>,
) -> Result<Json<ClaudeLoginStartResp>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    let login_session = providers
        .start_claude_setup_token_login(req.label)
        .await
        .map_err(claude_setup_token_login_start_error)?;

    Ok(Json(ClaudeLoginStartResp {
        login_id: login_session.login_id,
        auth_url: login_session.auth_url,
    }))
}

pub(crate) async fn get_claude_login(
    State(providers): State<ProvidersHandle>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Path(id): Path<String>,
) -> Result<Json<provider_accounts::ClaudeLoginStatus>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    let status = providers.claude_login_status(&id).await.ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "login not found".to_string(),
            }),
        )
    })?;
    Ok(Json(status))
}

fn claude_setup_token_login_start_error(
    err: ctx_daemon::daemon::providers::ClaudeSetupTokenLoginStartError,
) -> (StatusCode, Json<ApiErrorResp>) {
    use ctx_daemon::daemon::providers::ClaudeSetupTokenLoginStartErrorKind;

    let status = match err.kind() {
        ClaudeSetupTokenLoginStartErrorKind::BadRequest => StatusCode::BAD_REQUEST,
        ClaudeSetupTokenLoginStartErrorKind::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (
        status,
        Json(ApiErrorResp {
            error: err.route_safe_message().to_string(),
        }),
    )
}
