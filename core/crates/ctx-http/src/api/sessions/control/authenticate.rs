use super::super::*;

#[derive(Debug, Deserialize)]
pub(crate) struct AuthenticateSessionReq {
    #[serde(default)]
    method_id: Option<String>,
}

pub(crate) async fn authenticate_session(
    State(state): State<Arc<AppState>>,
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

    let store = store_for_existing_session_api_error_for_write(&state, session_id).await?;
    let session = store
        .get_session(session_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load session".to_string(),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "session not found".to_string(),
                }),
            )
        })?;
    crate::daemon::sessions::auth::run_session_authentication(
        &state,
        &store,
        &session,
        req.method_id,
    )
    .await
    .map_err(map_session_auth_error)?;
    Ok(StatusCode::OK)
}

fn map_session_auth_error(
    error: crate::daemon::sessions::auth::SessionAuthError,
) -> (StatusCode, Json<ApiErrorResp>) {
    match error {
        crate::daemon::sessions::auth::SessionAuthError::NotFound(entity) => (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: format!("{entity} not found"),
            }),
        ),
        crate::daemon::sessions::auth::SessionAuthError::BadRequest(error) => {
            (StatusCode::BAD_REQUEST, Json(ApiErrorResp { error }))
        }
        crate::daemon::sessions::auth::SessionAuthError::Forbidden(error) => {
            (StatusCode::FORBIDDEN, Json(ApiErrorResp { error }))
        }
        crate::daemon::sessions::auth::SessionAuthError::Internal(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp { error }),
        ),
        crate::daemon::sessions::auth::SessionAuthError::AuthenticationFailed {
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
        let (status, body) =
            map_session_auth_error(crate::daemon::sessions::auth::SessionAuthError::Forbidden(
                "host execution is disabled by daemon policy".to_string(),
            ));

        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(
            body.0.error,
            "host execution is disabled by daemon policy".to_string()
        );
    }
}
