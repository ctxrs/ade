use std::sync::Arc;
use std::time::Instant;

use crate::daemon::scheduler::{QueuedMessage, SchedulerCommand};
use crate::daemon::DaemonState;
use ctx_core::models::{Message, Session};

pub(in crate::daemon::sessions::subagents) async fn dispatch_subagent_prompt(
    state: &Arc<DaemonState>,
    session: &Session,
    saved: &Message,
) {
    let tx = state.ensure_scheduler(session.clone()).await;
    let queued = QueuedMessage {
        message: saved.clone(),
        enqueued_at: Instant::now(),
        run_id: None,
    };
    let _ = tx.send(SchedulerCommand::Enqueue(queued)).await;
}
