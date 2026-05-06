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
    SessionMetadata, SessionSnapshotSummary, SessionStatus, SessionTurnStatus, Task, TaskStatus,
    WorkspaceActiveTaskSummary, WorktreeVcsBaseResolution, WorktreeVcsBaseResolutionKind,
    WorktreeVcsComputeState, WorktreeVcsFreshness, WorktreeVcsGitStatusSummary,
    WorktreeVcsSnapshot, WorktreeVcsSummary, WorktreeVcsTouchedFile, WorktreeVcsTouchedFiles,
};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use tokio::sync::Mutex as AsyncMutex;

use ctx_workspace_active_snapshot::WorkspaceActiveSnapshotHub;

use crate::daemon::state::{WorkspaceRuntime, WorktreeVcsSchedulerRuntime};

struct FakeHydrationStore {
    snapshot_state: (i64, i64),
    tasks: Vec<WorkspaceActiveTaskSummary>,
    heads: Vec<SessionHeadSnapshot>,
    worktree_vcs_snapshots: Vec<WorktreeVcsSnapshot>,
    heads_error: Option<&'static str>,
    calls: Mutex<Vec<&'static str>>,
}

#[async_trait]
impl WorkspaceSnapshotHydrationStore for FakeHydrationStore {
    async fn get_snapshot_state(&self, _workspace_id: WorkspaceId) -> Result<(i64, i64)> {
        self.calls.lock().unwrap().push("snapshot_state");
        Ok(self.snapshot_state)
    }

    async fn list_active_page_for_hydration(
        &self,
        _workspace_id: WorkspaceId,
        limit: i64,
    ) -> Result<Vec<WorkspaceActiveTaskSummary>> {
        self.calls.lock().unwrap().push("active_page");
        assert_eq!(limit, i64::MAX);
        Ok(self.tasks.clone())
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

    async fn list_worktree_vcs_snapshots(
        &self,
        _workspace_id: WorkspaceId,
        worktree_ids: &HashSet<WorktreeId>,
    ) -> Result<Vec<WorktreeVcsSnapshot>> {
        self.calls.lock().unwrap().push("worktree_vcs");
        Ok(self
            .worktree_vcs_snapshots
            .iter()
            .filter(|snapshot| worktree_ids.contains(&snapshot.worktree_id))
            .cloned()
            .collect())
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

fn test_worktree_vcs_snapshot(worktree_id: WorktreeId) -> WorktreeVcsSnapshot {
    WorktreeVcsSnapshot {
        worktree_id,
        rev: 5,
        emitted_at_ms: 123,
        base_commit_sha: "base".to_string(),
        head_commit_sha: "head".to_string(),
        target_branch: Some("origin/main".to_string()),
        target_branch_commit_sha: Some("target".to_string()),
        base_resolution: WorktreeVcsBaseResolution {
            kind: WorktreeVcsBaseResolutionKind::MergeBase,
            target_source: None,
            error: None,
        },
        compute_state: WorktreeVcsComputeState::Ready,
        summary: WorktreeVcsSummary {
            file_count: Some(7),
            line_additions: Some(11),
            line_deletions: Some(4),
            line_count: Some(15),
        },
        git_status: WorktreeVcsGitStatusSummary {
            raw: "## main\n M edited.txt\n".to_string(),
            entries: vec![WorktreeVcsTouchedFile {
                path: "edited.txt".to_string(),
                orig_path: None,
                index_status: Some("M".to_string()),
                worktree_status: Some(" ".to_string()),
            }],
            ..Default::default()
        },
        touched_files: WorktreeVcsTouchedFiles::default(),
        touched_files_state: ctx_core::models::WorktreeVcsTouchedFilesState::Ready,
        freshness: WorktreeVcsFreshness::Fresh,
        available: true,
        unavailable_reason: None,
        schema_version: 1,
    }
}

#[tokio::test]
async fn workspace_hydration_payload_uses_canonical_page_and_preserves_snapshot_rev() {
    let workspace_id = WorkspaceId::new();
    let task_id = TaskId::new();
    let session_id = SessionId::new();
    let head = test_head(workspace_id, task_id, session_id);
    let mut summary = test_summary(workspace_id, task_id, session_id);
    summary.task.primary_worktree_id = Some(head.session.worktree_id);
    summary.primary_session.session.worktree_id = head.session.worktree_id;
    let store = FakeHydrationStore {
        snapshot_state: (17, 4),
        tasks: vec![summary],
        heads: vec![head.clone()],
        worktree_vcs_snapshots: vec![test_worktree_vcs_snapshot(head.session.worktree_id)],
        heads_error: None,
        calls: Mutex::new(Vec::new()),
    };

    let payload = load_workspace_snapshot_hydration_payload(&store, workspace_id)
        .await
        .expect("expected hydration payload");
    let calls = store.calls.lock().unwrap().clone();
    assert_eq!(
        calls,
        vec![
            "snapshot_state",
            "active_page",
            "active_heads",
            "worktree_vcs"
        ]
    );
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
    assert_eq!(payload.worktree_vcs_snapshots.len(), 1);
    assert_eq!(
        payload.worktree_vcs_snapshots[0].freshness,
        WorktreeVcsFreshness::Stale
    );
}

#[tokio::test]
async fn workspace_hydration_payload_propagates_active_head_batch_errors() {
    let workspace_id = WorkspaceId::new();
    let task_id = TaskId::new();
    let store = FakeHydrationStore {
        snapshot_state: (19, 5),
        tasks: vec![test_summary(workspace_id, task_id, SessionId::new())],
        heads: Vec::new(),
        worktree_vcs_snapshots: Vec::new(),
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
    let worktree_id = WorktreeId::new();
    let runtime = WorkspaceRuntime {
        worktree_vcs_enabled: true,
        file_completions_cache: AsyncMutex::new(HashMap::new()),
        workspace_file_completions_cache: AsyncMutex::new(HashMap::new()),
        git_status_snapshots: AsyncMutex::new(HashMap::new()),
        worktree_vcs_snapshots: AsyncMutex::new(HashMap::new()),
        worktree_vcs_active: AsyncMutex::new(HashMap::new()),
        worktree_vcs_refresh_locks: AsyncMutex::new(HashMap::new()),
        worktree_vcs_open_panes: AsyncMutex::new(HashMap::new()),
        worktree_vcs_summary_gen: AsyncMutex::new(HashMap::new()),
        worktree_vcs_runtime: AsyncMutex::new(HashMap::new()),
        worktree_vcs_scheduler: WorktreeVcsSchedulerRuntime::with_concurrency(1),
        git_status_watchers: AsyncMutex::new(HashSet::new()),
        workspace_active_snapshot: Arc::new(WorkspaceActiveSnapshotHub::new()),
        workspace_active_snapshot_cache: AsyncMutex::new(HashMap::new()),
        workspace_active_heads_cache: AsyncMutex::new(HashMap::new()),
        worktree_bootstrap_gates: AsyncMutex::new(HashMap::new()),
        attachment_materializations: AsyncMutex::new(HashMap::new()),
        attachment_materialization_generation: AtomicU64::new(0),
    };
    let payload = WorkspaceSnapshotHydrationPayload {
        snapshot_rev: 23,
        archived_rev: 6,
        tasks: vec![WorkspaceActiveTaskSummary {
            task: test_task(workspace_id, task_id, session_id),
            primary_session: SessionSnapshotSummary {
                session: SessionMetadata {
                    worktree_id,
                    ..test_session_metadata(workspace_id, task_id, session_id)
                },
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
        }],
        heads: vec![test_head(workspace_id, task_id, session_id)],
        worktree_vcs_snapshots: vec![test_worktree_vcs_snapshot(worktree_id)],
    };

    apply_workspace_snapshot_hydration_payload(&runtime, workspace_id, payload).await;

    let snapshot = runtime
        .workspace_active_snapshot
        .active_snapshot(workspace_id, i64::MAX)
        .await;
    assert_eq!(snapshot.snapshot_rev, 23);
    assert_eq!(snapshot.archived_rev, 6);
    assert_eq!(snapshot.active.tasks.len(), 1);
    assert_eq!(snapshot.worktree_vcs_snapshots.len(), 1);
    assert_eq!(
        snapshot.worktree_vcs_snapshots[0].freshness,
        WorktreeVcsFreshness::Stale
    );
    assert!(
        snapshot.worktree_vcs_snapshots[0].git_status.raw.is_empty(),
        "hydrated workspace snapshot should not expose raw git status text"
    );
    assert!(
        snapshot.worktree_vcs_snapshots[0]
            .git_status
            .entries
            .is_empty(),
        "hydrated workspace snapshot should not expose cached git status entries"
    );

    let heads = runtime
        .workspace_active_snapshot
        .active_heads(workspace_id)
        .await;
    assert_eq!(heads.snapshot_rev, 23);
    assert_eq!(heads.heads.len(), 1);
    assert_eq!(heads.heads[0].session.id, session_id);

    let cached = runtime
        .get_worktree_vcs_snapshot(worktree_id)
        .await
        .expect("expected hydrated runtime worktree vcs snapshot");
    assert_eq!(cached.freshness, WorktreeVcsFreshness::Stale);
    assert!(
        cached.git_status.raw.is_empty(),
        "hydrated runtime snapshot should not expose raw git status text"
    );
    assert!(
        cached.git_status.entries.is_empty(),
        "hydrated runtime snapshot should not expose cached git status entries"
    );
}
