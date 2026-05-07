use super::*;
use crate::perf_telemetry::{PerfMetric, PerfMetricKind};
#[path = "snapshot/vcs.rs"]
mod vcs;
pub(crate) use vcs::{
    apply_session_diff_patch, get_session_diff, get_session_diff_summary, get_session_git_status,
};

#[derive(Debug, Deserialize, Default)]
pub(crate) struct SessionSnapshotQuery {
    pub(crate) limit: Option<u32>,
    pub(crate) include_events: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct SessionHeadQuery {
    pub(crate) limit: Option<u32>,
    pub(crate) include_events: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct SessionEventsQuery {
    pub(crate) after_seq: Option<i64>,
    pub(crate) limit: Option<u32>,
    pub(crate) tail: Option<u32>,
    pub(crate) include_transient: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct SessionHistoryQuery {
    pub(crate) before_seq: Option<i64>,
    pub(crate) limit: Option<u32>,
}

pub(crate) async fn get_session_snapshot(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<SessionSnapshotQuery>,
) -> Result<Json<ctx_core::models::SessionSnapshot>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let limit = q.limit.unwrap_or(60);
    let include_events = parse_boolish_flag(q.include_events.as_deref(), "include_events")
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let store = store_for_existing_session_status_allow_archived(&state, session_id).await?;
    match store
        .get_session_snapshot(session_id, limit, include_events)
        .await
    {
        Ok(Some(snapshot)) => Ok(Json(snapshot)),
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

pub(crate) async fn get_session_head(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<SessionHeadQuery>,
) -> Result<Json<SessionHeadSnapshot>, StatusCode> {
    let started_at = Instant::now();
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let limit = q.limit.unwrap_or(60);
    let include_events = parse_boolish_flag(q.include_events.as_deref(), "include_events")
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let workspace_id = match state
        .global_store()
        .get_workspace_id_for_session(session_id)
        .await
    {
        Ok(Some(workspace_id)) => workspace_id,
        Ok(None) => return Err(StatusCode::NOT_FOUND),
        Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR),
    };
    if state.core.stores.is_workspace_deleting(workspace_id).await {
        return Err(StatusCode::NOT_FOUND);
    }
    if let Some(head) = state
        .workspaces
        .workspace_active_snapshot
        .get_cached_session_head_for_request(session_id, include_events, limit)
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
    let store = store_for_existing_session_status_allow_archived(&state, session_id).await?;
    state.emit_cache_miss("session_head").await;
    match store
        .get_session_head_snapshot(session_id, limit, include_events)
        .await
    {
        Ok(Some(head)) => {
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
            if include_events {
                state
                    .workspaces
                    .workspace_active_snapshot
                    .update_session_head(head.clone())
                    .await;
            } else {
                state
                    .workspaces
                    .workspace_active_snapshot
                    .update_compact_session_head(head.clone())
                    .await;
            }
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

fn record_session_head_recovery_metrics(
    state: &Arc<AppState>,
    source: &'static str,
    result: &'static str,
    elapsed: Duration,
    limit: u32,
    include_events: bool,
    head: Option<&SessionHeadSnapshot>,
) {
    let mut labels = HashMap::new();
    labels.insert("source".to_string(), "daemon".to_string());
    labels.insert("surface".to_string(), "session_head_recovery".to_string());
    labels.insert("recovery_source".to_string(), source.to_string());
    labels.insert("result".to_string(), result.to_string());
    labels.insert(
        "include_events".to_string(),
        if include_events { "true" } else { "false" }.to_string(),
    );
    labels.insert(
        "limit_bucket".to_string(),
        session_head_limit_bucket(limit).to_string(),
    );

    let response_bytes = head
        .map(|value| value.head_window.bytes.max(0) as f64)
        .unwrap_or(0.0);
    let metrics = [
        (
            "workbench.session_head_recovery_ms",
            "ms",
            elapsed.as_millis() as f64,
        ),
        (
            "workbench.session_head_recovery_response_bytes",
            "bytes",
            response_bytes,
        ),
        (
            "workbench.session_head_recovery_turn_count",
            "count",
            head.map(|value| value.turns.len() as f64).unwrap_or(0.0),
        ),
        (
            "workbench.session_head_recovery_message_count",
            "count",
            head.map(|value| value.messages.len() as f64).unwrap_or(0.0),
        ),
        (
            "workbench.session_head_recovery_tool_summary_count",
            "count",
            head.map(|value| value.tool_summaries.len() as f64)
                .unwrap_or(0.0),
        ),
        (
            "workbench.session_head_recovery_event_count",
            "count",
            head.map(|value| value.events.len() as f64).unwrap_or(0.0),
        ),
    ];
    let perf_telemetry = state.telemetry.perf_telemetry.clone();
    tokio::spawn(async move {
        for (name, unit, value) in metrics {
            perf_telemetry
                .record_metric(
                    PerfMetric {
                        name: name.to_string(),
                        kind: PerfMetricKind::Histogram,
                        unit: unit.to_string(),
                        value,
                        labels: labels.clone(),
                    },
                    None,
                    None,
                    None,
                )
                .await;
        }
    });
}

fn session_head_limit_bucket(limit: u32) -> &'static str {
    match limit {
        0 => "zero",
        1..=5 => "1_5",
        6..=60 => "6_60",
        61..=200 => "61_200",
        _ => "gt_200",
    }
}

pub(crate) async fn get_session_state(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<SessionState>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let store = store_for_existing_session_status_allow_archived(&state, session_id).await?;
    let session = store
        .get_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if session.is_none() {
        return Err(StatusCode::NOT_FOUND);
    }
    let mut session_state = store
        .get_session_state(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let session = session.ok_or(StatusCode::NOT_FOUND)?;
    for artifact in session_state.artifacts.iter_mut() {
        if !super::super::artifacts::session_artifact_path_is_accessible(
            &state,
            &store,
            &session,
            std::path::Path::new(&artifact.absolute_path),
        )
        .await?
        {
            artifact.missing = Some(true);
        }
    }
    Ok(Json(session_state))
}

pub(crate) async fn get_session_events(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<SessionEventsQuery>,
) -> Result<Json<ctx_core::models::SessionEventsPage>, StatusCode> {
    const DEFAULT_LIMIT: u32 = 200;
    const MAX_LIMIT: u32 = 1000;

    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let include_transient = parse_boolish_flag(q.include_transient.as_deref(), "include_transient")
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let store = store_for_existing_session_status_allow_archived(&state, session_id).await?;

    let (events, has_more, next_cursor) = if let Some(tail) = q.tail {
        let tail = tail.clamp(1, MAX_LIMIT);
        let mut rows = store
            .list_session_events_tail_by_seq(session_id, tail + 1, include_transient)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let has_more = rows.len() as u32 > tail;
        if has_more {
            rows = rows.split_off(rows.len().saturating_sub(tail as usize));
        }
        let next_cursor = rows.last().map(|ev| ev.seq);
        (rows, has_more, next_cursor)
    } else {
        let mut rows = store
            .list_session_events_page_by_seq(
                session_id,
                q.after_seq,
                Some(limit + 1),
                include_transient,
            )
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let has_more = rows.len() as u32 > limit;
        if has_more {
            rows.truncate(limit as usize);
        }
        let next_cursor = rows.last().map(|ev| ev.seq);
        (rows, has_more, next_cursor)
    };

    Ok(Json(ctx_core::models::SessionEventsPage {
        session_id,
        events,
        next_cursor,
        has_more,
    }))
}

pub(crate) async fn get_session_history(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<SessionHistoryQuery>,
) -> Result<Json<SessionHistoryPage>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let limit = q.limit.unwrap_or(60);
    let store = store_for_existing_session_status_allow_archived(&state, session_id).await?;
    match store
        .get_session_history_page(session_id, q.before_seq, limit)
        .await
    {
        Ok(Some(page)) => Ok(Json(page)),
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

pub(crate) async fn list_session_turn_tools(
    State(state): State<Arc<AppState>>,
    Path((id, turn_id)): Path<(String, String)>,
) -> Result<Json<Vec<SessionTurnTool>>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let turn_id = TurnId(uuid::Uuid::parse_str(&turn_id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let store = store_for_existing_session_status_allow_archived(&state, session_id).await?;
    store
        .list_turn_tools(session_id, turn_id)
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

fn parse_boolish_flag(raw: Option<&str>, label: &str) -> Result<bool, String> {
    match raw {
        Some(value) => ctx_core::boolish::parse_boolish(value)
            .ok_or_else(|| format!("{label} must be one of: 1/true/yes/on or 0/false/no/off")),
        None => Ok(false),
    }
}
