use ctx_core::ids::{SessionId, TaskId};
use ctx_store::Store;

use self::events::seed_events;
use self::messages::seed_messages;
use self::tools::seed_tools;
use self::turns::{build_turns, insert_turns};

mod events;
mod messages;
mod queries;
mod tools;
mod turns;

pub(super) struct CtxUiSizedSeed {
    pub(super) turn_count: i64,
    pub(super) message_count: i64,
    pub(super) tool_count: i64,
    pub(super) event_count: i64,
    pub(super) tool_output_bytes: usize,
}

struct CtxUiTurnSeed {
    index: i64,
    run_id: String,
    turn_id: String,
    started_at: String,
    start_seq: i64,
    end_seq: Option<i64>,
    status: &'static str,
    tool_total: i64,
}

const BATCH: i64 = 500;

pub(super) async fn seed_ctx_ui_sized_session(
    store: &Store,
    session_id: SessionId,
    task_id: TaskId,
    seed: CtxUiSizedSeed,
) {
    let session_id = session_id.0.to_string();
    let task_id = task_id.0.to_string();
    let turns = build_turns(&seed);

    insert_turns(store, &session_id, &turns).await;
    seed_events(store, &session_id, &seed, &turns).await;
    seed_messages(store, &session_id, &task_id, &seed, &turns).await;
    seed_tools(store, &session_id, &seed, &turns).await;
}

pub(super) use queries::{latest_turn_id, tail_turn_ids};
