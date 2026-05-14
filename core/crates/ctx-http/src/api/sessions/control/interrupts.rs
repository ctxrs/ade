use super::super::*;

use ctx_daemon::daemon::sessions::command_dispatch as daemon_command_dispatch;

pub(crate) async fn cancel_session(
    State(state): State<SessionsHandle>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);

    state
        .cancel_session(session_id)
        .await
        .map_err(session_command_status)?;
    Ok(StatusCode::OK)
}

pub(crate) async fn interrupt_session(
    State(state): State<SessionsHandle>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let request_started = std::time::Instant::now();
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);

    state
        .interrupt_session(session_id, request_started)
        .await
        .map_err(session_command_status)?;
    Ok(StatusCode::OK)
}

fn session_command_status(
    error: daemon_command_dispatch::SessionSchedulerCommandError,
) -> StatusCode {
    match error {
        daemon_command_dispatch::SessionSchedulerCommandError::BadRequest => {
            StatusCode::BAD_REQUEST
        }
        daemon_command_dispatch::SessionSchedulerCommandError::NotFound => StatusCode::NOT_FOUND,
        daemon_command_dispatch::SessionSchedulerCommandError::StoreUnavailable => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}
