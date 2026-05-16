use super::login::reject_mobile_auth;
use super::*;
use crate::api::MobileAuthContext;
use axum::Extension;
use ctx_daemon::daemon::providers::CursorProcessLoginStartErrorKind;

#[derive(Debug, Deserialize)]
pub(crate) struct CursorLoginStartReq {
    label: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct CursorLoginStartResp {
    login_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    auth_url: Option<String>,
}

pub(crate) async fn start_cursor_login(
    State(providers): State<ProvidersHandle>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Json(req): Json<CursorLoginStartReq>,
) -> Result<Json<CursorLoginStartResp>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    let login_session = providers
        .start_cursor_process_login(req.label)
        .await
        .map_err(|err| {
            let status = match err.kind() {
                CursorProcessLoginStartErrorKind::RuntimeCommandBadRequest => {
                    StatusCode::BAD_REQUEST
                }
                CursorProcessLoginStartErrorKind::InternalStartup => {
                    StatusCode::INTERNAL_SERVER_ERROR
                }
            };
            (
                status,
                Json(ApiErrorResp {
                    error: err.route_safe_message().to_string(),
                }),
            )
        })?;

    Ok(Json(CursorLoginStartResp {
        login_id: login_session.login_id,
        auth_url: None,
    }))
}

pub(crate) async fn get_cursor_login(
    State(providers): State<ProvidersHandle>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Path(id): Path<String>,
) -> Result<Json<provider_accounts::CursorLoginStatus>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    let status = providers.cursor_login_status(&id).await.ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "login not found".to_string(),
            }),
        )
    })?;
    Ok(Json(status))
}
