use serde::Serialize;

use super::*;

#[derive(Clone, Copy, Debug)]
pub(crate) struct SessionHeadLimits {
    pub turn_limit: usize,
    pub message_limit: usize,
    pub event_limit: usize,
    pub byte_limit: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct SessionHeadMaterialization {
    pub head_rev: i64,
    pub last_event_seq: i64,
    pub turns: Vec<SessionTurn>,
    pub tool_summaries: Vec<SessionTurnToolSummary>,
    pub events: Vec<SessionEvent>,
    pub messages: Vec<Message>,
    pub has_more_turns: bool,
    pub head_window: SessionHeadWindow,
}

impl SessionHeadMaterialization {
    pub(crate) fn from_head(head: &SessionHead) -> Self {
        Self {
            head_rev: head.projection_rev,
            last_event_seq: head.last_event_seq,
            turns: head.turns.clone(),
            tool_summaries: head.tool_summaries.clone(),
            events: head.events.clone(),
            messages: head.messages.clone(),
            has_more_turns: head.has_more_turns,
            head_window: head.head_window.clone(),
        }
    }

    pub(crate) fn into_session_head(
        self,
        session: Session,
        projection_rev: i64,
        summary_checkpoint: Option<SessionSummaryCheckpoint>,
    ) -> SessionHead {
        let last_status = self.turns.last().map(|t| t.status.clone());
        let has_running_turn = self.turns.iter().any(|turn| {
            matches!(
                turn.status,
                SessionTurnStatus::Starting | SessionTurnStatus::Running
            )
        });
        let activity = derive_activity_from_status(last_status, has_running_turn);
        SessionHead {
            session,
            turns: self.turns,
            tool_summaries: self.tool_summaries,
            events: self.events,
            messages: self.messages,
            last_event_seq: self.last_event_seq,
            projection_rev,
            activity,
            has_more_turns: self.has_more_turns,
            summary_checkpoint,
            head_window: self.head_window,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ActiveSnapshotHeadProjection {
    pub head_rev: i64,
    pub last_event_seq: i64,
    pub turns: Vec<SessionTurn>,
    pub tool_summaries: Vec<SessionTurnToolSummary>,
    pub messages: Vec<Message>,
    pub has_more_turns: bool,
    pub head_window: SessionHeadWindow,
    pub summary_checkpoint: Option<SessionSummaryCheckpoint>,
}

impl ActiveSnapshotHeadProjection {
    pub(crate) fn from_head(head: &SessionHead) -> Self {
        Self {
            head_rev: head.projection_rev,
            last_event_seq: head.last_event_seq,
            turns: head.turns.clone(),
            tool_summaries: head.tool_summaries.clone(),
            messages: head.messages.clone(),
            has_more_turns: head.has_more_turns,
            head_window: head.head_window.clone(),
            summary_checkpoint: head.summary_checkpoint.clone(),
        }
    }

    pub(crate) fn into_session_head(self, session: Session, projection_rev: i64) -> SessionHead {
        let last_status = self.turns.last().map(|t| t.status.clone());
        let has_running_turn = self.turns.iter().any(|turn| {
            matches!(
                turn.status,
                SessionTurnStatus::Starting | SessionTurnStatus::Running
            )
        });
        let activity = derive_activity_from_status(last_status, has_running_turn);
        SessionHead {
            session,
            turns: self.turns,
            tool_summaries: self.tool_summaries,
            events: Vec::new(),
            messages: self.messages,
            last_event_seq: self.last_event_seq,
            projection_rev,
            activity,
            has_more_turns: self.has_more_turns,
            summary_checkpoint: self.summary_checkpoint,
            head_window: self.head_window,
        }
    }
}

#[derive(Serialize)]
struct SessionHeadWindowPayload<'a> {
    turns: &'a [SessionTurn],
    tool_summaries: &'a [SessionTurnToolSummary],
    events: &'a [SessionEvent],
    messages: &'a [Message],
}

pub(crate) fn head_window_bytes(
    turns: &[SessionTurn],
    tool_summaries: &[SessionTurnToolSummary],
    events: &[SessionEvent],
    messages: &[Message],
) -> usize {
    let payload = SessionHeadWindowPayload {
        turns,
        tool_summaries,
        events,
        messages,
    };
    serde_json::to_vec(&payload)
        .map(|bytes| bytes.len())
        .unwrap_or(0)
}
