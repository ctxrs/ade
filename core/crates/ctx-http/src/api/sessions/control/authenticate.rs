use super::super::*;

#[derive(Debug, Deserialize)]
pub(crate) struct AuthenticateSessionReq {
    #[serde(default)]
    method_id: Option<String>,
}

pub(crate) async fn authenticate_session(
    State(state): State<SessionsHandle>,
    Path(id): Path<String>,
    Json(req): Json<AuthenticateSessionReq>,
) -> Result<StatusCode, (StatusCode, Json<ApiErrorResp>)> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);

    state
        .authenticate_session_for_request(session_id, req.method_id)
        .await
        .map_err(map_session_auth_error)?;
    Ok(StatusCode::OK)
}

fn map_session_auth_error(
    error: ctx_daemon::daemon::sessions::auth::SessionAuthError,
) -> (StatusCode, Json<ApiErrorResp>) {
    match error {
        ctx_daemon::daemon::sessions::auth::SessionAuthError::NotFound(entity) => (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: format!("{entity} not found"),
            }),
        ),
        ctx_daemon::daemon::sessions::auth::SessionAuthError::BadRequest(error) => {
            (StatusCode::BAD_REQUEST, Json(ApiErrorResp { error }))
        }
        ctx_daemon::daemon::sessions::auth::SessionAuthError::Forbidden(error) => {
            (StatusCode::FORBIDDEN, Json(ApiErrorResp { error }))
        }
        ctx_daemon::daemon::sessions::auth::SessionAuthError::Internal(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp { error }),
        ),
        ctx_daemon::daemon::sessions::auth::SessionAuthError::AuthenticationFailed {
            redacted_message,
        } => {
            tracing::warn!("session authentication failed: {redacted_message}");
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "authentication failed".to_string(),
                }),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_session_auth_error_maps_policy_denials_to_forbidden() {
        let (status, body) = map_session_auth_error(
            ctx_daemon::daemon::sessions::auth::SessionAuthError::Forbidden(
                "host execution is disabled by daemon policy".to_string(),
            ),
        );

        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(
            body.0.error,
            "host execution is disabled by daemon policy".to_string()
        );
    }
}
