use super::*;

#[derive(Debug, Deserialize)]
pub(crate) struct AmpLoginStartReq {
    label: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct AmpLoginStartResp {
    login_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    auth_url: Option<String>,
}

pub(crate) async fn start_amp_login(
    State(providers): State<ProvidersHandle>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Json(req): Json<AmpLoginStartReq>,
) -> Result<Json<AmpLoginStartResp>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    let login_session = providers.start_amp_browser_login(req.label).await;

    Ok(Json(AmpLoginStartResp {
        login_id: login_session.login_id,
        auth_url: login_session.auth_url,
    }))
}

pub(crate) async fn get_amp_login(
    State(providers): State<ProvidersHandle>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Path(id): Path<String>,
) -> Result<Json<provider_accounts::AmpLoginStatus>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    let status = providers.amp_login_status(&id).await.ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "login not found".to_string(),
            }),
        )
    })?;
    Ok(Json(status))
}
