use std::sync::Arc;

use ctx_core::ids::WorkspaceId;
use ctx_core::models::{
    WorkspaceActiveHeadBatch, WorkspaceActiveSnapshot, WorkspaceActiveSnapshotEvent,
};
use tokio::sync::broadcast;

use crate::daemon::workspaces::{load_workspace_active_snapshot_state, WorkspaceHydrationError};
use crate::daemon::DaemonState;
use crate::daemon::WorkspaceStreamHandle;

#[derive(Clone, Debug)]
pub struct WorkspaceStreamSnapshotReadModel {
    pub active_snapshot: WorkspaceActiveSnapshot,
    pub active_heads: WorkspaceActiveHeadBatch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkspaceStreamInitialState {
    pub snapshot_rev: i64,
    pub archived_rev: i64,
}

pub async fn initial_stream_state(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
) -> WorkspaceStreamInitialState {
    let (snapshot_rev, archived_rev) =
        load_workspace_active_snapshot_state(state, workspace_id).await;
    WorkspaceStreamInitialState {
        snapshot_rev,
        archived_rev,
    }
}

pub async fn prepare_subscription_read_model(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
) -> Result<(), WorkspaceHydrationError> {
    state
        .ensure_workspace_active_snapshot_hydrated(workspace_id)
        .await?;
    crate::daemon::merge_queue::activate_workspace_merge_queue(state, workspace_id).await;
    Ok(())
}

pub async fn load_initial_snapshot_read_model(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
) -> Result<WorkspaceStreamSnapshotReadModel, WorkspaceHydrationError> {
    prepare_subscription_read_model(state, workspace_id).await?;
    let active_snapshot = state
        .workspaces
        .workspace_active_snapshot
        .active_snapshot(workspace_id, i64::MAX)
        .await;
    let active_heads = state
        .workspaces
        .workspace_active_snapshot
        .active_heads(workspace_id)
        .await;
    Ok(WorkspaceStreamSnapshotReadModel {
        active_snapshot,
        active_heads,
    })
}

impl WorkspaceStreamHandle {
    pub async fn subscribe_workspace_active_snapshot(
        &self,
        workspace_id: WorkspaceId,
    ) -> broadcast::Receiver<WorkspaceActiveSnapshotEvent> {
        self.state
            .workspaces
            .workspace_active_snapshot
            .subscribe(workspace_id)
            .await
    }

    pub async fn ensure_workspace_active_snapshot_hydrated(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<(), WorkspaceHydrationError> {
        self.state
            .ensure_workspace_active_snapshot_hydrated(workspace_id)
            .await
    }

    pub async fn activate_workspace_merge_queue(&self, workspace_id: WorkspaceId) {
        crate::daemon::merge_queue::activate_workspace_merge_queue(&self.state, workspace_id).await;
    }

    pub async fn load_workspace_active_snapshot_state(
        &self,
        workspace_id: WorkspaceId,
    ) -> (i64, i64) {
        load_workspace_active_snapshot_state(&self.state, workspace_id).await
    }

    pub async fn initial_stream_state(
        &self,
        workspace_id: WorkspaceId,
    ) -> WorkspaceStreamInitialState {
        initial_stream_state(&self.state, workspace_id).await
    }

    pub async fn load_initial_snapshot_read_model(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<WorkspaceStreamSnapshotReadModel, WorkspaceHydrationError> {
        load_initial_snapshot_read_model(&self.state, workspace_id).await
    }

    pub async fn workspace_active_snapshot(
        &self,
        workspace_id: WorkspaceId,
    ) -> WorkspaceActiveSnapshot {
        self.state
            .workspaces
            .workspace_active_snapshot
            .active_snapshot(workspace_id, i64::MAX)
            .await
    }

    pub async fn workspace_active_heads(
        &self,
        workspace_id: WorkspaceId,
    ) -> WorkspaceActiveHeadBatch {
        self.state
            .workspaces
            .workspace_active_snapshot
            .active_heads(workspace_id)
            .await
    }
}
