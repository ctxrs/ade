use async_trait::async_trait;

use anyhow::Result;

use ctx_core::ids::WorkspaceId;
use ctx_core::models::{SessionHeadSnapshot, WorkspaceActiveTaskSummary};
use ctx_store::Store;
use ctx_workspace_active_snapshot::WorkspaceActiveSnapshotHub;

use crate::daemon::state::{AppState, WorkspaceRuntime};
use crate::daemon::StoreLookup;

#[derive(Debug)]
pub enum WorkspaceHydrationError {
    NotFound,
    Load(anyhow::Error),
}

impl WorkspaceHydrationError {
    pub fn status_code(&self) -> axum::http::StatusCode {
        match self {
            WorkspaceHydrationError::NotFound => axum::http::StatusCode::NOT_FOUND,
            WorkspaceHydrationError::Load(_) => axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

#[derive(Debug)]
struct WorkspaceSnapshotHydrationPayload {
    snapshot_rev: i64,
    archived_rev: i64,
    tasks: Vec<WorkspaceActiveTaskSummary>,
    heads: Vec<SessionHeadSnapshot>,
}

#[async_trait]
trait WorkspaceSnapshotHydrationStore {
    async fn get_snapshot_state(&self, workspace_id: WorkspaceId) -> Result<(i64, i64)>;
    async fn list_active_page(
        &self,
        workspace_id: WorkspaceId,
        limit: i64,
    ) -> Result<(Vec<WorkspaceActiveTaskSummary>, i64)>;
    async fn list_active_heads(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<SessionHeadSnapshot>>;
}

#[async_trait]
impl WorkspaceSnapshotHydrationStore for Store {
    async fn get_snapshot_state(&self, workspace_id: WorkspaceId) -> Result<(i64, i64)> {
        self.get_workspace_active_snapshot_state(workspace_id).await
    }

    async fn list_active_page(
        &self,
        workspace_id: WorkspaceId,
        limit: i64,
    ) -> Result<(Vec<WorkspaceActiveTaskSummary>, i64)> {
        self.list_workspace_active_page(workspace_id, limit).await
    }

    async fn list_active_heads(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<SessionHeadSnapshot>> {
        self.list_workspace_active_head_snapshots(workspace_id)
            .await
    }
}

async fn load_workspace_snapshot_hydration_payload<S: WorkspaceSnapshotHydrationStore + Sync>(
    store: &S,
    workspace_id: WorkspaceId,
) -> Result<WorkspaceSnapshotHydrationPayload> {
    let (snapshot_rev, archived_rev) = store.get_snapshot_state(workspace_id).await?;
    let (tasks, _) = store.list_active_page(workspace_id, i64::MAX).await?;
    let heads = store.list_active_heads(workspace_id).await?;
    Ok(WorkspaceSnapshotHydrationPayload {
        snapshot_rev,
        archived_rev,
        tasks,
        heads,
    })
}

async fn apply_workspace_snapshot_hydration_payload(
    hub: &WorkspaceActiveSnapshotHub,
    workspace_id: WorkspaceId,
    payload: WorkspaceSnapshotHydrationPayload,
) {
    hub.hydrate_snapshot(
        workspace_id,
        payload.snapshot_rev,
        payload.archived_rev,
        payload.tasks,
        payload.heads,
    )
    .await;
}

impl WorkspaceRuntime {
    pub async fn ensure_workspace_active_snapshot_hydrated(
        &self,
        state: &AppState,
        workspace_id: WorkspaceId,
    ) -> std::result::Result<(), WorkspaceHydrationError> {
        if !self
            .workspace_active_snapshot
            .needs_hydration(workspace_id)
            .await
        {
            return Ok(());
        }
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
        if !workspace_exists {
            return Err(WorkspaceHydrationError::NotFound);
        }
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
        apply_workspace_snapshot_hydration_payload(
            self.workspace_active_snapshot.as_ref(),
            workspace_id,
            payload,
        )
        .await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply_workspace_snapshot_hydration_payload, load_workspace_snapshot_hydration_payload,
        WorkspaceSnapshotHydrationPayload, WorkspaceSnapshotHydrationStore,
    };
    use anyhow::{anyhow, Result};
    use async_trait::async_trait;
    use chrono::Utc;
    use ctx_core::ids::{SessionId, TaskId, WorkspaceId, WorktreeId};
    use ctx_core::models::{
        ExecutionEnvironment, SessionActivityState, SessionHeadSnapshot, SessionHeadWindow,
        SessionMetadata, SessionSnapshotSummary, SessionStatus, SessionTurnStatus, Task,
        TaskStatus, WorkspaceActiveTaskSummary,
    };
    use std::sync::Mutex;

    use ctx_workspace_active_snapshot::WorkspaceActiveSnapshotHub;

    struct FakeHydrationStore {
        snapshot_state: (i64, i64),
        tasks: Vec<WorkspaceActiveTaskSummary>,
        heads: Vec<SessionHeadSnapshot>,
        heads_error: Option<&'static str>,
        calls: Mutex<Vec<&'static str>>,
    }

    #[async_trait]
    impl WorkspaceSnapshotHydrationStore for FakeHydrationStore {
        async fn get_snapshot_state(&self, _workspace_id: WorkspaceId) -> Result<(i64, i64)> {
            self.calls.lock().unwrap().push("snapshot_state");
            Ok(self.snapshot_state)
        }

        async fn list_active_page(
            &self,
            _workspace_id: WorkspaceId,
            limit: i64,
        ) -> Result<(Vec<WorkspaceActiveTaskSummary>, i64)> {
            self.calls.lock().unwrap().push("active_page");
            assert_eq!(limit, i64::MAX);
            Ok((self.tasks.clone(), self.tasks.len() as i64))
        }

        async fn list_active_heads(
            &self,
            _workspace_id: WorkspaceId,
        ) -> Result<Vec<SessionHeadSnapshot>> {
            self.calls.lock().unwrap().push("active_heads");
            if let Some(message) = self.heads_error {
                return Err(anyhow!(message));
            }
            Ok(self.heads.clone())
        }
    }

    fn test_task(workspace_id: WorkspaceId, task_id: TaskId, session_id: SessionId) -> Task {
        let now = Utc::now();
        Task {
            id: task_id,
            workspace_id,
            title: "hydrate".to_string(),
            description: None,
            status: TaskStatus::Pending,
            exec_plan_id: None,
            primary_session_id: Some(session_id),
            primary_worktree_id: Some(WorktreeId::new()),
            created_at: now,
            updated_at: now,
            archived_at: None,
            assistant_seen_at: None,
            last_activity_at: Some(now),
            last_assistant_message_at: None,
            has_active_session: true,
        }
    }

    fn test_session_metadata(
        workspace_id: WorkspaceId,
        task_id: TaskId,
        session_id: SessionId,
    ) -> SessionMetadata {
        let now = Utc::now();
        SessionMetadata {
            id: session_id,
            task_id,
            workspace_id,
            worktree_id: WorktreeId::new(),
            execution_environment: ExecutionEnvironment::Host,
            parent_session_id: None,
            relationship: None,
            provider_id: "fake".to_string(),
            model_id: "fake-model".to_string(),
            reasoning_effort: None,
            title: String::new(),
            agent_role: "assistant".to_string(),
            status: SessionStatus::Active,
            provider_session_ref: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn test_summary(
        workspace_id: WorkspaceId,
        task_id: TaskId,
        session_id: SessionId,
    ) -> WorkspaceActiveTaskSummary {
        WorkspaceActiveTaskSummary {
            task: test_task(workspace_id, task_id, session_id),
            primary_session: SessionSnapshotSummary {
                session: test_session_metadata(workspace_id, task_id, session_id),
                last_message_at: None,
                last_message_preview: Some("canonical-summary".to_string()),
                last_event_seq: Some(44),
                projection_rev: 44,
                state_rev: 44,
                activity: SessionActivityState {
                    is_working: false,
                    last_turn_status: Some(SessionTurnStatus::Completed),
                },
                unread: None,
            },
            primary_session_head: None,
            sessions: Vec::new(),
            sort_at: Utc::now(),
        }
    }

    fn test_head(
        workspace_id: WorkspaceId,
        task_id: TaskId,
        session_id: SessionId,
    ) -> SessionHeadSnapshot {
        SessionHeadSnapshot {
            session: test_session_metadata(workspace_id, task_id, session_id),
            turns: Vec::new(),
            tool_summaries: Vec::new(),
            events: Vec::new(),
            messages: Vec::new(),
            last_event_seq: 44,
            projection_rev: 44,
            state_rev: 44,
            activity: SessionActivityState {
                is_working: false,
                last_turn_status: Some(SessionTurnStatus::Completed),
            },
            has_more_turns: false,
            history_cursor: None,
            has_more_history: false,
            summary_checkpoint: None,
            head_window: SessionHeadWindow::default(),
        }
    }

    #[tokio::test]
    async fn workspace_hydration_payload_uses_canonical_page_and_preserves_snapshot_rev() {
        let workspace_id = WorkspaceId::new();
        let task_id = TaskId::new();
        let session_id = SessionId::new();
        let summary = test_summary(workspace_id, task_id, session_id);
        let head = test_head(workspace_id, task_id, session_id);
        let store = FakeHydrationStore {
            snapshot_state: (17, 4),
            tasks: vec![summary],
            heads: vec![head.clone()],
            heads_error: None,
            calls: Mutex::new(Vec::new()),
        };

        let payload = load_workspace_snapshot_hydration_payload(&store, workspace_id)
            .await
            .expect("expected hydration payload");
        let calls = store.calls.lock().unwrap().clone();
        assert_eq!(calls, vec!["snapshot_state", "active_page", "active_heads"]);
        assert_eq!(payload.snapshot_rev, 17);
        assert_eq!(payload.archived_rev, 4);
        assert_eq!(payload.tasks.len(), 1);
        assert_eq!(
            payload.tasks[0]
                .primary_session
                .last_message_preview
                .as_deref(),
            Some("canonical-summary")
        );
        assert_eq!(payload.tasks[0].primary_session.last_event_seq, Some(44));
        assert_eq!(payload.heads.len(), 1);
        assert_eq!(payload.heads[0].session.id, head.session.id);
        assert_eq!(payload.heads[0].last_event_seq, head.last_event_seq);
    }

    #[tokio::test]
    async fn workspace_hydration_payload_propagates_active_head_batch_errors() {
        let workspace_id = WorkspaceId::new();
        let task_id = TaskId::new();
        let store = FakeHydrationStore {
            snapshot_state: (19, 5),
            tasks: vec![test_summary(workspace_id, task_id, SessionId::new())],
            heads: Vec::new(),
            heads_error: Some("head decode failed"),
            calls: Mutex::new(Vec::new()),
        };

        let err = load_workspace_snapshot_hydration_payload(&store, workspace_id)
            .await
            .expect_err("expected hydration to fail");
        assert!(err.to_string().contains("head decode failed"));
    }

    #[tokio::test]
    async fn applying_workspace_hydration_payload_seeds_hub_with_loaded_snapshot_rev() {
        let workspace_id = WorkspaceId::new();
        let task_id = TaskId::new();
        let session_id = SessionId::new();
        let hub = WorkspaceActiveSnapshotHub::new();
        let payload = WorkspaceSnapshotHydrationPayload {
            snapshot_rev: 23,
            archived_rev: 6,
            tasks: vec![test_summary(workspace_id, task_id, session_id)],
            heads: vec![test_head(workspace_id, task_id, session_id)],
        };

        apply_workspace_snapshot_hydration_payload(&hub, workspace_id, payload).await;

        let snapshot = hub.active_snapshot(workspace_id, i64::MAX).await;
        assert_eq!(snapshot.snapshot_rev, 23);
        assert_eq!(snapshot.archived_rev, 6);
        assert_eq!(snapshot.active.tasks.len(), 1);

        let heads = hub.active_heads(workspace_id).await;
        assert_eq!(heads.snapshot_rev, 23);
        assert_eq!(heads.heads.len(), 1);
        assert_eq!(heads.heads[0].session.id, session_id);
    }
}
