use super::*;

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

    let cfg = state
        .global_store()
        .get_mobile_access_config()
        .await
        .map_err(|e| {
            tracing::error!(
                "failed to read mobile access config while disabling mobile access: {e:?}"
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to read mobile access config".into(),
                }),
            )
        })?;

    state.transport.mobile_tunnel.stop().await;
    state
        .global_store()
        .clear_mobile_pairing_tokens()
        .await
        .map_err(|e| {
            tracing::error!("failed to clear pairing tokens while disabling mobile access: {e:?}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to clear pairing tokens".into(),
                }),
            )
        })?;
    if let Some(cfg) = cfg {
        state
            .global_store()
            .delete_mobile_access_config()
            .await
            .map_err(|e| {
                tracing::error!(
                    "failed to delete mobile access config while disabling mobile access: {e:?}"
                );
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: "failed to delete mobile access config".into(),
                    }),
                )
            })?;
        state
            .global_store()
            .delete_mobile_connection_profile(cfg.profile_id)
            .await
            .map_err(|e| {
                tracing::error!(
                    "failed to delete mobile connection profile while disabling mobile access: {e:?}"
                );
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: "failed to delete mobile connection profile".into(),
                    }),
                )
            })?;
    }
    Ok(StatusCode::NO_CONTENT)
}
