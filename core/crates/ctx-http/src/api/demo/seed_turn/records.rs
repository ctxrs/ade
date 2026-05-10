use chrono::{DateTime, Duration, Utc};

use crate::api::demo::types::SeedTranscriptTurnReq;
use ctx_core::ids::{MessageId, RunId, SessionId, TaskId, TurnId};
use ctx_core::models::{Message, MessageDelivery, MessageRole};

pub(super) struct SeededTurn {
    pub(super) session_id: SessionId,
    pub(super) run_id: RunId,
    pub(super) turn_id: TurnId,
    pub(super) user_message: Message,
    pub(super) assistant_message: Message,
    pub(super) user_created_at: DateTime<Utc>,
    pub(super) assistant_created_at: DateTime<Utc>,
    pub(super) user_order_seq: i64,
    pub(super) assistant_order_seq: i64,
}

impl SeededTurn {
    pub(super) fn new(
        session_id: SessionId,
        task_id: TaskId,
        index: usize,
        base_time: DateTime<Utc>,
        turn: &SeedTranscriptTurnReq,
    ) -> Self {
        let run_id = RunId::new();
        let turn_id = TurnId::new();
        let user_created_at = base_time + Duration::seconds((index as i64) * 12);
        let assistant_created_at = user_created_at + Duration::seconds(4);
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
