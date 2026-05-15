use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::json;

use ctx_core::ids::{MessageId, RunId, TurnId};
use ctx_core::models::{SessionEventType, SessionTurn, SessionTurnStatus};
use ctx_providers::adapters::{
    ProviderAdapter, ProviderHealth, ProviderStatus, RunHandle, TurnInput,
};
use ctx_providers::events::NormalizedEvent;
use tokio::sync::mpsc;

mod common;

struct StartFailProvider;

#[async_trait]
impl ProviderAdapter for StartFailProvider {
    async fn inspect(&self) -> Result<ProviderStatus> {
        Ok(ProviderStatus {
            provider_id: "fake".into(),
            installed: true,
            detected_path: None,
            version: Some("0.1.0".into()),
            capabilities: None,
            health: ProviderHealth::Ok,
            diagnostics: Vec::new(),
            details: HashMap::new(),
            usability: ctx_providers::adapters::ProviderUsability::default(),
        })
    }

    async fn run(
        &self,
        _input: TurnInput,
        _workdir: PathBuf,
        _env: HashMap<String, String>,
        _event_sink: mpsc::Sender<NormalizedEvent>,
        _hooks: ctx_providers::adapters::ProviderRunHooks,
    ) -> Result<RunHandle> {
        anyhow::bail!("synthetic start failure");
    }

    async fn cancel(&self, _handle: &mut RunHandle) -> Result<()> {
        Ok(())
    }
}

struct TestHarness {
    _repo: tempfile::TempDir,
    _data_dir: tempfile::TempDir,
    daemon: ctx_daemon::test_support::TestDaemon,
    session: ctx_core::models::Session,
    store: ctx_store::Store,
}

async fn insert_running_turn(
    store: &ctx_store::Store,
    session_id: ctx_core::ids::SessionId,
    run_id: RunId,
    turn_id: TurnId,
) {
    store
        .insert_session_turn(SessionTurn {
            turn_id,
            session_id,
            run_id: Some(run_id),
            user_message_id: Some(MessageId::new()),
            status: SessionTurnStatus::Running,
            start_seq: Some(1),
            end_seq: None,
            started_at: common::fixed_utc(0),
            updated_at: common::fixed_utc(0),
            assistant_partial: None,
            thought_partial: None,
            metrics_json: None,
            failure: None,
            tool_total: 0,
            tool_pending: 0,
            tool_running: 0,
            tool_completed: 0,
            tool_failed: 0,
        })
        .await
        .unwrap();
}

async fn setup_state() -> TestHarness {
    setup_state_with_providers(common::fake_providers()).await
}

async fn setup_state_with_providers(
    providers: HashMap<String, Arc<dyn ProviderAdapter>>,
) -> TestHarness {
    let repo = common::init_git_repo(&[("file.txt", "hello\n")]).await;
    let data_dir = tempfile::tempdir().unwrap();
    let stores = common::setup_store(data_dir.path()).await;
    let daemon = common::build_daemon(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:0",
    );
    let app = common::router_for_daemon(&daemon);
    let ws = common::create_workspace(&app, repo.path(), "ws").await;
    let (_task, session) =
        common::create_task_with_session(&app, ws.id.0, "t1", "fake", "fake-model").await;
    let store = daemon.store_for_session(session.id).await.unwrap();
    TestHarness {
        _repo: repo,
        _data_dir: data_dir,
        daemon,
        session,
        store,
    }
}

#[tokio::test]
async fn reconcile_terminal_state_respects_turn_finished_status() {
    let harness = setup_state().await;
    let run_id = RunId::new();
    let turn_id = TurnId::new();
    insert_running_turn(&harness.store, harness.session.id, run_id, turn_id).await;

    let finished = harness
        .store
        .append_session_event(
            harness.session.id,
            Some(run_id),
            Some(turn_id),
            SessionEventType::TurnFinished,
            json!({"status": "interrupted"}),
        )
        .await
        .unwrap();

    harness
        .daemon
        .reconcile_turn_terminal_state_for_test(
            harness.session.id,
            Some(run_id),
            turn_id,
            "daemon_restart",
        )
        .await
        .unwrap();

    let turn = harness
        .store
        .get_session_turn(harness.session.id, turn_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(turn.status, SessionTurnStatus::Interrupted);
    assert_eq!(turn.end_seq, Some(finished.seq));
    let summary = harness
        .store
        .get_session_snapshot(harness.session.id, 50, false)
        .await
        .unwrap()
        .expect("session snapshot")
        .summary;
    assert_eq!(
        summary.activity.last_turn_status,
        Some(SessionTurnStatus::Interrupted)
    );
    assert!(!summary.activity.is_working);
}

#[tokio::test]
async fn reconcile_terminal_state_emits_interrupt_when_terminal_event_missing() {
    let harness = setup_state().await;
    let run_id = RunId::new();
    let turn_id = TurnId::new();
    insert_running_turn(&harness.store, harness.session.id, run_id, turn_id).await;

    harness
        .daemon
        .reconcile_turn_terminal_state_for_test(
            harness.session.id,
            Some(run_id),
            turn_id,
            "daemon_restart",
        )
        .await
        .unwrap();

    let turn = harness
        .store
        .get_session_turn(harness.session.id, turn_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(turn.status, SessionTurnStatus::Interrupted);

    let events = harness
        .store
        .list_session_events_for_turn(harness.session.id, turn_id, false)
        .await
        .unwrap();
    assert!(events
        .iter()
        .any(|event| matches!(event.event_type, SessionEventType::TurnInterrupted)));
    let finished = events
        .iter()
        .find(|event| matches!(event.event_type, SessionEventType::TurnFinished))
        .expect("turn finished event");
    assert_eq!(
        finished
            .payload_json
            .get("status")
            .and_then(|value| value.as_str()),
        Some("interrupted")
    );
}

#[tokio::test]
async fn reconcile_provider_exit_emits_failed_terminal_events_when_missing() {
    let harness = setup_state().await;
    let run_id = RunId::new();
    let turn_id = TurnId::new();
    insert_running_turn(&harness.store, harness.session.id, run_id, turn_id).await;

    harness
        .daemon
        .reconcile_turn_failed_on_provider_exit_for_test(
            harness.session.id,
            Some(run_id),
            turn_id,
            "provider_exit",
        )
        .await
        .unwrap();

    let turn = harness
        .store
        .get_session_turn(harness.session.id, turn_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(turn.status, SessionTurnStatus::Failed);
    let summary = harness
        .store
        .get_session_snapshot(harness.session.id, 50, false)
        .await
        .unwrap()
        .expect("session snapshot")
        .summary;
    assert_eq!(
        summary.activity.last_turn_status,
        Some(SessionTurnStatus::Failed)
    );
    assert!(!summary.activity.is_working);

    let events = harness
        .store
        .list_session_events_for_turn(harness.session.id, turn_id, false)
        .await
        .unwrap();
    let failed_finished = events
        .iter()
        .find(|event| {
            matches!(event.event_type, SessionEventType::TurnFinished)
                && event
                    .payload_json
                    .get("status")
                    .and_then(|value| value.as_str())
                    == Some("failed")
        })
        .expect("failed turn_finished event");
    assert_eq!(
        failed_finished
            .payload_json
            .get("reason")
            .and_then(|value| value.as_str()),
        Some("provider_exit")
    );
    let finished = events
        .iter()
        .find(|event| matches!(event.event_type, SessionEventType::TurnFinished))
        .expect("turn finished event");
    assert_eq!(
        finished
            .payload_json
            .get("status")
            .and_then(|value| value.as_str()),
        Some("failed")
    );
}

#[tokio::test]
async fn start_failure_marks_turn_failed_and_finishes() {
    let mut providers = common::fake_providers();
    providers.insert("fake".into(), Arc::new(StartFailProvider));
    let harness = setup_state_with_providers(providers).await;
    let app = common::router_for_daemon(&harness.daemon);

    let (status, message): (axum::http::StatusCode, ctx_core::models::Message) =
        common::json_request(
            &app,
            axum::http::Method::POST,
            format!("/api/sessions/{}/messages", harness.session.id.0),
            Some(json!({"content":"start failure"})),
        )
        .await;
    assert_eq!(status, axum::http::StatusCode::OK);

    let turn_id = message.turn_id.expect("turn id");
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let events = harness
            .store
            .list_session_events(harness.session.id)
            .await
            .unwrap();
        if events.iter().any(|event| {
            event.turn_id == Some(turn_id)
                && matches!(event.event_type, SessionEventType::TurnFinished)
        }) {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("timed out waiting for start failure turn finish");
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }

    let turn = harness
        .store
        .get_session_turn(harness.session.id, turn_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(turn.status, SessionTurnStatus::Failed);

    let events = harness
        .store
        .list_session_events_for_turn(harness.session.id, turn_id, false)
        .await
        .unwrap();
    assert!(events.iter().any(|event| {
        matches!(event.event_type, SessionEventType::TurnFinished)
            && event
                .payload_json
                .get("status")
                .and_then(|value| value.as_str())
                == Some("failed")
    }));
}
