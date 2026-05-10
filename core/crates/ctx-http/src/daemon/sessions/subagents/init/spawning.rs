use std::sync::Arc;

use ctx_core::ids::{SessionId, TurnId, WorktreeId};

use super::super::{finalize_subagent_invocation, run_subagent_child, SpawnedChild};
use crate::daemon::AppState;

pub(super) fn spawn_subagent_completion_tasks(
    state: &Arc<AppState>,
    spawned_children: &[SpawnedChild],
    invocation_id: String,
    tool_call_id: String,
    parent_id: SessionId,
    parent_turn_id: Option<TurnId>,
    parent_worktree_id: WorktreeId,
) {
    for spawned in spawned_children.iter().cloned() {
        let state_weak = Arc::downgrade(state);
        let invocation_id = invocation_id.clone();
        let tool_call_id = tool_call_id.clone();
        tokio::spawn(async move {
            if let Err(error) =
                run_subagent_child(&state_weak, spawned.child, parent_worktree_id).await
            {
                tracing::warn!(error = %error, "subagent execution failed");
            }
            if let Some(state) = state_weak.upgrade() {
                if let Err(error) = finalize_subagent_invocation(
                    &state,
                    &invocation_id,
                    &tool_call_id,
                    parent_id,
                    parent_turn_id,
                )
                .await
                {
                    tracing::warn!(error = %error, "failed to finalize subagent invocation");
                }
            }
        });
    }
}
