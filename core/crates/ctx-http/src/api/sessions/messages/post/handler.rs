use super::super::*;
use super::delivery::queued_messages_enabled;
use super::persistence::PostMessageParts;
use super::request::PostMessageReq;
use crate::daemon::sessions::{PostUserMessageError, PostUserMessageInput};

pub(crate) async fn post_message(
    State(state): State<SessionsHandle>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
    Json(req): Json<PostMessageReq>,
) -> Result<Json<Message>, ApiErr> {
    let session_id = SessionId(
        uuid::Uuid::parse_str(&id)
            .map_err(|_| api_error(StatusCode::BAD_REQUEST, "Invalid session id."))?,
    );
    let run_id_header = headers
        .get("x-ctx-run-id")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_string());

    let parts = PostMessageParts::from_request(&state, req).await?;
    state
        .post_user_message_for_request(
            session_id,
            PostUserMessageInput {
                message_id: parts.message_id,
                turn_id: parts.turn_id,
                client_supplied_ids: parts.client_supplied_ids,
                content: parts.content,
                requested_delivery: parts.requested_delivery,
                attachments: parts.attachments,
                queued_messages_enabled: queued_messages_enabled(),
                run_id_header,
            },
        )
        .await
        .map(Json)
        .map_err(post_user_message_api_error)
}

fn post_user_message_api_error(error: PostUserMessageError) -> ApiErr {
    match error {
        PostUserMessageError::BadRequest(error) => api_error(StatusCode::BAD_REQUEST, error),
        PostUserMessageError::Conflict(error) => api_error(StatusCode::CONFLICT, error),
        PostUserMessageError::NotFound(error) => api_error(StatusCode::NOT_FOUND, error),
        PostUserMessageError::ServiceUnavailable(error) => {
            api_error(StatusCode::SERVICE_UNAVAILABLE, error)
        }
        PostUserMessageError::Internal(error) => {
            api_error(StatusCode::INTERNAL_SERVER_ERROR, error)
        }
    }
}
