use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use serde_json::json;
use tokio::sync::{watch, Mutex};

use ctx_core::models::Session;
use ctx_observability::ops_events::OpsEvent;
use ctx_session_tools::order_seq::OrderSeqState;

use crate::daemon::AppState;

use super::super::QueuedMessage;

pub(super) struct WorkerBootstrap {
    pub(super) store: ctx_store::Store,
    pub(super) order_seq_state: Arc<Mutex<OrderSeqState>>,
    pub(super) queue: VecDeque<QueuedMessage>,
    pub(super) event_head_rx: watch::Receiver<i64>,
    pub(super) workdir: PathBuf,
    pub(super) session_root_kind: String,
}

pub(super) async fn bootstrap_worker(
    state: &Arc<AppState>,
    session: &Session,
) -> Option<WorkerBootstrap> {
    let store = state.store_for_session(session.id).await.ok()?;
    let order_seq_state = state.sessions.get_order_seq_state(&store, session.id).await;
    let queue = load_initial_queue(&store, session).await;
    let event_head_rx = state
        .sessions
        .subscribe_session_event_head(session.id)
        .await;

    let worktree = store.get_worktree(session.worktree_id).await.ok()??;
    let workdir = PathBuf::from(worktree.root_path.clone());
    let is_worktree = worktree.vcs_ref.is_some() || worktree.git_branch.is_some();
    let session_root_kind = if is_worktree {
        "worktree".to_string()
    } else {
        "workspace_root".to_string()
    };
    emit_worktree_resolved_event(state, session, &workdir, &session_root_kind, &worktree);

    Some(WorkerBootstrap {
        store,
        order_seq_state,
        queue,
        event_head_rx,
        workdir,
        session_root_kind,
    })
}

async fn load_initial_queue(
    store: &ctx_store::Store,
    session: &Session,
) -> VecDeque<QueuedMessage> {
    let mut queue = VecDeque::new();
    let Ok(mut queued) = store.list_queued_messages_for_session(session.id).await else {
        return queue;
    };
    for message in queued.drain(..) {
        queue.push_back(QueuedMessage {
            message,
            enqueued_at: Instant::now(),
            run_id: None,
        });
    }
    queue
}

fn emit_worktree_resolved_event(
    state: &Arc<AppState>,
    session: &Session,
    workdir: &std::path::Path,
    session_root_kind: &str,
    worktree: &ctx_core::models::Worktree,
) {
    let mut worktree_event = OpsEvent::new("info", "worktree_resolved");
    worktree_event.session_id = Some(session.id.0.to_string());
    worktree_event.worktree_id = Some(session.worktree_id.0.to_string());
    worktree_event.worktree_root = Some(workdir.to_string_lossy().to_string());
    worktree_event.meta = Some(json!({
        "execution_environment": session.execution_environment.as_str(),
        "session_root_kind": session_root_kind,
        "vcs_kind": worktree.vcs_kind,
        "vcs_ref": worktree.vcs_ref,
        "git_branch": worktree.git_branch,
    }));
    state.telemetry.ops_events.emit(worktree_event);
}
