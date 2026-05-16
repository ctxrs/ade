use super::*;
use chrono::Utc;
use ctx_daemon::test_support::TestDaemon;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::AtomicI64;
use std::sync::Arc;

fn cursor(last_event_seq: i64, projection_rev: i64) -> SessionCursor {
    SessionCursor {
        last_sent: SessionReplayCursor {
            last_event_seq,
            projection_rev,
        },
    }
}

fn test_runtime(
    subscription_state: WorkspaceActiveSubscriptionState,
    subscriptions: HashMap<SessionId, SessionCursor>,
) -> WorkspaceStreamRuntime {
    WorkspaceStreamRuntime {
        priority_control: Arc::new(StreamQueue::new(
            WORKSPACE_STREAM_QUEUE_LIMIT,
            WORKSPACE_STREAM_QUEUE_MAX_AGE,
        )),
        control: Arc::new(StreamQueue::new(
            WORKSPACE_STREAM_QUEUE_LIMIT,
            WORKSPACE_STREAM_QUEUE_MAX_AGE,
        )),
        foreground_head_buffer: Arc::new(HeadBatchBuffer::new()),
        background_head_buffer: Arc::new(HeadBatchBuffer::new()),
        summary_buffer: Arc::new(SummaryBatchBuffer::new(HEAD_BATCH_TOTAL_LIMIT)),
        send_control: Arc::new(StreamSendControl::new()),
        subscriptions,
        last_subscription_fingerprint: None,
        subscription_state,
        reset_queued: false,
        latest_snapshot_rev: Arc::new(AtomicI64::new(0)),
    }
}

fn task(workspace_id: WorkspaceId, task_id: TaskId, session_id: SessionId) -> Task {
    Task {
        id: task_id,
        workspace_id,
        title: "task".to_string(),
        description: None,
        status: TaskStatus::Running,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        exec_plan_id: None,
        primary_session_id: Some(session_id),
        primary_worktree_id: None,
        archived_at: None,
        assistant_seen_at: None,
        last_activity_at: None,
        last_assistant_message_at: None,
        has_active_session: true,
    }
}

fn session_summary(workspace_id: WorkspaceId, session_id: SessionId) -> SessionSnapshotSummary {
    SessionSnapshotSummary {
        session: SessionMetadata {
            id: session_id,
            task_id: TaskId::new(),
            workspace_id,
            worktree_id: WorktreeId::new(),
            execution_environment: ExecutionEnvironment::Host,
            parent_session_id: None,
            relationship: None,
            provider_id: "fake".to_string(),
            model_id: "fake-model".to_string(),
            reasoning_effort: None,
            title: "test".to_string(),
            agent_role: "assistant".to_string(),
            status: SessionStatus::Active,
            provider_session_ref: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        },
        last_message_at: None,
        last_message_preview: None,
        last_event_seq: None,
        projection_rev: 0,
        state_rev: 0,
        activity: SessionActivityState::default(),
        unread: None,
    }
}

fn active_task_summary(
    workspace_id: WorkspaceId,
    task_id: TaskId,
    session_id: SessionId,
) -> WorkspaceActiveTaskSummary {
    WorkspaceActiveTaskSummary {
        task: task(workspace_id, task_id, session_id),
        primary_session: session_summary(workspace_id, session_id),
        primary_session_head: None,
        sessions: Vec::new(),
        sort_at: Utc::now(),
    }
}

async fn test_workspace_stream() -> (tempfile::TempDir, WorkspaceStreamHandle) {
    let root = tempfile::tempdir().expect("tempdir");
    let daemon =
        TestDaemon::new_for_test(root.path().to_path_buf(), "http://127.0.0.1:0".to_string())
            .await
            .expect("test daemon should start");
    (root, daemon.handle().workspace_stream())
}

#[tokio::test]
async fn adapter_applies_active_task_insert_and_preserves_existing_cursor() {
    let (_root, state) = test_workspace_stream().await;
    let workspace_id = WorkspaceId::new();
    let task_id = TaskId::new();
    let session_id = SessionId::new();
    let mut subscription_state = WorkspaceActiveSubscriptionState::default();
    subscription_state.active_scope = true;
    let mut runtime = test_runtime(subscription_state, HashMap::new());

    let should_route = update_workspace_stream_subscriptions_for_event(
        &state,
        workspace_id,
        &mut runtime,
        &WorkspaceActiveSnapshotEvent::ActiveTaskUpsert {
            workspace_id,
            snapshot_rev: 1,
            task: Box::new(active_task_summary(workspace_id, task_id, session_id)),
        },
    )
    .await;

    assert!(should_route);
    assert_eq!(
        runtime
            .subscription_state
            .active_task_sessions
            .get(&task_id)
            .copied(),
        Some(session_id),
    );
    assert!(runtime.subscriptions.contains_key(&session_id));

    let existing_cursor = cursor(12, 13);
    runtime.subscriptions.insert(
        session_id,
        cursor(
            existing_cursor.last_sent.last_event_seq,
            existing_cursor.last_sent.projection_rev,
        ),
    );
    let should_route = update_workspace_stream_subscriptions_for_event(
        &state,
        workspace_id,
        &mut runtime,
        &WorkspaceActiveSnapshotEvent::ActiveTaskUpsert {
            workspace_id,
            snapshot_rev: 2,
            task: Box::new(active_task_summary(workspace_id, task_id, session_id)),
        },
    )
    .await;

    assert!(should_route);
    assert_eq!(
        runtime
            .subscriptions
            .get(&session_id)
            .map(|cursor| cursor.last_sent),
        Some(existing_cursor.last_sent),
    );
}

#[tokio::test]
async fn adapter_applies_active_task_delete_and_session_removed() {
    let (_root, state) = test_workspace_stream().await;
    let workspace_id = WorkspaceId::new();
    let task_id = TaskId::new();
    let session_id = SessionId::new();
    let mut subscription_state = WorkspaceActiveSubscriptionState::default();
    subscription_state.active_scope = true;
    subscription_state
        .active_task_sessions
        .insert(task_id, session_id);
    let mut runtime = test_runtime(
        subscription_state,
        HashMap::from([(session_id, cursor(5, 6))]),
    );

    let should_route = update_workspace_stream_subscriptions_for_event(
        &state,
        workspace_id,
        &mut runtime,
        &WorkspaceActiveSnapshotEvent::ActiveTaskDelete {
            workspace_id,
            snapshot_rev: 1,
            task_id,
        },
    )
    .await;

    assert!(should_route);
    assert!(runtime.subscription_state.active_task_sessions.is_empty());
    assert!(!runtime.subscriptions.contains_key(&session_id));

    runtime
        .subscription_state
        .explicit_sessions
        .insert(session_id);
    runtime.subscription_state.foreground_session_ids = Some(HashSet::from([session_id]));
    runtime
        .subscription_state
        .replay_sessions
        .insert(session_id);
    runtime.subscriptions.insert(session_id, cursor(7, 8));

    let should_route = update_workspace_stream_subscriptions_for_event(
        &state,
        workspace_id,
        &mut runtime,
        &WorkspaceActiveSnapshotEvent::SessionRemoved {
            workspace_id,
            snapshot_rev: 2,
            session_id,
        },
    )
    .await;

    assert!(should_route);
    assert!(runtime.subscription_state.explicit_sessions.is_empty());
    assert!(runtime.subscription_state.replay_sessions.is_empty());
    assert!(runtime.subscription_state.foreground_session_ids.is_none());
    assert!(!runtime.subscriptions.contains_key(&session_id));
}
