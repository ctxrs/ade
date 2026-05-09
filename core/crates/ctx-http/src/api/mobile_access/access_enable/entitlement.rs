use super::*;

pub(super) async fn request_control_plane_enable(
    supabase_token: &str,
) -> Result<ControlPlaneEnableResp, (StatusCode, Json<ApiErrorResp>)> {
    let control_plane_url = resolve_control_plane_url();
    if control_plane_url.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "CTX_TUNNEL_CONTROL_PLANE_URL is not set".into(),
            }),
        ));
    }

    let enable_resp = reqwest::Client::new()
        .post(format!(
            "{}/v1/mobile/enable",
            control_plane_url.trim_end_matches('/')
        ))
        .bearer_auth(supabase_token.trim())
        .send()
        .await
        .map_err(|e| {
            tracing::error!("failed to call control plane: {e:?}");
            (
                StatusCode::BAD_GATEWAY,
                Json(ApiErrorResp {
                    error: "failed to reach control plane".into(),
                }),
            )
        })?;

    if !enable_resp.status().is_success() {
        let status = enable_resp.status();
        let body = enable_resp.text().await.unwrap_or_default();
        tracing::warn!("control plane denied enable: {status} {body}");
        return Err((
            StatusCode::FORBIDDEN,
            Json(ApiErrorResp {
                error: "mobile access not entitled".into(),
            }),
        ));
    }

    enable_resp
        .json::<ControlPlaneEnableResp>()
        .await
        .map_err(|e| {
            tracing::error!("invalid control plane response: {e:?}");
            (
                StatusCode::BAD_GATEWAY,
                Json(ApiErrorResp {
                    error: "invalid control plane response".into(),
                }),
            )
        })
}

pub(super) fn parse_allowed_public_url(
    raw_public_base_url: &str,
) -> Result<Url, (StatusCode, Json<ApiErrorResp>)> {
    let public_url = Url::parse(raw_public_base_url).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "public_base_url must be a valid URL".into(),
            }),
        )
    })?;
    if !mobile_public_url_is_allowed(&public_url) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "public_base_url must use https://".into(),
            }),
        ));
    }
    Ok(public_url)
}
