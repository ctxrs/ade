use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use ctx_core::ids::{RunId, SessionId, WorktreeId};
use ctx_subagent_service::{
    legacy_context_window_metric_key, summarize_context_window as summarize_context_window_policy,
};

use super::ContextWindowSummary;
use crate::daemon::DaemonState;

pub(in crate::daemon) type LegacyContextWindowRejectCounter =
    Arc<dyn Fn(String) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> + Send + Sync>;

fn summarize_context_window(metrics: &serde_json::Value) -> Option<ContextWindowSummary> {
    summarize_context_window_policy(metrics).map(Into::into)
}

pub async fn context_window_for_run(
    state: &Arc<DaemonState>,
    session_id: SessionId,
    run_id: RunId,
) -> Option<ContextWindowSummary> {
    let store = state.store_for_session(session_id).await.ok()?;
    let turn = store
        .get_latest_turn_for_run(session_id, run_id)
        .await
        .ok()
        .flatten()?;
    let metrics = turn.metrics_json.as_ref()?;
    if let Some(legacy_key) = legacy_context_window_metric_key(metrics) {
        state
            .emit_compat_payload_reject_counter(
                "sessions.context_window_summary",
                "legacy_context_window_key",
                Some(("legacy_key", legacy_key)),
            )
            .await;
    }
    summarize_context_window(metrics)
}

pub(in crate::daemon) async fn context_window_for_run_in_store(
    store: &ctx_store::Store,
    session_id: SessionId,
    run_id: RunId,
    emit_legacy_key_reject: &LegacyContextWindowRejectCounter,
) -> Option<ContextWindowSummary> {
    let turn = store
        .get_latest_turn_for_run(session_id, run_id)
        .await
        .ok()
        .flatten()?;
    let metrics = turn.metrics_json.as_ref()?;
    if let Some(legacy_key) = legacy_context_window_metric_key(metrics) {
        emit_legacy_key_reject(legacy_key.to_string()).await;
    }
    summarize_context_window(metrics)
}

pub async fn worktree_path_for_child(
    state: &Arc<DaemonState>,
    parent_worktree_id: WorktreeId,
    child_session_id: SessionId,
) -> Option<String> {
    let store = state.store_for_session(child_session_id).await.ok()?;
    let session = store.get_session(child_session_id).await.ok().flatten()?;
    if session.worktree_id == parent_worktree_id {
        return None;
    }
    let worktree = store
        .get_worktree(session.worktree_id)
        .await
        .ok()
        .flatten()?;
    Some(worktree.root_path)
}

pub(in crate::daemon) async fn worktree_path_for_child_in_store(
    store: &ctx_store::Store,
    parent_worktree_id: WorktreeId,
    child_session_id: SessionId,
) -> Option<String> {
    let session = store.get_session(child_session_id).await.ok().flatten()?;
    if session.worktree_id == parent_worktree_id {
        return None;
    }
    let worktree = store
        .get_worktree(session.worktree_id)
        .await
        .ok()
        .flatten()?;
    Some(worktree.root_path)
}
