use super::*;

pub(super) async fn emit_session_started_observability(
    handles: &TaskSessionHandles,
    session: &Session,
    task: &Task,
) {
    handles
        .sessions
        .emit_session_started_observability_for_task(session, task)
        .await;
}
