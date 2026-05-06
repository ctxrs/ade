use async_trait::async_trait;

use anyhow::Result;
use std::collections::HashSet;

use ctx_core::ids::{WorkspaceId, WorktreeId};
use ctx_core::models::{SessionHeadSnapshot, WorkspaceActiveTaskSummary, WorktreeVcsSnapshot};
use ctx_store::Store;

use crate::daemon::state::{AppState, WorkspaceRuntime};
use crate::daemon::StoreLookup;

#[derive(Debug)]
pub enum WorkspaceHydrationError {
    NotFound,
    Load(anyhow::Error),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspaceHydrationErrorKind {
    NotFound,
    Load,
}

impl WorkspaceHydrationError {
    pub fn kind(&self) -> WorkspaceHydrationErrorKind {
        match self {
            WorkspaceHydrationError::NotFound => WorkspaceHydrationErrorKind::NotFound,
            WorkspaceHydrationError::Load(_) => WorkspaceHydrationErrorKind::Load,
        }
    }
}

#[derive(Debug)]
struct WorkspaceSnapshotHydrationPayload {
    snapshot_rev: i64,
    archived_rev: i64,
    tasks: Vec<WorkspaceActiveTaskSummary>,
    heads: Vec<SessionHeadSnapshot>,
    worktree_vcs_snapshots: Vec<WorktreeVcsSnapshot>,
}

#[async_trait]
trait WorkspaceSnapshotHydrationStore {
    async fn get_snapshot_state(&self, workspace_id: WorkspaceId) -> Result<(i64, i64)>;
    async fn list_active_page_for_hydration(
        &self,
        workspace_id: WorkspaceId,
        limit: i64,
    ) -> Result<Vec<WorkspaceActiveTaskSummary>>;
    async fn list_active_heads(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<SessionHeadSnapshot>>;
    async fn list_worktree_vcs_snapshots(
        &self,
        workspace_id: WorkspaceId,
        worktree_ids: &HashSet<WorktreeId>,
    ) -> Result<Vec<WorktreeVcsSnapshot>>;
}

#[async_trait]
impl WorkspaceSnapshotHydrationStore for Store {
    async fn get_snapshot_state(&self, workspace_id: WorkspaceId) -> Result<(i64, i64)> {
        self.get_workspace_active_snapshot_state(workspace_id).await
    }

    async fn list_active_page_for_hydration(
        &self,
        workspace_id: WorkspaceId,
        limit: i64,
    ) -> Result<Vec<WorkspaceActiveTaskSummary>> {
        self.list_workspace_active_page_without_total(workspace_id, limit)
            .await
    }

    async fn list_active_heads(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<SessionHeadSnapshot>> {
        self.list_workspace_active_head_snapshots(workspace_id)
            .await
    }

    async fn list_worktree_vcs_snapshots(
        &self,
        workspace_id: WorkspaceId,
        worktree_ids: &HashSet<WorktreeId>,
    ) -> Result<Vec<WorktreeVcsSnapshot>> {
        self.list_workspace_worktree_vcs_snapshots(workspace_id, worktree_ids)
            .await
    }
}

fn active_worktree_ids_for_tasks(tasks: &[WorkspaceActiveTaskSummary]) -> HashSet<WorktreeId> {
    let mut worktree_ids = HashSet::new();
    for task in tasks {
        worktree_ids.insert(task.primary_session.session.worktree_id);
        for session in &task.sessions {
            worktree_ids.insert(session.session.worktree_id);
        }
    }
    worktree_ids
}

async fn load_workspace_snapshot_hydration_payload<S: WorkspaceSnapshotHydrationStore + Sync>(
    store: &S,
    workspace_id: WorkspaceId,
) -> Result<WorkspaceSnapshotHydrationPayload> {
    let payload_start = std::time::Instant::now();
    let snapshot_state_start = std::time::Instant::now();
    let (snapshot_rev, archived_rev) = store.get_snapshot_state(workspace_id).await?;
    let snapshot_state_ms = snapshot_state_start.elapsed().as_millis();
    let active_page_start = std::time::Instant::now();
    let tasks = store
        .list_active_page_for_hydration(workspace_id, i64::MAX)
        .await?;
    let active_page_ms = active_page_start.elapsed().as_millis();
    let active_worktree_ids = active_worktree_ids_for_tasks(&tasks);
    let active_heads_start = std::time::Instant::now();
    let heads = store.list_active_heads(workspace_id).await?;
    let active_heads_ms = active_heads_start.elapsed().as_millis();
    let worktree_vcs_start = std::time::Instant::now();
    let worktree_vcs_snapshots = store
        .list_worktree_vcs_snapshots(workspace_id, &active_worktree_ids)
        .await?
        .into_iter()
        .map(crate::daemon::workspaces::runtime::normalize_hydrated_worktree_vcs_snapshot)
        .collect::<Vec<_>>();
    let worktree_vcs_ms = worktree_vcs_start.elapsed().as_millis();
    if std::env::var_os("CTX_DEBUG_WORKSPACE_STREAM_TIMINGS").is_some() {
        eprintln!(
            "CTX_WS_TIMING hydration_payload workspace_id={} snapshot_state_ms={} active_page_ms={} active_heads_ms={} worktree_vcs_ms={} active_tasks={} active_heads={} worktree_vcs={} total_ms={}",
            workspace_id.0,
            snapshot_state_ms,
            active_page_ms,
            active_heads_ms,
            worktree_vcs_ms,
            tasks.len(),
            heads.len(),
            worktree_vcs_snapshots.len(),
            payload_start.elapsed().as_millis(),
        );
    }
    Ok(WorkspaceSnapshotHydrationPayload {
        snapshot_rev,
        archived_rev,
        tasks,
        heads,
        worktree_vcs_snapshots,
    })
}

async fn apply_workspace_snapshot_hydration_payload(
    runtime: &WorkspaceRuntime,
    workspace_id: WorkspaceId,
    payload: WorkspaceSnapshotHydrationPayload,
) {
    let worktree_vcs_snapshots = payload
        .worktree_vcs_snapshots
        .into_iter()
        .map(crate::daemon::workspaces::runtime::normalize_hydrated_worktree_vcs_snapshot)
        .collect::<Vec<_>>();
    runtime
        .workspace_active_snapshot
        .hydrate_snapshot(
            workspace_id,
            payload.snapshot_rev,
            payload.archived_rev,
            payload.tasks,
            payload.heads,
        )
        .await;
    runtime
        .workspace_active_snapshot
        .hydrate_worktree_vcs_snapshots(workspace_id, worktree_vcs_snapshots.clone())
        .await;
    runtime
        .hydrate_worktree_vcs_snapshots(worktree_vcs_snapshots)
        .await;
}

impl WorkspaceRuntime {
    pub async fn ensure_workspace_active_snapshot_hydrated(
        &self,
        state: &AppState,
        workspace_id: WorkspaceId,
    ) -> std::result::Result<(), WorkspaceHydrationError> {
        let hydration_start = std::time::Instant::now();
        if !self
            .workspace_active_snapshot
            .needs_hydration(workspace_id)
            .await
        {
            return Ok(());
        }
        let workspace_exists_start = std::time::Instant::now();
        let workspace_exists = match state.global_store().get_workspace(workspace_id).await {
            Ok(Some(_)) => true,
            Ok(None) => false,
            Err(err) => {
                tracing::warn!(
                    workspace_id = ?workspace_id,
                    "failed to check workspace existence before hydration: {err:#}"
                );
                return Err(WorkspaceHydrationError::Load(err));
            }
        };
        let workspace_exists_ms = workspace_exists_start.elapsed().as_millis();
        if !workspace_exists {
            return Err(WorkspaceHydrationError::NotFound);
        }
        let lookup_store_start = std::time::Instant::now();
        let store = match state.lookup_workspace_store(workspace_id).await {
            StoreLookup::Found(store) => store,
            StoreLookup::Missing | StoreLookup::Deleting => {
                return Err(WorkspaceHydrationError::NotFound);
            }
            StoreLookup::Unavailable(err) => {
                tracing::warn!(
                    workspace_id = ?workspace_id,
                    err = %err,
                    "failed to hydrate workspace snapshot (store lookup)"
                );
                return Err(WorkspaceHydrationError::Load(err));
            }
        };
        let lookup_store_ms = lookup_store_start.elapsed().as_millis();
        let load_payload_start = std::time::Instant::now();
        let payload = match load_workspace_snapshot_hydration_payload(&store, workspace_id).await {
            Ok(payload) => payload,
            Err(err) => {
                tracing::warn!(
                    workspace_id = ?workspace_id,
                    "failed to load workspace snapshot hydration payload: {err:#}"
                );
                return Err(WorkspaceHydrationError::Load(err));
            }
        };
        let load_payload_ms = load_payload_start.elapsed().as_millis();
        let active_task_count = payload.tasks.len();
        let active_head_count = payload.heads.len();
        let apply_payload_start = std::time::Instant::now();
        apply_workspace_snapshot_hydration_payload(self, workspace_id, payload).await;
        let apply_payload_ms = apply_payload_start.elapsed().as_millis();
        if std::env::var_os("CTX_DEBUG_WORKSPACE_STREAM_TIMINGS").is_some() {
            eprintln!(
                "CTX_WS_TIMING hydration workspace_id={} workspace_exists_ms={} lookup_store_ms={} load_payload_ms={} apply_payload_ms={} active_tasks={} active_heads={} total_ms={}",
                workspace_id.0,
                workspace_exists_ms,
                lookup_store_ms,
                load_payload_ms,
                apply_payload_ms,
                active_task_count,
                active_head_count,
                hydration_start.elapsed().as_millis(),
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
