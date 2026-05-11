use super::super::*;

#[cfg(test)]
pub(super) use self::coalesce::merge_partial_fragment;
pub(super) use self::coalesce::try_coalesce_partial_delta;

#[path = "partials/coalesce.rs"]
mod coalesce;

fn is_partial_event(event: &SessionEvent) -> bool {
    matches!(
        event.event_type,
        SessionEventType::AssistantChunk
            | SessionEventType::ThoughtChunk
            | SessionEventType::ContextWindowUpdate
    )
}

fn allows_partial_for_foreground_session(
    foreground_session_ids: Option<&HashSet<SessionId>>,
    session_id: SessionId,
) -> bool {
    foreground_session_ids
        .map(|session_ids| session_ids.contains(&session_id))
        .unwrap_or(false)
}

pub(crate) fn is_foreground_session(
    foreground_session_ids: Option<&HashSet<SessionId>>,
    session_id: SessionId,
) -> bool {
    allows_partial_for_foreground_session(foreground_session_ids, session_id)
}

pub(crate) fn should_stream_head_delta(
    active_task_sessions: &HashMap<TaskId, SessionId>,
    explicit_sessions: &HashSet<SessionId>,
    foreground_session_ids: Option<&HashSet<SessionId>>,
    session_id: SessionId,
) -> bool {
    active_task_sessions
        .values()
        .any(|active_session_id| *active_session_id == session_id)
        || explicit_sessions.contains(&session_id)
        || allows_partial_for_foreground_session(foreground_session_ids, session_id)
}

pub(crate) fn is_priority_control_event(
    event: &WorkspaceActiveSnapshotEvent,
    foreground_session_ids: Option<&HashSet<SessionId>>,
) -> bool {
    match event {
        WorkspaceActiveSnapshotEvent::SessionGap { session_id, .. } => {
            is_foreground_session(foreground_session_ids, *session_id)
        }
        WorkspaceActiveSnapshotEvent::SessionHeadSeed { head, .. } => {
            is_foreground_session(foreground_session_ids, head.session.id)
        }
        _ => false,
    }
}

pub(crate) fn filter_partial_delta_for_active_tasks(
    mut delta: SessionHeadDelta,
    _active_task_sessions: &HashMap<TaskId, SessionId>,
    foreground_session_ids: Option<&HashSet<SessionId>>,
) -> Option<SessionHeadDelta> {
    if let Some(event) = delta.event.as_ref() {
        if is_partial_event(event)
            && !allows_partial_for_foreground_session(foreground_session_ids, delta.session_id)
        {
            delta.event = None;
            if delta.turn.is_none() && delta.message.is_none() && delta.tool_summaries.is_empty() {
                return None;
            }
        }
    }
    Some(delta)
}
