use std::sync::Arc;
use std::time::Duration;

use ctx_core::ids::{SessionId, TaskId};
use ctx_core::models::{
    SessionEvent, SessionEventType, SessionHeadDelta, SessionTurn, SessionTurnToolSummary,
    TaskDeltaKind,
};
use ctx_workspace_active_snapshot::session_metadata_from_session;

use ctx_session_service::head_projection::{
    activity_from_turn, build_session_summary_delta, derive_message_preview,
    derive_summary_activity, event_context_window, is_session_gap_notice, message_from_event,
    patch_turn_from_event, recompute_turn_tool_counts, resolve_projection_rev_for_stream_delta,
    should_include_session_metadata_in_head_delta, should_refresh_turn_from_store, turn_from_event,
};
use ctx_session_service::runtime::ActiveTaskRefreshEntry;

use crate::daemon::state::AppState;

const ACTIVE_TASK_REFRESH_DEBOUNCE_MS: u64 = 250;

pub async fn publish_event(state: &Arc<AppState>, event: SessionEvent) {
    let tx = state.sessions.get_broadcaster(event.session_id).await;
    let _ = tx.send(event.clone());
    state
        .sessions
        .publish_session_event_head(event.session_id, event.seq)
        .await;
    update_workspace_active_snapshot_for_event(state, &event).await;
}

async fn update_workspace_active_snapshot_for_event(state: &Arc<AppState>, event: &SessionEvent) {
    if is_session_gap_notice(event) {
        return;
    }
    let session = {
        let mut cache = state.sessions.session_meta_cache.lock().await;
        cache.get_mut(&event.session_id).map(|entry| {
            entry.touch();
            entry.value.clone()
        })
    };
    let session = match session {
        Some(session) => session,
        None => {
            let store = match state.store_for_session(event.session_id).await {
                Ok(store) => store,
                Err(_) => return,
            };
            let Some(session) = store.get_session(event.session_id).await.ok().flatten() else {
                return;
            };
            state.sessions.remember_session_meta(&session).await;
            session
        }
    };

    let stream_only = matches!(
        event.event_type,
        SessionEventType::AssistantChunk
            | SessionEventType::ThoughtChunk
            | SessionEventType::ContextWindowUpdate
    );

    let update_task = matches!(
        event.event_type,
        SessionEventType::UserMessage
            | SessionEventType::AssistantMessageInserted
            | SessionEventType::AssistantComplete
            | SessionEventType::Done
            | SessionEventType::TurnQueued
            | SessionEventType::TurnStarted
            | SessionEventType::TurnFinished
            | SessionEventType::TurnInterrupted
            | SessionEventType::MessageQueueAdded
            | SessionEventType::MessageQueueUpdated
            | SessionEventType::MessageQueueRemoved
            | SessionEventType::MessageQueuePromoted
            | SessionEventType::Error
    );
    if update_task {
        queue_workspace_task_delta_refresh(state, session.task_id).await;
    }

    let message = if matches!(
        event.event_type,
        SessionEventType::UserMessage
            | SessionEventType::AssistantMessageInserted
            | SessionEventType::Notice
    ) {
        message_from_event(event, &session)
    } else {
        None
    };
    let mut tool_summaries: Vec<SessionTurnToolSummary> = Vec::new();
    let tool_event = matches!(
        event.event_type,
        SessionEventType::ToolCall
            | SessionEventType::ToolCallUpdate
            | SessionEventType::ToolResult
    );
    if tool_event {
        if let Some(turn_id) = event.turn_id {
            if let Ok(store) = state.store_for_session(event.session_id).await {
                if let Ok(list) = store
                    .list_turn_tool_summaries_for_turns(
                        event.session_id,
                        std::slice::from_ref(&turn_id),
                    )
                    .await
                {
                    tool_summaries = list;
                }
            }
        }
    }
    let mut turn = turn_from_event(event, message.as_ref());
    let prefers_cached_turn = tool_event
        || event_context_window(event).is_some()
        || should_refresh_turn_from_store(&event.event_type);
    if turn.is_none() && prefers_cached_turn {
        if let Some(turn_id) = event.turn_id {
            turn = turn_from_cached_head_for_read(state, event.session_id, turn_id).await;
        }
    }
    if turn.is_none() && (should_refresh_turn_from_store(&event.event_type) || tool_event) {
        if let Some(turn_id) = event.turn_id {
            if let Ok(store) = state.store_for_session(event.session_id).await {
                if let Ok(Some(fetched)) = store.get_session_turn(event.session_id, turn_id).await {
                    turn = Some(fetched);
                }
            }
        }
    }
    if let Some(turn) = turn.as_mut() {
        patch_turn_from_event(turn, event);
        if !tool_summaries.is_empty() {
            recompute_turn_tool_counts(turn, &tool_summaries);
        }
    }

    let cached_replay_cursor = if stream_only {
        state
            .workspaces
            .workspace_active_snapshot
            .session_replay_cursor(session.workspace_id, event.session_id)
            .await
    } else {
        Default::default()
    };
    let last_event_seq = if stream_only {
        cached_replay_cursor.last_event_seq
    } else {
        event.seq
    };
    let projection_rev = resolve_projection_rev_for_stream_delta(
        stream_only,
        last_event_seq,
        cached_replay_cursor.projection_rev,
        || async {
            match state.store_for_session(event.session_id).await {
                Ok(store) => store
                    .get_session_projection_rev(event.session_id)
                    .await
                    .ok(),
                Err(_) => None,
            }
        },
    )
    .await;
    let state_rev = last_event_seq;

    let activity = derive_summary_activity(event).or_else(|| turn.as_ref().map(activity_from_turn));

    let mut last_message_at = None;
    let mut last_message_preview = None;
    if let Some(message) = message.as_ref() {
        last_message_at = Some(message.created_at);
        last_message_preview = Some(derive_message_preview(&message.content));
    }

    let summary_delta = build_session_summary_delta(
        &session,
        activity.clone(),
        last_message_at,
        last_message_preview,
        last_event_seq,
        projection_rev,
        state_rev,
    );

    let delta = SessionHeadDelta {
        session_id: event.session_id,
        last_event_seq,
        projection_rev,
        state_rev,
        emitted_at_ms: Some(chrono::Utc::now().timestamp_millis()),
        session: should_include_session_metadata_in_head_delta(&event.event_type)
            .then(|| session_metadata_from_session(&session)),
        activity,
        event: Some(event.clone()),
        turn,
        message,
        tool_summaries,
    };
    state
        .workspaces
        .workspace_active_snapshot
        .publish_session_head_delta(session.workspace_id, &session, delta, !stream_only)
        .await;

    if let Some(summary_delta) = summary_delta {
        state
            .workspaces
            .workspace_active_snapshot
            .publish_session_summary_delta(session.workspace_id, summary_delta)
            .await;
    }
}

async fn queue_workspace_task_delta_refresh(state: &Arc<AppState>, task_id: TaskId) {
    let should_spawn = {
        let mut map = state.sessions.active_task_refreshes.lock().await;
        if let Some(entry) = map.get_mut(&task_id) {
            entry.generation = entry.generation.wrapping_add(1);
            false
        } else {
            map.insert(task_id, ActiveTaskRefreshEntry { generation: 1 });
            true
        }
    };
    if should_spawn {
        let state = Arc::downgrade(state);
        tokio::spawn(async move {
            let Some(state) = state.upgrade() else {
                return;
            };
            let state_clone = state.clone();
            run_workspace_task_delta_refresh(state_clone, task_id).await;
        });
    }
}

async fn run_workspace_task_delta_refresh(state: Arc<AppState>, task_id: TaskId) {
    let debounce = Duration::from_millis(ACTIVE_TASK_REFRESH_DEBOUNCE_MS.max(1));
    loop {
        let generation = {
            let map = state.sessions.active_task_refreshes.lock().await;
            match map.get(&task_id) {
                Some(entry) => entry.generation,
                None => return,
            }
        };

        tokio::time::sleep(debounce).await;

        let current = {
            let map = state.sessions.active_task_refreshes.lock().await;
            match map.get(&task_id) {
                Some(entry) => entry.generation,
                None => return,
            }
        };
        if current != generation {
            continue;
        }

        match state.store_for_task(task_id).await {
            Ok(store) => match store.get_workspace_active_task_summary(task_id).await {
                Ok(Some(summary)) => {
                    let _ = state
                        .emit_workspace_task_delta(summary.task, TaskDeltaKind::Updated)
                        .await;
                }
                Ok(None) => match store.get_task(task_id).await {
                    Ok(Some(task)) => {
                        let kind = if task.archived_at.is_some() {
                            TaskDeltaKind::Archived
                        } else {
                            TaskDeltaKind::Updated
                        };
                        let _ = state.emit_workspace_task_delta(task, kind).await;
                    }
                    Ok(None) => {}
                    Err(err) => {
                        tracing::warn!(
                            task_id = %task_id.0,
                            "workspace task delta refresh read failed: {err:?}"
                        );
                    }
                },
                Err(err) => {
                    tracing::warn!(
                        task_id = %task_id.0,
                        "workspace task delta refresh summary read failed: {err:?}"
                    );
                }
            },
            Err(err) => {
                tracing::warn!(
                    task_id = %task_id.0,
                    "workspace task delta refresh store lookup failed: {err:?}"
                );
            }
        }

        let mut map = state.sessions.active_task_refreshes.lock().await;
        match map.get(&task_id) {
            Some(entry) if entry.generation == current => {
                map.remove(&task_id);
                return;
            }
            Some(_) => continue,
            None => return,
        }
    }
}

pub async fn refresh_session_head_cache(state: &AppState, session_id: SessionId) {
    let store = match state.store_for_session(session_id).await {
        Ok(store) => store,
        Err(_) => return,
    };
    let head = match store.get_active_snapshot_head(session_id).await {
        Ok(Some(head)) => head,
        Ok(None) => {
            state
                .workspaces
                .workspace_active_snapshot
                .remove_session(session_id)
                .await;
            return;
        }
        Err(err) => {
            tracing::warn!(
                session_id = %session_id.0,
                "active session head cache refresh failed: {err:#}"
            );
            return;
        }
    };
    state
        .workspaces
        .workspace_active_snapshot
        .update_compact_session_head(head)
        .await;
}

async fn turn_from_cached_head_for_read(
    state: &Arc<AppState>,
    session_id: SessionId,
    turn_id: ctx_core::ids::TurnId,
) -> Option<SessionTurn> {
    state
        .workspaces
        .workspace_active_snapshot
        .get_cached_session_head_for_read(session_id)
        .await
        .and_then(|head| head.turns.into_iter().find(|turn| turn.turn_id == turn_id))
}
