use super::head_projection::{
    build_session_summary_delta, derive_summary_activity, resolve_projection_rev_for_stream_delta,
};
use chrono::Utc;
use ctx_core::ids::{SessionId, TaskId, WorkspaceId, WorktreeId};
use ctx_core::models::{
    ExecutionEnvironment, Session, SessionActivityState, SessionEventType, SessionStatus,
    SessionTurnStatus,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

fn test_session() -> Session {
    Session {
        id: SessionId::new(),
        task_id: TaskId::new(),
        workspace_id: WorkspaceId::new(),
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
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

#[test]
fn queued_turns_do_not_publish_working_activity() {
    let queued = derive_summary_activity(&SessionEventType::TurnQueued)
        .expect("queued turns should publish summary activity");
    assert!(!queued.is_working);
    assert_eq!(queued.last_turn_status, Some(SessionTurnStatus::Queued));

    let running = derive_summary_activity(&SessionEventType::TurnStarted)
        .expect("running turns should publish summary activity");
    assert!(running.is_working);
    assert_eq!(running.last_turn_status, Some(SessionTurnStatus::Running));
}

#[test]
fn emitted_session_summary_deltas_always_include_monotonic_versions() {
    let session = test_session();
    let now = Utc::now();

    let message_delta = build_session_summary_delta(
        &session,
        None,
        Some(now),
        Some("preview".to_string()),
        22,
        22,
        22,
    )
    .expect("message preview should emit a summary delta");
    assert_eq!(message_delta.last_event_seq, Some(22));
    assert_eq!(message_delta.projection_rev, Some(22));
    assert_eq!(message_delta.state_rev, Some(22));
}

#[test]
fn empty_session_summary_delta_is_not_emitted() {
    let session = test_session();
    assert!(
        build_session_summary_delta(&session, None, None, None, 5, 5, 5).is_none(),
        "empty updates should not publish summary deltas"
    );
}

#[test]
fn activity_only_session_summary_delta_is_emitted() {
    let session = test_session();
    let delta = build_session_summary_delta(
        &session,
        Some(SessionActivityState {
            is_working: true,
            last_turn_status: Some(SessionTurnStatus::Running),
        }),
        None,
        None,
        5,
        5,
        5,
    )
    .expect("activity updates should emit a summary delta");
    assert!(delta.activity.expect("activity delta").is_working);
    assert_eq!(delta.last_event_seq, Some(5));
}

#[tokio::test]
async fn stream_only_projection_rev_skips_lookup() {
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_for_lookup = Arc::clone(&calls);
    let projection_rev =
        resolve_projection_rev_for_stream_delta(true, 41, 23, move || async move {
            calls_for_lookup.fetch_add(1, Ordering::SeqCst);
            Some(99)
        })
        .await;

    assert_eq!(projection_rev, 23);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn non_stream_only_projection_rev_uses_lookup_when_available() {
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_for_lookup = Arc::clone(&calls);
    let projection_rev =
        resolve_projection_rev_for_stream_delta(false, 17, 7, move || async move {
            calls_for_lookup.fetch_add(1, Ordering::SeqCst);
            Some(23)
        })
        .await;

    assert_eq!(projection_rev, 23);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
