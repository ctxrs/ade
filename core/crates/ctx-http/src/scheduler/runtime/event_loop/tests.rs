use super::*;
use crate::order_seq::OrderSeqState;
use ctx_core::models::{ExecutionEnvironment, SessionTurn, VcsKind};
use ctx_providers::adapters::ProviderAdapter;
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
) -> SessionTurn {
    let (ev_tx, ev_rx) = mpsc::channel(8);
    let (events_done_tx, events_done_rx) = oneshot::channel();
    let loop_task = tokio::spawn(run_turn_event_loop(TurnEventLoop {
        state: fixture.state.clone(),
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

    fixture
        .store
        .get_session_turn(fixture.session_id, fixture.turn_id)
        .await
        .expect("load turn")
        .expect("turn exists")
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
    let loop_task = tokio::spawn(run_turn_event_loop(TurnEventLoop {
        state: state.clone(),
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
        .update_session_head(seeded)
        .await;

    let (ev_tx, ev_rx) = mpsc::channel(8);
    let (events_done_tx, events_done_rx) = oneshot::channel();
    let loop_task = tokio::spawn(run_turn_event_loop(TurnEventLoop {
        state: state.clone(),
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
}

#[tokio::test]
async fn codex_done_metrics_use_runtime_codex_home_instead_of_home_dir_guess() {
    let session_ref = "019d5ac4-e8b0-7c93-9b0f-e4b22203d391";
    let codex_home = tempdir().expect("temp codex home");
    let data_dir = tempdir().expect("temp data dir");

    write_codex_rollout_log(codex_home.path(), session_ref).await;
    let fixture = build_loop_fixture(data_dir.path(), "codex", "gpt-5.4/medium").await;
    let turn = run_done_event_loop(
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
