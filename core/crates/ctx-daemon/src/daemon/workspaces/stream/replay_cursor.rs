use std::collections::HashMap;
use std::sync::Arc;

use ctx_core::ids::{SessionId, WorkspaceId};
use ctx_workspace_active_snapshot::SessionReplayCursor;
pub use ctx_workspace_stream_service::replay_cursor::{
    active_head_cursors_from_snapshot_read_model, plan_resume_replay_cursor,
    WorkspaceStreamResumeReplayCursorPlan,
};

use super::read_model::WorkspaceStreamSnapshotReadModel;
use crate::daemon::DaemonState;
use crate::daemon::WorkspaceStreamHandle;

pub async fn head_only_snapshot_cursor(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
    session_id: SessionId,
    live_cursor: Option<SessionReplayCursor>,
    snapshot_cursor: Option<SessionReplayCursor>,
    include_initial_snapshot: bool,
) -> SessionReplayCursor {
    let snapshot_cursor = if include_initial_snapshot {
        snapshot_cursor
    } else {
        None
    };
    match snapshot_cursor {
        Some(cursor) => live_cursor.unwrap_or_default().cover(cursor),
        None => {
            let current_tail = session_replay_tail_cursor(state, workspace_id, session_id).await;
            live_cursor.unwrap_or_default().cover(current_tail)
        }
    }
}

pub async fn active_task_subscription_cursor(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
    session_id: SessionId,
) -> SessionReplayCursor {
    session_replay_tail_cursor(state, workspace_id, session_id).await
}

async fn session_replay_tail_cursor(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
    session_id: SessionId,
) -> SessionReplayCursor {
    state
        .workspaces
        .workspace_active_snapshot
        .session_replay_cursor(workspace_id, session_id)
        .await
}

impl WorkspaceStreamHandle {
    pub async fn session_replay_cursor(
        &self,
        workspace_id: WorkspaceId,
        session_id: SessionId,
    ) -> SessionReplayCursor {
        session_replay_tail_cursor(&self.state, workspace_id, session_id).await
    }

    pub fn active_head_cursors_from_snapshot_read_model(
        &self,
        read_model: &WorkspaceStreamSnapshotReadModel,
    ) -> HashMap<SessionId, SessionReplayCursor> {
        active_head_cursors_from_snapshot_read_model(read_model)
    }

    pub async fn active_task_subscription_cursor(
        &self,
        workspace_id: WorkspaceId,
        session_id: SessionId,
    ) -> SessionReplayCursor {
        active_task_subscription_cursor(&self.state, workspace_id, session_id).await
    }
}
