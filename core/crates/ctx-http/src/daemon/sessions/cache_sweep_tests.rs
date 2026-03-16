use super::{
    active_head_projection_should_flush, active_head_projection_wait_duration,
    build_session_summary_delta, derive_summary_activity, resolve_projection_rev_for_stream_delta,
};
use chrono::Utc;
use ctx_core::ids::{SessionId, TaskId, WorkspaceId, WorktreeId};
use ctx_core::models::{
    ExecutionEnvironment, Session, SessionEventType, SessionStatus, SessionTurnStatus,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

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
fn active_head_projection_waits_for_debounce_or_max_flush() {
    let debounce = Duration::from_millis(200);
    let max_flush = Duration::from_millis(1500);
    let now = Instant::now();

    let wait = active_head_projection_wait_duration(
        now,
        now - Duration::from_millis(100),
        now - Duration::from_millis(100),
        debounce,
        max_flush,
    );
    assert_eq!(wait, Duration::from_millis(100));

    let wait = active_head_projection_wait_duration(
        now,
        now - Duration::from_millis(50),
        now - Duration::from_millis(1490),
        debounce,
        max_flush,
    );
    assert_eq!(wait, Duration::from_millis(10));
}

#[test]
fn active_head_projection_flushes_on_idle_or_max() {
    let debounce = Duration::from_millis(200);
    let max_flush = Duration::from_millis(1500);
    let now = Instant::now();

    assert!(active_head_projection_should_flush(
        now,
        now - Duration::from_millis(250),
        now - Duration::from_millis(500),
        debounce,
        max_flush
    ));
    assert!(active_head_projection_should_flush(
        now,
        now - Duration::from_millis(50),
        now - Duration::from_millis(1600),
        debounce,
        max_flush
    ));
    assert!(!active_head_projection_should_flush(
        now,
        now - Duration::from_millis(50),
        now - Duration::from_millis(500),
        debounce,
        max_flush
    ));
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

    let activity_delta = build_session_summary_delta(
        &session,
        derive_summary_activity(&SessionEventType::TurnStarted),
        None,
        None,
        17,
        21,
        21,
    )
    .expect("activity change should emit a summary delta");
    assert_eq!(activity_delta.last_event_seq, Some(17));
    assert_eq!(activity_delta.projection_rev, Some(21));
    assert_eq!(activity_delta.state_rev, Some(21));

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
