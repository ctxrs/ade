use chrono::{Duration as ChronoDuration, Utc};
use ctx_core::ids::{MessageId, RunId, SessionId, TaskId, TurnId};
use ctx_core::models::{
    Message, MessageDelivery, MessageRole, SessionEventType, SessionTurn, SessionTurnStatus,
};
use ctx_store::Store;

use crate::daemon::handle::SessionsHandle;

pub struct DemoSeedTranscript {
    pub session_title: Option<String>,
    pub task_title: Option<String>,
    pub append: bool,
    pub refresh: bool,
    pub materialize_tail_turns: Option<usize>,
    pub turns: Vec<DemoSeedTranscriptTurn>,
}

pub struct DemoSeedTranscriptTurn {
    pub user: String,
    pub assistant: String,
    pub context_window: Option<serde_json::Value>,
}

pub struct DemoSeedTranscriptResult {
    pub seeded_turns: usize,
    pub seeded_messages: usize,
    pub seeded_events: usize,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum DemoSeedTranscriptError {
    SessionNotFound,
    SessionAlreadyHasMessages,
    StoreUnavailable,
    InspectMessages,
    UpdateSessionTitle,
    UpdateTaskTitle,
    ReloadSession,
    InsertUserMessage,
    InsertAssistantMessage,
    InsertSessionTurn,
    AppendUserEvent,
    AppendTurnStartedEvent,
    AppendAssistantEvent,
    AppendDoneEvent,
    AppendTurnFinishedEvent,
}

impl SessionsHandle {
    pub async fn seed_demo_transcript(
        &self,
        session_id: SessionId,
        seed: DemoSeedTranscript,
    ) -> Result<DemoSeedTranscriptResult, DemoSeedTranscriptError> {
        let store = self
            .store_for_session(session_id)
            .await
            .map_err(|_| DemoSeedTranscriptError::SessionNotFound)?;
        let mut session = store
            .get_session(session_id)
            .await
            .map_err(|_| DemoSeedTranscriptError::StoreUnavailable)?
            .ok_or(DemoSeedTranscriptError::SessionNotFound)?;

        if !seed.append
            && !store
                .list_messages_for_session(session_id)
                .await
                .map_err(|_| DemoSeedTranscriptError::InspectMessages)?
                .is_empty()
        {
            return Err(DemoSeedTranscriptError::SessionAlreadyHasMessages);
        }

        if let Some(session_title) = seed
            .session_title
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            store
                .update_session_title(session_id, session_title.to_string())
                .await
                .map_err(|_| DemoSeedTranscriptError::UpdateSessionTitle)?;
        }

        if let Some(task_title) = seed
            .task_title
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            store
                .update_task_title(session.task_id, task_title.to_string())
                .await
                .map_err(|_| DemoSeedTranscriptError::UpdateTaskTitle)?;
        }

        session = store
            .get_session(session_id)
            .await
            .map_err(|_| DemoSeedTranscriptError::ReloadSession)?
            .ok_or(DemoSeedTranscriptError::SessionNotFound)?;
        self.remember_session_meta(&session).await;

        let mut seeded_messages = 0usize;
        let mut seeded_events = 0usize;
        let base_time = Utc::now() - ChronoDuration::minutes(seed.turns.len() as i64);
        let materialize_from_index = seed
            .materialize_tail_turns
            .map(|tail| seed.turns.len().saturating_sub(tail));

        for (index, turn) in seed.turns.iter().enumerate() {
            let materialize_turn = materialize_from_index
                .map(|from_index| index >= from_index)
                .unwrap_or(true);
            let seeded = DemoSeededTurn::new(session_id, session.task_id, index, base_time, turn);
            let counts = self
                .seed_demo_transcript_turn(&store, &seeded, turn, materialize_turn)
                .await?;
            seeded_messages += counts.seeded_messages;
            seeded_events += counts.seeded_events;
        }

        if seed.refresh {
            self.refresh_session_head_cache(session_id).await;
            if let Err(error) = self.emit_workspace_task_upsert(session.task_id).await {
                tracing::warn!(task_id = %session.task_id.0, "workspace active snapshot refresh failed after demo transcript seed: {error:?}");
            }
        }

        Ok(DemoSeedTranscriptResult {
            seeded_turns: seed.turns.len(),
            seeded_messages,
            seeded_events,
        })
    }

    async fn seed_demo_transcript_turn(
        &self,
        store: &Store,
        seeded: &DemoSeededTurn,
        turn: &DemoSeedTranscriptTurn,
        materialize_turn: bool,
    ) -> Result<DemoSeedTranscriptResult, DemoSeedTranscriptError> {
        let mut seeded_messages = 0usize;
        let mut seeded_events = 0usize;

        if materialize_turn {
            store
                .insert_message(seeded.user_message.clone())
                .await
                .map_err(|_| DemoSeedTranscriptError::InsertUserMessage)?;
            seeded_messages += 1;
        }

        let user_event = store
            .append_session_event(
                seeded.session_id,
                Some(seeded.run_id),
                Some(seeded.turn_id),
                SessionEventType::UserMessage,
                serde_json::json!({
                    "message_id": seeded.user_message.id.0,
                    "content": &seeded.user_message.content,
                    "delivery": &seeded.user_message.delivery,
                    "attachments": [],
                    "order_seq": seeded.user_order_seq,
                }),
            )
            .await
            .map_err(|_| DemoSeedTranscriptError::AppendUserEvent)?;
        seeded_events += 1;

        store
            .append_session_event(
                seeded.session_id,
                Some(seeded.run_id),
                Some(seeded.turn_id),
                SessionEventType::TurnStarted,
                serde_json::json!({
                    "message_id": seeded.user_message.id.0,
                }),
            )
            .await
            .map_err(|_| DemoSeedTranscriptError::AppendTurnStartedEvent)?;
        seeded_events += 1;

        if materialize_turn {
            store
                .insert_message(seeded.assistant_message.clone())
                .await
                .map_err(|_| DemoSeedTranscriptError::InsertAssistantMessage)?;
            seeded_messages += 1;
        }

        store
            .append_session_event(
                seeded.session_id,
                Some(seeded.run_id),
                Some(seeded.turn_id),
                SessionEventType::AssistantMessageInserted,
                serde_json::json!({
                    "message_id": seeded.assistant_message.id.0,
                    "content": &seeded.assistant_message.content,
                    "attachments": [],
                    "delivery": &seeded.assistant_message.delivery,
                    "order_seq": seeded.assistant_order_seq,
                    "turn_sequence": 1,
                }),
            )
            .await
            .map_err(|_| DemoSeedTranscriptError::AppendAssistantEvent)?;
        seeded_events += 1;

        let done_event = store
            .append_session_event(
                seeded.session_id,
                Some(seeded.run_id),
                Some(seeded.turn_id),
                SessionEventType::Done,
                {
                    let mut payload = serde_json::json!({ "status": "completed" });
                    if let Some(metrics) = turn.context_window.as_ref() {
                        if let Some(obj) = payload.as_object_mut() {
                            obj.insert("context_window".to_string(), metrics.clone());
                        }
                    }
                    payload
                },
            )
            .await
            .map_err(|_| DemoSeedTranscriptError::AppendDoneEvent)?;
        seeded_events += 1;

        store
            .append_session_event(
                seeded.session_id,
                Some(seeded.run_id),
                Some(seeded.turn_id),
                SessionEventType::TurnFinished,
                serde_json::json!({
                    "message_id": seeded.user_message.id.0,
                    "status": "completed",
                }),
            )
            .await
            .map_err(|_| DemoSeedTranscriptError::AppendTurnFinishedEvent)?;
        seeded_events += 1;

        if materialize_turn {
            store
                .insert_session_turn(SessionTurn {
                    turn_id: seeded.turn_id,
                    session_id: seeded.session_id,
                    run_id: Some(seeded.run_id),
                    user_message_id: Some(seeded.user_message.id),
                    status: SessionTurnStatus::Completed,
                    start_seq: Some(user_event.seq),
                    end_seq: Some(done_event.seq),
                    started_at: seeded.user_created_at,
                    updated_at: seeded.assistant_created_at,
                    assistant_partial: None,
                    thought_partial: None,
                    metrics_json: turn.context_window.clone(),
                    failure: None,
                    tool_total: 0,
                    tool_pending: 0,
                    tool_running: 0,
                    tool_completed: 0,
                    tool_failed: 0,
                })
                .await
                .map_err(|_| DemoSeedTranscriptError::InsertSessionTurn)?;
        }

        Ok(DemoSeedTranscriptResult {
            seeded_turns: 1,
            seeded_messages,
            seeded_events,
        })
    }
}

struct DemoSeededTurn {
    session_id: SessionId,
    run_id: RunId,
    turn_id: TurnId,
    user_message: Message,
    assistant_message: Message,
    user_created_at: chrono::DateTime<Utc>,
    assistant_created_at: chrono::DateTime<Utc>,
    user_order_seq: i64,
    assistant_order_seq: i64,
}

impl DemoSeededTurn {
    fn new(
        session_id: SessionId,
        task_id: TaskId,
        index: usize,
        base_time: chrono::DateTime<Utc>,
        turn: &DemoSeedTranscriptTurn,
    ) -> Self {
        let run_id = RunId::new();
        let turn_id = TurnId::new();
        let user_created_at = base_time + ChronoDuration::seconds((index as i64) * 12);
        let assistant_created_at = user_created_at + ChronoDuration::seconds(4);
        let user_order_seq = (index as i64) * 2 + 1;
        let assistant_order_seq = user_order_seq + 1;

        let user_message = Message {
            id: MessageId::new(),
            session_id,
            task_id,
            run_id: Some(run_id),
            turn_id: Some(turn_id),
            turn_sequence: Some(0),
            order_seq: Some(user_order_seq),
            role: MessageRole::User,
            content: turn.user.clone(),
            attachments: Vec::new(),
            delivery: MessageDelivery::Immediate,
            delivered_at: Some(user_created_at),
            created_at: user_created_at,
        };
        let assistant_message = Message {
            id: MessageId::new(),
            session_id,
            task_id,
            run_id: Some(run_id),
            turn_id: Some(turn_id),
            turn_sequence: Some(1),
            order_seq: Some(assistant_order_seq),
            role: MessageRole::Assistant,
            content: turn.assistant.clone(),
            attachments: Vec::new(),
            delivery: MessageDelivery::Immediate,
            delivered_at: Some(assistant_created_at),
            created_at: assistant_created_at,
        };

        Self {
            session_id,
            run_id,
            turn_id,
            user_message,
            assistant_message,
            user_created_at,
            assistant_created_at,
            user_order_seq,
            assistant_order_seq,
        }
    }
}
