use super::*;
use crate::daemon::mobile_access as daemon_mobile_access;

pub(in crate::api) async fn disable_mobile_access(
    State(state): State<Arc<AppState>>,
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

    daemon_mobile_access::disable_mobile_access_runtime(&state)
        .await
        .map_err(disable_mobile_access_error)?;
    Ok(StatusCode::NO_CONTENT)
}

fn disable_mobile_access_error(
    error: daemon_mobile_access::DisableMobileAccessError,
) -> (StatusCode, Json<ApiErrorResp>) {
    let message = match error {
        daemon_mobile_access::DisableMobileAccessError::ReadConfig => {
            "failed to read mobile access config"
        }
        daemon_mobile_access::DisableMobileAccessError::ClearPairingTokens => {
            "failed to clear pairing tokens"
        }
        daemon_mobile_access::DisableMobileAccessError::DeleteConfig => {
            "failed to delete mobile access config"
        }
        daemon_mobile_access::DisableMobileAccessError::DeleteConnectionProfile => {
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
