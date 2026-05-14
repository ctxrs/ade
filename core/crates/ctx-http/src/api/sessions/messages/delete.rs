use super::*;

pub(crate) async fn delete_session_message(
    State(state): State<SessionsHandle>,
    Path((session_id, id)): Path<(String, String)>,
) -> Result<StatusCode, StatusCode> {
    let session_id =
        SessionId(uuid::Uuid::parse_str(&session_id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let msg_id = MessageId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);

    state
        .delete_queued_session_message(session_id, msg_id)
        .await
        .map_err(session_command_status)?;
    Ok(StatusCode::NO_CONTENT)
}

fn session_command_status(
    error: ctx_daemon::daemon::sessions::command_dispatch::SessionSchedulerCommandError,
) -> StatusCode {
    match error {
        ctx_daemon::daemon::sessions::command_dispatch::SessionSchedulerCommandError::BadRequest => {
            StatusCode::BAD_REQUEST
        }
        ctx_daemon::daemon::sessions::command_dispatch::SessionSchedulerCommandError::NotFound => {
            StatusCode::NOT_FOUND
        }
        ctx_daemon::daemon::sessions::command_dispatch::SessionSchedulerCommandError::StoreUnavailable => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}
