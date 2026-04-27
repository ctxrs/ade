use super::*;
use crate::order_seq::OrderSeqState;
use crate::scheduler::lifecycle::{fail_starting_turn, RunningTurn};
use ctx_core::models::{ExecutionEnvironment, SessionTurn, VcsKind};
use ctx_providers::adapters::{ProviderAdapter, ProviderRunHooks, TurnInput};
use ctx_providers::events::NormalizedEvent;
use ctx_providers::fake::FakeProviderAdapter;
use ctx_store::StoreManager;
use serde_json::json;
use std::collections::HashMap;
use std::path::Path;
use tempfile::tempdir;

struct LoopFixture {
    state: Arc<AppState>,
    store: ctx_store::Store,
    workspace_id: ctx_core::ids::WorkspaceId,
    worktree_id: ctx_core::ids::WorktreeId,
    task_id: ctx_core::ids::TaskId,
    session_id: ctx_core::ids::SessionId,
    turn_id: TurnId,
    run_id: RunId,
    message_id: MessageId,
    workspace_root: std::path::PathBuf,
}

async fn build_loop_fixture(data_dir: &Path, provider_id: &str, model_id: &str) -> LoopFixture {
    let stores = StoreManager::open(data_dir).await.expect("open stores");
    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));
    let state = Arc::new(AppState::new(
        data_dir.to_path_buf(),
        stores.clone(),
        providers,
        "http://localhost".to_string(),
        None,
    ));

    let workspace_root = data_dir.join("workspace");
    tokio::fs::create_dir_all(&workspace_root)
        .await
        .expect("workspace root");
    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            workspace_root.to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .expect("workspace");
    let store = state
        .store_for_workspace(workspace.id)
        .await
        .expect("workspace store");
    let worktree = store
        .create_worktree(
            workspace.id,
            workspace_root.to_string_lossy().to_string(),
            "deadbeef".to_string(),
            None,
        )
        .await
        .expect("worktree");
    let task = store
        .create_task(workspace.id, "task".to_string(), None)
        .await
        .expect("task");
    let session = store
        .create_session(
            task.id,
            workspace.id,
            worktree.id,
            ExecutionEnvironment::Host,
            provider_id.to_string(),
            model_id.to_string(),
            "implementer".to_string(),
            None,
            None,
            None,
        )
        .await
        .expect("session");
    store
        .set_task_primary_session(task.id, session.id, worktree.id)
        .await
        .expect("primary session");
    state
        .global_store()
        .upsert_workspace_session_index(session.id, workspace.id)
        .await
        .expect("workspace session index");
    state.remember_session_meta(&session).await;

    let run_id = RunId::new();
    let turn_id = TurnId::new();
    let now = chrono::Utc::now();
    store
        .insert_session_turn(SessionTurn {
            turn_id,
            session_id: session.id,
            run_id: Some(run_id),
            user_message_id: None,
            status: SessionTurnStatus::Starting,
            start_seq: Some(1),
            end_seq: None,
            started_at: now,
            updated_at: now,
            assistant_partial: None,
            thought_partial: None,
            metrics_json: None,
            tool_total: 0,
            tool_pending: 0,
            tool_running: 0,
            tool_completed: 0,
            tool_failed: 0,
        })
        .await
        .expect("insert turn");

    let seeded = store
        .get_session_head_snapshot(session.id, 60, false)
        .await
        .expect("load baseline head")
        .expect("baseline head");
    state
        .workspaces
        .workspace_active_snapshot
        .update_session_head(seeded)
        .await;
    let active_task_summary = store
        .get_workspace_active_task_summary(task.id)
        .await
        .expect("load active task summary")
        .expect("active task summary exists");
    state
        .workspaces
        .workspace_active_snapshot
        .publish_active_task_upsert(workspace.id, active_task_summary)
        .await;

    LoopFixture {
        state,
        store,
        workspace_id: workspace.id,
        worktree_id: worktree.id,
        task_id: task.id,
        session_id: session.id,
        turn_id,
        run_id,
        message_id: MessageId::new(),
        workspace_root,
    }
}

async fn run_done_event_loop(
    fixture: LoopFixture,
    provider_id: &str,
    model_id: &str,
    provider_session_ref: &str,
    codex_home: Option<&Path>,
) -> (LoopFixture, SessionTurn) {
    let (ev_tx, ev_rx) = mpsc::channel(8);
    let (events_done_tx, events_done_rx) = oneshot::channel();
    let (start_progress_tx, _start_progress_rx) =
        tokio::sync::watch::channel(TurnStartProgress::Pending);
    let loop_task = tokio::spawn(run_turn_event_loop(TurnEventLoop {
        state_weak: Arc::downgrade(&fixture.state),
        store: fixture.store.clone(),
        session_id: fixture.session_id,
        task_id: fixture.task_id,
        workspace_id: fixture.workspace_id,
        worktree_id: fixture.worktree_id,
        provider_id: provider_id.to_string(),
        model_id: model_id.to_string(),
        session_root_kind: "primary".to_string(),
        execution_environment_label: "host".to_string(),
        perf_run_id: None,
        workdir_root: fixture.workspace_root.clone(),
        workdir_canonical: Some(fixture.workspace_root.clone()),
        workdir_str: fixture.workspace_root.to_string_lossy().to_string(),
        run_started_at: Instant::now(),
        run_id: fixture.run_id,
        turn_id: fixture.turn_id,
        message_id: fixture.message_id,
        provider_session_ref: Some(provider_session_ref.to_string()),
        codex_home: codex_home.map(|path| path.to_path_buf()),
        context_window_metrics: None,
        ev_rx,
        events_done_tx,
        start_progress_tx,
        order_seq_state: Arc::new(Mutex::new(OrderSeqState::new(1))),
    }));

    ev_tx
        .send(NormalizedEvent {
            event_type: SessionEventType::Done,
            payload_json: json!({}),
        })
        .await
        .expect("send done event");
    drop(ev_tx);

    events_done_rx.await.expect("event loop completion");
    loop_task.await.expect("event loop join");
    crate::scheduler::terminal::finalize_completed_turn(
        &fixture.state,
        fixture.session_id,
        Some(fixture.run_id),
        fixture.turn_id,
        fixture.message_id,
    )
    .await
    .expect("finalize completed turn");

    let turn = fixture
        .store
        .get_session_turn(fixture.session_id, fixture.turn_id)
        .await
        .expect("load turn")
        .expect("turn exists");
    (fixture, turn)
}

#[tokio::test]
async fn turn_started_event_promotes_starting_turn_to_running() {
    let data_dir = tempdir().expect("temp dir");
    let fixture = build_loop_fixture(data_dir.path(), "fake", "model").await;

    let (ev_tx, ev_rx) = mpsc::channel(8);
    let (events_done_tx, events_done_rx) = oneshot::channel();
    let (start_progress_tx, start_progress_rx) =
        tokio::sync::watch::channel(TurnStartProgress::Pending);
    let loop_task = tokio::spawn(run_turn_event_loop(TurnEventLoop {
        state_weak: Arc::downgrade(&fixture.state),
        store: fixture.store.clone(),
        session_id: fixture.session_id,
        task_id: fixture.task_id,
        workspace_id: fixture.workspace_id,
        worktree_id: fixture.worktree_id,
        provider_id: "fake".to_string(),
        model_id: "model".to_string(),
        session_root_kind: "primary".to_string(),
        execution_environment_label: "host".to_string(),
        perf_run_id: None,
        workdir_root: fixture.workspace_root.clone(),
        workdir_canonical: Some(fixture.workspace_root.clone()),
        workdir_str: fixture.workspace_root.to_string_lossy().to_string(),
        run_started_at: Instant::now(),
        run_id: fixture.run_id,
        turn_id: fixture.turn_id,
        message_id: fixture.message_id,
        provider_session_ref: None,
        codex_home: None,
        context_window_metrics: None,
        ev_rx,
        events_done_tx,
        start_progress_tx,
        order_seq_state: Arc::new(Mutex::new(OrderSeqState::new(1))),
    }));

    ev_tx
        .send(NormalizedEvent {
            event_type: SessionEventType::TurnStarted,
            payload_json: json!({
                "session_id": fixture.session_id.0,
                "turn_id": fixture.turn_id.0,
            }),
        })
        .await
        .expect("send turn started event");
    drop(ev_tx);

    events_done_rx.await.expect("event loop completion");
    loop_task.await.expect("event loop join");

    assert_eq!(*start_progress_rx.borrow(), TurnStartProgress::Started);
    let turn = fixture
        .store
        .get_session_turn(fixture.session_id, fixture.turn_id)
        .await
        .expect("load turn")
        .expect("turn exists");
    assert_eq!(turn.status, SessionTurnStatus::Running);
}

#[tokio::test]
async fn event_loop_exits_without_persisting_when_app_state_owner_is_gone() {
    let data_dir = tempdir().expect("temp dir");
    let fixture = build_loop_fixture(data_dir.path(), "fake", "model").await;
    let LoopFixture {
        state,
        store,
        workspace_id,
        worktree_id,
        task_id,
        session_id,
        turn_id,
        run_id,
        message_id,
        workspace_root,
    } = fixture;

    let (ev_tx, ev_rx) = mpsc::channel(8);
    let (events_done_tx, events_done_rx) = oneshot::channel();
    let (start_progress_tx, _start_progress_rx) =
        tokio::sync::watch::channel(TurnStartProgress::Pending);
    let loop_task = tokio::spawn(run_turn_event_loop(TurnEventLoop {
        state_weak: Arc::downgrade(&state),
        store: store.clone(),
        session_id,
        task_id,
        workspace_id,
        worktree_id,
        provider_id: "fake".to_string(),
        model_id: "model".to_string(),
        session_root_kind: "primary".to_string(),
        execution_environment_label: "host".to_string(),
        perf_run_id: None,
        workdir_root: workspace_root.clone(),
        workdir_canonical: Some(workspace_root.clone()),
        workdir_str: workspace_root.to_string_lossy().to_string(),
        run_started_at: Instant::now(),
        run_id,
        turn_id,
        message_id,
        provider_session_ref: None,
        codex_home: None,
        context_window_metrics: None,
        ev_rx,
        events_done_tx,
        start_progress_tx,
        order_seq_state: Arc::new(Mutex::new(OrderSeqState::new(1))),
    }));

    drop(state);

    ev_tx
        .send(NormalizedEvent {
            event_type: SessionEventType::TurnStarted,
            payload_json: json!({}),
        })
        .await
        .expect("send event after dropping app state owner");
    drop(ev_tx);

    events_done_rx.await.expect("event loop completion");
    loop_task.await.expect("event loop join");

    let events = store
        .list_session_events_for_turn(session_id, turn_id, false)
        .await
        .expect("load persisted events");
    assert!(events.is_empty());
}

#[tokio::test]
async fn start_deadline_failure_finalizes_starting_turn_as_failed() {
    let data_dir = tempdir().expect("temp dir");
    let fixture = build_loop_fixture(data_dir.path(), "fake", "model").await;
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(FakeProviderAdapter::new());
    let (event_tx, _event_rx) = mpsc::channel(8);
    let handle = adapter
        .run(
            TurnInput {
                content: "slow-diff-test".to_string(),
                attachments: Vec::new(),
                context_blocks: Vec::new(),
                model_id: None,
            },
            fixture.workspace_root.clone(),
            HashMap::new(),
            event_tx.clone(),
            ProviderRunHooks::default(),
        )
        .await
        .expect("run handle");
    let (_start_progress_tx, start_progress_rx) =
        tokio::sync::watch::channel(TurnStartProgress::Pending);

    fail_starting_turn(
        &fixture.state,
        fixture.session_id,
        RunningTurn {
            adapter,
            handle,
            run_id: fixture.run_id,
            turn_id: fixture.turn_id,
            message_id: fixture.message_id,
            provider_id: "fake".to_string(),
            model_id: "model".to_string(),
            execution_environment_label: "host".to_string(),
            session_root_kind: "primary".to_string(),
            event_tx,
            events_done: None,
            start_progress: start_progress_rx,
            start_deadline: tokio::time::Instant::now(),
        },
        "provider did not report turn start before deadline",
    )
    .await;

    let turn = fixture
        .store
        .get_session_turn(fixture.session_id, fixture.turn_id)
        .await
        .expect("load turn")
        .expect("turn exists");
    assert_eq!(turn.status, SessionTurnStatus::Failed);

    let events = fixture
        .store
        .list_session_events_for_turn(fixture.session_id, fixture.turn_id, false)
        .await
        .expect("load turn events");
    assert!(events.iter().any(|event| {
        matches!(&event.event_type, SessionEventType::Error)
            && event
                .payload_json
                .get("reason")
                .and_then(|value| value.as_str())
                == Some("start_not_acknowledged")
    }));
    assert!(events.iter().any(|event| {
        matches!(&event.event_type, SessionEventType::TurnFinished)
            && event
                .payload_json
                .get("status")
                .and_then(|value| value.as_str())
                == Some("failed")
            && event.payload_json.get("kind") == Some(&json!("start_not_acknowledged"))
    }));
}

async fn write_codex_rollout_log(codex_home: &Path, session_ref: &str) {
    let sessions_dir = codex_home
        .join("sessions")
        .join("2026")
        .join("04")
        .join("05");
    tokio::fs::create_dir_all(&sessions_dir)
        .await
        .expect("create sessions dir");
    let path = sessions_dir.join(format!("rollout-2026-04-05T00-00-00-{session_ref}.jsonl"));
    let payload = json!({
        "type": "event_msg",
        "payload": {
            "type": "token_count",
            "info": {
                "model_context_window": 258400,
                "last_token_usage": {
                    "input_tokens": 12,
                    "output_tokens": 3,
                    "reasoning_output_tokens": 2,
                    "total_tokens": 17
                }
            }
        }
    });
    tokio::fs::write(&path, format!("{payload}\n"))
        .await
        .expect("write rollout log");
}

#[tokio::test]
async fn tool_events_publish_after_tool_state_persists() {
    let data_dir = tempdir().expect("temp dir");
    let stores = StoreManager::open(data_dir.path())
        .await
        .expect("open stores");
    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));
    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores.clone(),
        providers,
        "http://localhost".to_string(),
        None,
    ));

    let workspace_root = data_dir.path().join("workspace");
    tokio::fs::create_dir_all(&workspace_root)
        .await
        .expect("workspace root");
    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            workspace_root.to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .expect("workspace");
    let store = state
        .store_for_workspace(workspace.id)
        .await
        .expect("workspace store");
    let worktree = store
        .create_worktree(
            workspace.id,
            workspace_root.to_string_lossy().to_string(),
            "deadbeef".to_string(),
            None,
        )
        .await
        .expect("worktree");
    let task = store
        .create_task(workspace.id, "task".to_string(), None)
        .await
        .expect("task");
    let session = store
        .create_session(
            task.id,
            workspace.id,
            worktree.id,
            ExecutionEnvironment::Host,
            "fake".to_string(),
            "model".to_string(),
            "implementer".to_string(),
            None,
            None,
            None,
        )
        .await
        .expect("session");
    store
        .set_task_primary_session(task.id, session.id, worktree.id)
        .await
        .expect("primary session");
    state
        .global_store()
        .upsert_workspace_session_index(session.id, workspace.id)
        .await
        .expect("workspace session index");
    state.remember_session_meta(&session).await;

    let run_id = RunId::new();
    let turn_id = TurnId::new();
    let now = chrono::Utc::now();
    store
        .insert_session_turn(SessionTurn {
            turn_id,
            session_id: session.id,
            run_id: Some(run_id),
            user_message_id: None,
            status: SessionTurnStatus::Running,
            start_seq: Some(1),
            end_seq: None,
            started_at: now,
            updated_at: now,
            assistant_partial: None,
            thought_partial: None,
            metrics_json: None,
            tool_total: 0,
            tool_pending: 0,
            tool_running: 0,
            tool_completed: 0,
            tool_failed: 0,
        })
        .await
        .expect("insert turn");

    let seeded = store
        .get_session_head_snapshot(session.id, 60, false)
        .await
        .expect("load baseline head")
        .expect("baseline head");
    state
        .workspaces
        .workspace_active_snapshot
        .update_compact_session_head(seeded)
        .await;

    let (ev_tx, ev_rx) = mpsc::channel(8);
    let (events_done_tx, events_done_rx) = oneshot::channel();
    let (start_progress_tx, _start_progress_rx) =
        tokio::sync::watch::channel(TurnStartProgress::Pending);
    let loop_task = tokio::spawn(run_turn_event_loop(TurnEventLoop {
        state_weak: Arc::downgrade(&state),
        store: store.clone(),
        session_id: session.id,
        task_id: task.id,
        workspace_id: workspace.id,
        worktree_id: worktree.id,
        provider_id: "fake".to_string(),
        model_id: "model".to_string(),
        session_root_kind: "primary".to_string(),
        execution_environment_label: "host".to_string(),
        perf_run_id: None,
        workdir_root: workspace_root.clone(),
        workdir_canonical: Some(workspace_root.clone()),
        workdir_str: workspace_root.to_string_lossy().to_string(),
        run_started_at: Instant::now(),
        run_id,
        turn_id,
        message_id: MessageId::new(),
        provider_session_ref: None,
        codex_home: None,
        context_window_metrics: None,
        ev_rx,
        events_done_tx,
        start_progress_tx,
        order_seq_state: Arc::new(Mutex::new(OrderSeqState::new(1))),
    }));

    ev_tx
        .send(NormalizedEvent {
            event_type: SessionEventType::ToolCall,
            payload_json: json!({
                "tool_call_id": "call-1",
                "order_seq": 1,
                "toolCall": {
                    "name": "Bash",
                    "kind": "execute"
                },
                "status": "running",
                "rawInput": {
                    "command": "pwd"
                }
            }),
        })
        .await
        .expect("send tool call");
    drop(ev_tx);

    events_done_rx.await.expect("event loop completion");
    loop_task.await.expect("event loop join");

    let head = state
        .workspaces
        .workspace_active_snapshot
        .get_cached_session_head_for_read(session.id)
        .await
        .expect("hydrated session head should stay readable from the compact cache");
    let turn = head
        .turns
        .into_iter()
        .find(|turn| turn.turn_id == turn_id)
        .expect("turn in cached head");
    assert_eq!(turn.tool_total, 1);
    assert_eq!(turn.tool_running, 1);
    assert_eq!(turn.tool_pending, 0);
    assert_eq!(head.tool_summaries.len(), 1);
    assert_eq!(
        head.tool_summaries[0].status.as_deref(),
        Some("in_progress")
    );
}

#[tokio::test]
async fn tool_result_uses_sanitized_payload_for_persisted_summary() {
    let data_dir = tempdir().expect("temp dir");
    let stores = StoreManager::open(data_dir.path())
        .await
        .expect("open stores");
    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));
    let mut app_state = AppState::new(
        data_dir.path().to_path_buf(),
        stores.clone(),
        providers,
        "http://localhost".to_string(),
        None,
    );
    app_state.core.tool_output_spool_enabled = true;
    std::fs::create_dir_all(&app_state.core.tool_output_spool_dir).expect("tool output spool dir");
    let state = Arc::new(app_state);

    let workspace_root = data_dir.path().join("workspace");
    tokio::fs::create_dir_all(&workspace_root)
        .await
        .expect("workspace root");
    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            workspace_root.to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .expect("workspace");
    let store = state
        .store_for_workspace(workspace.id)
        .await
        .expect("workspace store");
    let worktree = store
        .create_worktree(
            workspace.id,
            workspace_root.to_string_lossy().to_string(),
            "deadbeef".to_string(),
            None,
        )
        .await
        .expect("worktree");
    let task = store
        .create_task(workspace.id, "task".to_string(), None)
        .await
        .expect("task");
    let session = store
        .create_session(
            task.id,
            workspace.id,
            worktree.id,
            ExecutionEnvironment::Host,
            "fake".to_string(),
            "model".to_string(),
            "implementer".to_string(),
            None,
            None,
            None,
        )
        .await
        .expect("session");
    store
        .set_task_primary_session(task.id, session.id, worktree.id)
        .await
        .expect("primary session");
    state
        .global_store()
        .upsert_workspace_session_index(session.id, workspace.id)
        .await
        .expect("workspace session index");
    state.remember_session_meta(&session).await;

    let run_id = RunId::new();
    let turn_id = TurnId::new();
    let now = chrono::Utc::now();
    store
        .insert_session_turn(SessionTurn {
            turn_id,
            session_id: session.id,
            run_id: Some(run_id),
            user_message_id: None,
            status: SessionTurnStatus::Running,
            start_seq: Some(1),
            end_seq: None,
            started_at: now,
            updated_at: now,
            assistant_partial: None,
            thought_partial: None,
            metrics_json: None,
            tool_total: 0,
            tool_pending: 0,
            tool_running: 0,
            tool_completed: 0,
            tool_failed: 0,
        })
        .await
        .expect("insert turn");

    let seeded = store
        .get_session_head_snapshot(session.id, 60, false)
        .await
        .expect("load baseline head")
        .expect("baseline head");
    state
        .workspaces
        .workspace_active_snapshot
        .update_session_head(seeded)
        .await;

    let (ev_tx, ev_rx) = mpsc::channel(8);
    let (events_done_tx, events_done_rx) = oneshot::channel();
    let (start_progress_tx, _start_progress_rx) =
        tokio::sync::watch::channel(TurnStartProgress::Pending);
    let loop_task = tokio::spawn(run_turn_event_loop(TurnEventLoop {
        state_weak: Arc::downgrade(&state),
        store: store.clone(),
        session_id: session.id,
        task_id: task.id,
        workspace_id: workspace.id,
        worktree_id: worktree.id,
        provider_id: "fake".to_string(),
        model_id: "model".to_string(),
        session_root_kind: "primary".to_string(),
        execution_environment_label: "host".to_string(),
        perf_run_id: None,
        workdir_root: workspace_root.clone(),
        workdir_canonical: Some(workspace_root.clone()),
        workdir_str: workspace_root.to_string_lossy().to_string(),
        run_started_at: Instant::now(),
        run_id,
        turn_id,
        message_id: MessageId::new(),
        provider_session_ref: None,
        codex_home: None,
        context_window_metrics: None,
        ev_rx,
        events_done_tx,
        start_progress_tx,
        order_seq_state: Arc::new(Mutex::new(OrderSeqState::new(1))),
    }));

    ev_tx
        .send(NormalizedEvent {
            event_type: SessionEventType::ToolResult,
            payload_json: json!({
                "tool_call_id": "toolu_exec_result_1",
                "kind": "unknown",
                "tool_name": "unknown",
                "title": "unknown",
                "status": "completed",
                "output_preview": "1"
            }),
        })
        .await
        .expect("send tool result");
    drop(ev_tx);

    events_done_rx.await.expect("event loop completion");
    loop_task.await.expect("event loop join");

    let tool = store
        .get_session_turn_tool(session.id, "toolu_exec_result_1")
        .await
        .expect("load tool")
        .expect("persisted tool summary");
    assert_eq!(tool.order_seq, 1);
    assert_eq!(tool.tool_kind.as_deref(), Some("execute"));
    assert_eq!(tool.provider_tool_name.as_deref(), Some("Bash"));
    assert_eq!(tool.title.as_deref(), Some("Bash"));
    assert_eq!(tool.output_text.as_deref(), Some("1"));

    let events = store
        .list_session_events_for_turn(session.id, turn_id, false)
        .await
        .expect("load persisted events");
    let result_event = events
        .into_iter()
        .find(|event| matches!(event.event_type, SessionEventType::ToolResult))
        .expect("tool result event");
    assert!(result_event.payload_json.get("output_artifact").is_none());
}

#[tokio::test]
async fn large_tool_result_spills_to_artifact_and_keeps_preview_bounded() {
    let data_dir = tempdir().expect("temp dir");
    let stores = StoreManager::open(data_dir.path())
        .await
        .expect("open stores");
    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));
    let mut app_state = AppState::new(
        data_dir.path().to_path_buf(),
        stores.clone(),
        providers,
        "http://localhost".to_string(),
        None,
    );
    app_state.core.tool_output_spool_enabled = true;
    std::fs::create_dir_all(&app_state.core.tool_output_spool_dir).expect("tool output spool dir");
    let state = Arc::new(app_state);

    let workspace_root = data_dir.path().join("workspace");
    tokio::fs::create_dir_all(&workspace_root)
        .await
        .expect("workspace root");
    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            workspace_root.to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .expect("workspace");
    let store = state
        .store_for_workspace(workspace.id)
        .await
        .expect("workspace store");
    let worktree = store
        .create_worktree(
            workspace.id,
            workspace_root.to_string_lossy().to_string(),
            "deadbeef".to_string(),
            None,
        )
        .await
        .expect("worktree");
    let task = store
        .create_task(workspace.id, "task".to_string(), None)
        .await
        .expect("task");
    let session = store
        .create_session(
            task.id,
            workspace.id,
            worktree.id,
            ExecutionEnvironment::Host,
            "fake".to_string(),
            "model".to_string(),
            "implementer".to_string(),
            None,
            None,
            None,
        )
        .await
        .expect("session");
    store
        .set_task_primary_session(task.id, session.id, worktree.id)
        .await
        .expect("primary session");
    state
        .global_store()
        .upsert_workspace_session_index(session.id, workspace.id)
        .await
        .expect("workspace session index");
    state.remember_session_meta(&session).await;

    let run_id = RunId::new();
    let turn_id = TurnId::new();
    let now = chrono::Utc::now();
    store
        .insert_session_turn(SessionTurn {
            turn_id,
            session_id: session.id,
            run_id: Some(run_id),
            user_message_id: None,
            status: SessionTurnStatus::Running,
            start_seq: Some(1),
            end_seq: None,
            started_at: now,
            updated_at: now,
            assistant_partial: None,
            thought_partial: None,
            metrics_json: None,
            tool_total: 0,
            tool_pending: 0,
            tool_running: 0,
            tool_completed: 0,
            tool_failed: 0,
        })
        .await
        .expect("insert turn");

    let seeded = store
        .get_session_head_snapshot(session.id, 60, false)
        .await
        .expect("load baseline head")
        .expect("baseline head");
    state
        .workspaces
        .workspace_active_snapshot
        .update_session_head(seeded)
        .await;

    let (ev_tx, ev_rx) = mpsc::channel(8);
    let (events_done_tx, events_done_rx) = oneshot::channel();
    let (start_progress_tx, _start_progress_rx) =
        tokio::sync::watch::channel(TurnStartProgress::Pending);
    let loop_task = tokio::spawn(run_turn_event_loop(TurnEventLoop {
        state_weak: Arc::downgrade(&state),
        store: store.clone(),
        session_id: session.id,
        task_id: task.id,
        workspace_id: workspace.id,
        worktree_id: worktree.id,
        provider_id: "fake".to_string(),
        model_id: "model".to_string(),
        session_root_kind: "primary".to_string(),
        execution_environment_label: "host".to_string(),
        perf_run_id: None,
        workdir_root: workspace_root.clone(),
        workdir_canonical: Some(workspace_root.clone()),
        workdir_str: workspace_root.to_string_lossy().to_string(),
        run_started_at: Instant::now(),
        run_id,
        turn_id,
        message_id: MessageId::new(),
        provider_session_ref: None,
        codex_home: None,
        context_window_metrics: None,
        ev_rx,
        events_done_tx,
        start_progress_tx,
        order_seq_state: Arc::new(Mutex::new(OrderSeqState::new(1))),
    }));

    let large_output = (1..=10)
        .map(|index| format!("line-{index}"))
        .collect::<Vec<_>>()
        .join("\n");
    ev_tx
        .send(NormalizedEvent {
            event_type: SessionEventType::ToolResult,
            payload_json: json!({
                "tool_call_id": "tool-large-1",
                "kind": "execute",
                "tool_name": "Bash",
                "title": "Bash",
                "status": "completed",
                "output_text": large_output,
                "order_seq": 1
            }),
        })
        .await
        .expect("send tool result");
    drop(ev_tx);

    events_done_rx.await.expect("event loop completion");
    loop_task.await.expect("event loop join");

    let events = store
        .list_session_events_for_turn(session.id, turn_id, false)
        .await
        .expect("load persisted events");
    let result_event = events
        .into_iter()
        .find(|event| matches!(event.event_type, SessionEventType::ToolResult))
        .expect("tool result event");
    let output_preview = result_event
        .payload_json
        .get("output_preview")
        .and_then(serde_json::Value::as_str)
        .expect("output preview");
    assert!(output_preview.contains("... +6 lines"));
    assert!(result_event.payload_json.get("output_text").is_none());
    let artifact_id = result_event
        .payload_json
        .get("output_artifact")
        .and_then(|value| value.get("artifact_id"))
        .and_then(serde_json::Value::as_str)
        .expect("artifact id");
    let artifact = store
        .get_artifact(ctx_core::ids::ArtifactId(
            uuid::Uuid::parse_str(artifact_id).expect("uuid"),
        ))
        .await
        .expect("artifact lookup")
        .expect("artifact row");
    assert_eq!(artifact.mime_type, "text/plain");
    assert_eq!(
        tokio::fs::read_to_string(&artifact.absolute_path)
            .await
            .expect("artifact body")
            .lines()
            .count(),
        10
    );

    let session_state = store
        .get_session_state(session.id)
        .await
        .expect("load session state");
    assert_eq!(session_state.artifacts.len(), 1);
    assert_eq!(session_state.artifacts[0].id, artifact.id);
}

#[tokio::test]
async fn codex_done_metrics_use_runtime_codex_home_instead_of_home_dir_guess() {
    let session_ref = "019d5ac4-e8b0-7c93-9b0f-e4b22203d391";
    let codex_home = tempdir().expect("temp codex home");
    let data_dir = tempdir().expect("temp data dir");

    write_codex_rollout_log(codex_home.path(), session_ref).await;
    let fixture = build_loop_fixture(data_dir.path(), "codex", "gpt-5.4/medium").await;
    let (_fixture, turn) = run_done_event_loop(
        fixture,
        "codex",
        "gpt-5.4/medium",
        session_ref,
        Some(codex_home.path()),
    )
    .await;
    let metrics = turn.metrics_json.expect("expected codex rollout metrics");

    assert_eq!(turn.status, SessionTurnStatus::Completed);
    assert_eq!(
        metrics
            .get("context_window_tokens")
            .and_then(serde_json::Value::as_u64),
        Some(258400)
    );
    assert_eq!(
        metrics
            .get("context_tokens_estimate")
            .and_then(serde_json::Value::as_u64),
        Some(17)
    );
    assert_eq!(
        metrics
            .get("remaining_tokens_estimate")
            .and_then(serde_json::Value::as_u64),
        Some(258383)
    );
    assert_eq!(
        metrics
            .get("total_input_tokens")
            .and_then(serde_json::Value::as_u64),
        Some(12)
    );
    assert_eq!(
        metrics
            .get("total_output_tokens")
            .and_then(serde_json::Value::as_u64),
        Some(5)
    );
}

#[tokio::test]
async fn done_events_update_active_task_summary_activity_to_match_head() {
    let data_dir = tempdir().expect("temp data dir");
    let fixture = build_loop_fixture(data_dir.path(), "fake", "fake-model").await;
    let (fixture, turn) =
        run_done_event_loop(fixture, "fake", "fake-model", "session-ref", None).await;

    assert_eq!(turn.status, SessionTurnStatus::Completed);

    let active_task = fixture
        .state
        .workspaces
        .workspace_active_snapshot
        .active_task_summary(fixture.workspace_id, fixture.task_id)
        .await
        .expect("active task summary");
    assert_eq!(
        active_task.primary_session.activity.last_turn_status,
        Some(SessionTurnStatus::Completed)
    );
    assert!(!active_task.primary_session.activity.is_working);

    let active_heads = fixture
        .state
        .workspaces
        .workspace_active_snapshot
        .active_heads(fixture.workspace_id)
        .await;
    let head = active_heads
        .heads
        .into_iter()
        .find(|head| head.session.id == fixture.session_id)
        .expect("active head");
    assert_eq!(head.activity, active_task.primary_session.activity);
}
