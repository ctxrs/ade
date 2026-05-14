use super::*;

#[derive(Debug, Deserialize)]
pub(crate) struct SetSessionModeReq {
    pub(crate) mode_id: String,
}

pub(crate) async fn set_session_mode(
    State(state): State<SessionsHandle>,
    Path(id): Path<String>,
    Json(req): Json<SetSessionModeReq>,
) -> Result<StatusCode, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);

    state
        .set_session_mode_for_request(session_id, req.mode_id)
        .await
        .map_err(map_set_session_mode_error)?;

    Ok(StatusCode::OK)
}

fn map_set_session_mode_error(error: crate::daemon::sessions::SetSessionModeError) -> StatusCode {
    match error {
        crate::daemon::sessions::SetSessionModeError::NotFound => StatusCode::NOT_FOUND,
        crate::daemon::sessions::SetSessionModeError::BadRequest => StatusCode::BAD_REQUEST,
        crate::daemon::sessions::SetSessionModeError::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
