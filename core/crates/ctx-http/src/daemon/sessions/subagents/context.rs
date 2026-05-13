use std::sync::Arc;

use ctx_core::ids::{RunId, SessionId, WorktreeId};
use ctx_session_service::subagents::{
    legacy_context_window_metric_key, summarize_context_window as summarize_context_window_policy,
    SubagentContextWindowSummary,
};

use super::ContextWindowSummary;
use crate::daemon::AppState;

impl From<SubagentContextWindowSummary> for ContextWindowSummary {
    fn from(summary: SubagentContextWindowSummary) -> Self {
        Self {
            total: summary.total,
            used: summary.used,
            remaining: summary.remaining,
            utilization: summary.utilization,
        }
    }
}

fn summarize_context_window(metrics: &serde_json::Value) -> Option<ContextWindowSummary> {
    summarize_context_window_policy(metrics).map(Into::into)
}

pub(crate) async fn context_window_for_run(
    state: &Arc<AppState>,
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

pub(crate) async fn worktree_path_for_child(
    state: &Arc<AppState>,
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
