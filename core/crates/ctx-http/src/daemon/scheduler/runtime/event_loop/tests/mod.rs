use super::*;
use crate::daemon::scheduler::lifecycle::{fail_starting_turn, RunningTurn};
use ctx_core::models::{
    ExecutionEnvironment, SessionEventType, SessionTurn, SessionTurnStatus, VcsKind,
};
use ctx_providers::adapters::{ProviderAdapter, ProviderRunHooks, TurnInput};
use ctx_providers::events::NormalizedEvent;
use ctx_providers::fake::FakeProviderAdapter;
use ctx_session_tools::order_seq::OrderSeqState;
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
    state.sessions.remember_session_meta(&session).await;

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
            failure: None,
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
    crate::daemon::scheduler::terminal::finalize_completed_turn(
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

mod done_metrics;
mod lifecycle;
mod tools;
