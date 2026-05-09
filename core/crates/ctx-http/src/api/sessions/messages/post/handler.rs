use super::super::*;
use super::events::publish_user_message_events;
use super::persistence::{load_matching_existing_message, persist_user_message, PostMessageParts};
use super::request::PostMessageReq;
use super::scheduler::enqueue_message_for_scheduler;

pub(crate) async fn post_message(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
    Json(req): Json<PostMessageReq>,
) -> Result<Json<Message>, ApiErr> {
    let session_id = SessionId(
        uuid::Uuid::parse_str(&id)
            .map_err(|_| api_error(StatusCode::BAD_REQUEST, "Invalid session id."))?,
    );
    let store = store_for_existing_session_status_for_write(&state, session_id)
        .await
        .map_err(session_store_api_error)?;
    let run_id_header = headers
        .get("x-ctx-run-id")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_string());

    let session = store
        .get_session(session_id)
        .await
        .map_err(|_| api_error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to load session."))?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "Session not found."))?;
    state.sessions.remember_session_meta(&session).await;
    if let Some(drain) = state.core.update_drain.snapshot().await {
        return Err(api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            format!(
                "Daemon update is in progress; retry after the daemon restarts. ({})",
                drain.reason
            ),
        ));
    }

    let parts = PostMessageParts::from_request(&state, req).await?;
    if let Some(existing) =
        load_matching_existing_message(&state, &store, session_id, &parts).await?
    {
        return Ok(Json(existing));
    }

    let persisted = persist_user_message(&state, &store, &session, parts).await?;
    publish_user_message_events(&state, &store, session_id, &persisted).await?;
    enqueue_message_for_scheduler(&state, &store, session, &persisted, run_id_header).await;

    Ok(Json(persisted.saved))
}
