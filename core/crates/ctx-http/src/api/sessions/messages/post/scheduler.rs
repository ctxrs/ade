use super::super::*;
use super::persistence::PersistedPostMessage;
use crate::daemon::sessions::title_generation::schedule_session_title_generation;
use ctx_store::Store;

pub(super) async fn enqueue_message_for_scheduler(
    state: &Arc<AppState>,
    store: &Store,
    session: Session,
    persisted: &PersistedPostMessage,
    run_id_header: Option<String>,
) {
    let tx = state.ensure_scheduler(session.clone()).await;
    let queued = crate::daemon::scheduler::QueuedMessage {
        message: persisted.saved.clone(),
        enqueued_at: Instant::now(),
        run_id: run_id_header,
    };
    let _ = tx.send(SchedulerCommand::Enqueue(queued)).await;

    if let Ok(count) = store.count_user_messages_for_session(session.id).await {
        if count == 1 {
            let prompt = persisted.saved.content.clone();
            let _ =
                schedule_session_title_generation(state.clone(), session.clone(), prompt, false)
                    .await;
        }
    }
}
