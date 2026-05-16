use super::*;

#[derive(Debug, Deserialize)]
pub(crate) struct KimiLoginStartReq {
    label: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct KimiLoginStartResp {
    login_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    auth_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    device_code: Option<String>,
}

pub(crate) async fn start_kimi_login(
    State(providers): State<ProvidersHandle>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Json(req): Json<KimiLoginStartReq>,
) -> Result<Json<KimiLoginStartResp>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    let login_session = providers
        .start_kimi_oauth_login(req.label)
        .await
        .map_err(|err| {
            (
                StatusCode::BAD_GATEWAY,
                Json(ApiErrorResp {
                    error: err.route_safe_message().to_string(),
                }),
            )
        })?;

    Ok(Json(KimiLoginStartResp {
        login_id: login_session.login_id,
        auth_url: login_session.auth_url,
        device_code: login_session.device_code,
    }))
}

pub(crate) async fn get_kimi_login(
    State(providers): State<ProvidersHandle>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Path(id): Path<String>,
) -> Result<Json<provider_accounts::KimiLoginStatus>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    let status = providers.kimi_login_status(&id).await.ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "login not found".to_string(),
            }),
        )
    })?;
    Ok(Json(status))
}
