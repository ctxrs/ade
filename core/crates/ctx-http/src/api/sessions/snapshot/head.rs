use super::*;
use crate::api::sessions::snapshot::head_metrics::record_session_head_recovery_metrics;

#[derive(Debug, Deserialize, Default)]
pub(crate) struct SessionHeadQuery {
    pub(crate) limit: Option<u32>,
    pub(crate) include_events: Option<String>,
    pub(crate) min_event_seq: Option<i64>,
}

pub(crate) async fn get_session_head(
    State(state): State<SessionsHandle>,
    Path(id): Path<String>,
    Query(q): Query<SessionHeadQuery>,
) -> Result<Json<SessionHeadSnapshot>, StatusCode> {
    let started_at = Instant::now();
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let limit = q.limit.unwrap_or(60);
    let include_events = super::parse_boolish_flag(q.include_events.as_deref(), "include_events")
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let min_event_seq = match q.min_event_seq {
        Some(value) if value < 0 => return Err(StatusCode::BAD_REQUEST),
        value => value,
    };
    let workspace_id = match state.workspace_id_for_session(session_id).await {
        Ok(Some(workspace_id)) => workspace_id,
        Ok(None) => return Err(StatusCode::NOT_FOUND),
        Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR),
    };
    if state.is_workspace_deleting(workspace_id).await {
        return Err(StatusCode::NOT_FOUND);
    }
    if let Some(head) = state
        .cached_session_head_for_request(session_id, include_events, limit, min_event_seq)
        .await
    {
        record_session_head_recovery_metrics(
            &state,
            "active_snapshot_cache",
            "ok",
            started_at.elapsed(),
            limit,
            include_events,
            Some(&head),
        );
        return Ok(Json(head));
    }
    state.emit_cache_miss("session_head").await;
    match state
        .load_session_head_snapshot_from_store(session_id, limit, include_events)
        .await
    {
        Ok(Some(head)) => {
            if let Some(min_event_seq) = min_event_seq {
                if head.last_event_seq < min_event_seq {
                    state.emit_cache_rehydrate("session_head", false).await;
                    record_session_head_recovery_metrics(
                        &state,
                        "store_rebuild",
                        "stale",
                        started_at.elapsed(),
                        limit,
                        include_events,
                        Some(&head),
                    );
                    return Err(StatusCode::CONFLICT);
                }
            }
            state.emit_cache_rehydrate("session_head", true).await;
            record_session_head_recovery_metrics(
                &state,
                "store_rebuild",
                "ok",
                started_at.elapsed(),
                limit,
                include_events,
                Some(&head),
            );
            state
                .update_session_head_cache(head.clone(), include_events)
                .await;
            Ok(Json(head))
        }
        Ok(None) => {
            state.emit_cache_rehydrate("session_head", false).await;
            record_session_head_recovery_metrics(
                &state,
                "store_rebuild",
                "missing",
                started_at.elapsed(),
                limit,
                include_events,
                None,
            );
            Err(StatusCode::NOT_FOUND)
        }
        Err(_) => {
            state.emit_cache_rehydrate("session_head", false).await;
            record_session_head_recovery_metrics(
                &state,
                "store_rebuild",
                "error",
                started_at.elapsed(),
                limit,
                include_events,
                None,
            );
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}
