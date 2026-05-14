use super::*;
use ctx_daemon::daemon::mobile_access::DisableMobileAccessError;

pub(in crate::api) async fn disable_mobile_access(
    State(state): State<CoreHandle>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Json(req): Json<EnableMobileAccessReq>,
) -> Result<StatusCode, (StatusCode, Json<ApiErrorResp>)> {
    if mobile_auth.is_some() {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ApiErrorResp {
                error: "desktop auth required".into(),
            }),
        ));
    }

    let control_plane_url = resolve_control_plane_url();
    if !control_plane_url.trim().is_empty() {
        let _ = reqwest::Client::new()
            .post(format!(
                "{}/v1/mobile/revoke",
                control_plane_url.trim_end_matches('/')
            ))
            .bearer_auth(req.supabase_token.trim())
            .send()
            .await;
    }

    state
        .disable_mobile_access_runtime()
        .await
        .map_err(disable_mobile_access_error)?;
    Ok(StatusCode::NO_CONTENT)
}

fn disable_mobile_access_error(
    error: DisableMobileAccessError,
) -> (StatusCode, Json<ApiErrorResp>) {
    let message = match error {
        DisableMobileAccessError::ReadConfig => "failed to read mobile access config",
        DisableMobileAccessError::ClearPairingTokens => "failed to clear pairing tokens",
        DisableMobileAccessError::DeleteConfig => "failed to delete mobile access config",
        DisableMobileAccessError::DeleteConnectionProfile => {
            "failed to delete mobile connection profile"
        }
    };
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ApiErrorResp {
            error: message.into(),
        }),
    )
}
