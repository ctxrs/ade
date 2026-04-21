use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use axum::http::StatusCode;
use serde_json::json;
use tokio::sync::oneshot;
use tokio::time::sleep;

use ctx_core::ids::{RunId, SessionId, TurnId};
use ctx_core::models::{SessionEventType, SessionTurn, SessionTurnStatus, VcsKind};
use ctx_http::daemon::AppState;
use ctx_providers::adapters::{
    ProviderAdapter, ProviderHealth, ProviderStatus, ProviderUsability, RunHandle, TurnInput,
};
use ctx_providers::events::NormalizedEvent;
use ctx_store::Store;
use uuid::Uuid;

mod common;

#[derive(Default)]
struct BrokenOutcomeProviderAdapter;

#[async_trait]
impl ProviderAdapter for BrokenOutcomeProviderAdapter {
    async fn inspect(&self) -> Result<ProviderStatus> {
        Ok(ProviderStatus {
            // The adapter is registered under "broken"; fake is test-special-cased
            // as ready without a provider-matrix install contract.
            provider_id: "fake".into(),
            installed: true,
            detected_path: None,
            version: Some("test".into()),
            capabilities: None,
            health: ProviderHealth::Ok,
            diagnostics: Vec::new(),
            details: HashMap::new(),
            usability: ProviderUsability::default(),
        })
    }

    async fn run(
        &self,
        input: TurnInput,
        _workdir: PathBuf,
        _env: HashMap<String, String>,
        event_sink: tokio::sync::mpsc::Sender<NormalizedEvent>,
    ) -> Result<RunHandle> {
        let (cancel_tx, cancel_rx) = oneshot::channel();
        let (done_tx, done_rx) = oneshot::channel();
        let (outcome_tx, outcome_rx) = oneshot::channel();
        let omit_abort_handle = input.content.contains("no-abort");
        let join = tokio::spawn(async move {
            if input.content.contains("done-without-outcome") {
                let _ = done_tx.send(());
                let _keep_event_sink = event_sink;
                let _keep_outcome_open = outcome_tx;
                std::future::pending::<()>().await;
            } else if input.content.contains("done-close-outcome") {
                let _ = done_tx.send(());
                let _keep_event_sink = event_sink;
                drop(outcome_tx);
                std::future::pending::<()>().await;
            } else if input.content.contains("cancel-without-outcome") {
                let _ = cancel_rx.await;
                let _keep_event_sink = event_sink;
                let _keep_done_open = done_tx;
                let _keep_outcome_open = outcome_tx;
                std::future::pending::<()>().await;
            } else if input.content.contains("cancel-close-outcome") {
                let _ = cancel_rx.await;
                let _keep_event_sink = event_sink;
                let _keep_done_open = done_tx;
                drop(outcome_tx);
                std::future::pending::<()>().await;
            } else {
                let _keep_event_sink = event_sink;
                let _keep_done_open = done_tx;
                let _keep_outcome_open = outcome_tx;
                std::future::pending::<()>().await;
            }
        });

        Ok(RunHandle {
            done: done_rx,
            outcome: outcome_rx,
            cancel: Some(cancel_tx),
            abort: if omit_abort_handle {
                None
            } else {
                Some(join.abort_handle())
            },
        })
    }

    async fn cancel(&self, handle: &mut RunHandle) -> Result<()> {
        if let Some(cancel) = handle.cancel.take() {
            let _ = cancel.send(());
        }
        Ok(())
    }
}

async fn setup_state_with_providers(
    repo_root: &Path,
    providers: HashMap<String, Arc<dyn ProviderAdapter>>,
) -> (
    tempfile::TempDir,
    Arc<AppState>,
    common::TestServer,
    Store,
    String,
) {
    let data_dir = tempfile::tempdir().unwrap();
    let stores = common::setup_store(data_dir.path()).await;
    let statuses = providers.clone();
    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores.clone(),
        providers,
        "http://127.0.0.1:0",
    );
    for (provider_id, provider) in statuses {
        let status = provider.inspect().await.unwrap();
        state
            .providers
            .statuses
            .lock()
            .await
            .insert(provider_id, status);
    }
    let app = common::router(state.clone());
    let server = common::spawn_http_server(app).await;

    let ws = stores
        .global()
        .create_workspace(
            "test".into(),
            repo_root.to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();
    let store = stores.workspace(ws.id).await.unwrap();
    let vcs = ctx_fs::vcs::driver_for_path(repo_root).await.unwrap();
    let base_commit = vcs.rev_parse_head(repo_root).await.unwrap();
    let worktree = store
        .create_worktree(
            ws.id,
            repo_root.to_string_lossy().to_string(),
            base_commit,
            None,
        )
        .await
        .unwrap();
    let task = store.create_task(ws.id, "task".into(), None).await.unwrap();
    let session = store
        .create_session(
            task.id,
            ws.id,
            worktree.id,
            ctx_core::models::ExecutionEnvironment::Host,
            "fake".into(),
            "fake-model".into(),
            "assistant".into(),
            None,
            None,
            None,
        )
        .await
        .unwrap();

    state
        .global_store()
        .upsert_workspace_session_index(session.id, ws.id)
        .await
        .unwrap();
    state
        .global_store()
        .upsert_workspace_worktree_index(worktree.id, ws.id)
        .await
        .unwrap();
    state
        .global_store()
        .upsert_workspace_task_index(task.id, ws.id)
        .await
        .unwrap();

    (data_dir, state, server, store, session.id.0.to_string())
}

async fn setup_state(
    repo_root: &Path,
) -> (
    tempfile::TempDir,
    Arc<AppState>,
    common::TestServer,
    Store,
    String,
) {
    setup_state_with_providers(repo_root, common::fake_providers()).await
}

#[tokio::test]
async fn subagent_init_accepts_raw_provider_status_when_derived_status_is_ready() {
    let repo = common::init_git_repo(&[("README.md", "ok")]).await;
    let (_data_dir, _state, server, _store, parent_id) = setup_state(repo.path()).await;
    let client = &server.client;
    let base = &server.base_url;

    let resp = client
        .post(format!("{base}/api/mcp/sessions/{parent_id}/subagent_init"))
        .json(&json!({
            "worktree": "inherit",
            "agents": [
                { "prompt": "ready", "label": "Ready", "harness": "fake", "model": "fake-model" }
            ]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "running");
    assert_eq!(body["results"][0]["label"], "Ready");
    assert_eq!(body["results"][0]["status"], "running");
}

#[tokio::test]
async fn subagent_init_rejects_duplicate_labels() {
    let repo = common::init_git_repo(&[("README.md", "ok")]).await;
    let (_data_dir, _state, server, _store, parent_id) = setup_state(repo.path()).await;
    let client = &server.client;
    let base = &server.base_url;

    let resp = client
        .post(format!("{base}/api/mcp/sessions/{parent_id}/subagent_init"))
        .json(&json!({
            "worktree": "inherit",
            "agents": [
                { "prompt": "a", "label": "Dup", "harness": "fake", "model": "fake-model" },
                { "prompt": "b", "label": "Dup", "harness": "fake", "model": "fake-model" }
            ]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["error"]
        .as_str()
        .unwrap_or("")
        .contains("duplicate subagent label"));
}

#[tokio::test]
async fn subagent_init_rejects_existing_label() {
    let repo = common::init_git_repo(&[("README.md", "ok")]).await;
    let (_data_dir, state, server, store, parent_id) = setup_state(repo.path()).await;
    let parent = store
        .get_session(SessionId(Uuid::parse_str(&parent_id).unwrap()))
        .await
        .unwrap()
        .unwrap();
    let child = store
        .create_session(
            parent.task_id,
            parent.workspace_id,
            parent.worktree_id,
            parent.execution_environment,
            "fake".into(),
            "fake-model".into(),
            "subagent".into(),
            Some(parent.id),
            Some("sub_agent".into()),
            None,
        )
        .await
        .unwrap();
    store
        .update_session_title(child.id, "Existing".into())
        .await
        .unwrap();
    state
        .global_store()
        .upsert_workspace_session_index(child.id, parent.workspace_id)
        .await
        .unwrap();

    let client = &server.client;
    let base = &server.base_url;
    let resp = client
        .post(format!("{base}/api/mcp/sessions/{parent_id}/subagent_init"))
        .json(&json!({
            "worktree": "inherit",
            "agents": [
                { "prompt": "a", "label": "Existing", "harness": "fake", "model": "fake-model" }
            ]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["error"]
        .as_str()
        .unwrap_or("")
        .contains("already exists"));
}

#[tokio::test]
async fn subagent_init_rejects_response_mode() {
    let repo = common::init_git_repo(&[("README.md", "ok")]).await;
    let (_data_dir, _state, server, _store, parent_id) = setup_state(repo.path()).await;
    let client = &server.client;
    let base = &server.base_url;

    let resp = client
        .post(format!("{base}/api/mcp/sessions/{parent_id}/subagent_init"))
        .json(&json!({
            "worktree": "inherit",
            "response_mode": "await",
            "agents": [
                { "prompt": "a", "label": "Mode", "harness": "fake", "model": "fake-model" }
            ]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["error"]
        .as_str()
        .unwrap_or("")
        .contains("response_mode"));
}

#[tokio::test]
async fn subagent_init_rejects_worktree_new_when_dirty() {
    let repo = common::init_git_repo(&[("README.md", "ok")]).await;
    fs::write(repo.path().join("dirty.txt"), "dirty").unwrap();

    let (_data_dir, _state, server, _store, parent_id) = setup_state(repo.path()).await;
    let client = &server.client;
    let base = &server.base_url;

    let resp = client
        .post(format!("{base}/api/mcp/sessions/{parent_id}/subagent_init"))
        .json(&json!({
            "worktree": "new",
            "agents": [
                { "prompt": "a", "label": "Dirty", "harness": "fake", "model": "fake-model" }
            ]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        body["error"].as_str().unwrap_or(""),
        "Your worktree has uncommitted changes. Before starting new subagents in new worktree mode, you must commit or stash your changes to be explicit about whether subagents should inherit these diffs."
    );
}

#[tokio::test]
async fn subagent_reply_rejects_when_busy() {
    let repo = common::init_git_repo(&[("README.md", "ok")]).await;
    let (_data_dir, state, server, store, parent_id) = setup_state(repo.path()).await;
    let parent = store
        .get_session(SessionId(Uuid::parse_str(&parent_id).unwrap()))
        .await
        .unwrap()
        .unwrap();
    let child = store
        .create_session(
            parent.task_id,
            parent.workspace_id,
            parent.worktree_id,
            parent.execution_environment,
            "fake".into(),
            "fake-model".into(),
            "subagent".into(),
            Some(parent.id),
            Some("sub_agent".into()),
            None,
        )
        .await
        .unwrap();
    store
        .update_session_title(child.id, "Busy".into())
        .await
        .unwrap();
    state
        .global_store()
        .upsert_workspace_session_index(child.id, parent.workspace_id)
        .await
        .unwrap();

    let turn = SessionTurn {
        turn_id: TurnId::new(),
        session_id: child.id,
        run_id: Some(RunId::new()),
        user_message_id: None,
        status: SessionTurnStatus::Running,
        start_seq: Some(1),
        end_seq: None,
        started_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        assistant_partial: None,
        thought_partial: None,
        metrics_json: None,
        tool_total: 0,
        tool_pending: 0,
        tool_running: 0,
        tool_completed: 0,
        tool_failed: 0,
    };
    store.insert_session_turn(turn).await.unwrap();

    let client = &server.client;
    let base = &server.base_url;
    let resp = client
        .post(format!(
            "{base}/api/mcp/sessions/{parent_id}/subagent_reply"
        ))
        .json(&json!({ "label": "Busy", "prompt": "hi" }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::CONFLICT);
    let body: serde_json::Value = resp.json().await.unwrap();
    let error = body["error"].as_str().unwrap_or("");
    assert!(error.contains("subagent_wait"));
    assert!(error.contains("subagent_interrupt"));
}

#[tokio::test]
async fn subagent_interrupt_all_includes_context_window() {
    let repo = common::init_git_repo(&[("README.md", "ok")]).await;
    let (_data_dir, state, server, store, parent_id) = setup_state(repo.path()).await;
    let parent = store
        .get_session(SessionId(Uuid::parse_str(&parent_id).unwrap()))
        .await
        .unwrap()
        .unwrap();

    for label in ["Alpha", "Beta"] {
        let child = store
            .create_session(
                parent.task_id,
                parent.workspace_id,
                parent.worktree_id,
                parent.execution_environment,
                "fake".into(),
                "fake-model".into(),
                "subagent".into(),
                Some(parent.id),
                Some("sub_agent".into()),
                None,
            )
            .await
            .unwrap();
        store
            .update_session_title(child.id, label.into())
            .await
            .unwrap();
        state
            .global_store()
            .upsert_workspace_session_index(child.id, parent.workspace_id)
            .await
            .unwrap();

        let turn = SessionTurn {
            turn_id: TurnId::new(),
            session_id: child.id,
            run_id: Some(RunId::new()),
            user_message_id: None,
            status: SessionTurnStatus::Completed,
            start_seq: Some(1),
            end_seq: Some(2),
            started_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            assistant_partial: None,
            thought_partial: None,
            metrics_json: Some(json!({
                "context_window_tokens": 100,
                "context_tokens_estimate": 40,
                "remaining_tokens_estimate": 60,
                "remaining_fraction": 0.6
            })),
            tool_total: 0,
            tool_pending: 0,
            tool_running: 0,
            tool_completed: 0,
            tool_failed: 0,
        };
        store.insert_session_turn(turn).await.unwrap();
    }

    let client = &server.client;
    let base = &server.base_url;
    let resp = client
        .post(format!(
            "{base}/api/mcp/sessions/{parent_id}/subagent_interrupt"
        ))
        .json(&json!({ "all": true }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    let results = body["results"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    let ctx = results[0]["context_window"].as_object().unwrap();
    assert_eq!(ctx.len(), 4);
    assert!(ctx.contains_key("total"));
    assert!(ctx.contains_key("used"));
    assert!(ctx.contains_key("remaining"));
    assert!(ctx.contains_key("utilization"));
}

#[tokio::test]
async fn subagent_wait_fails_when_child_exits_without_terminal_event() {
    let repo = common::init_git_repo(&[("README.md", "ok")]).await;
    let (_data_dir, _state, server, _store, parent_id) = setup_state(repo.path()).await;
    let client = &server.client;
    let base = &server.base_url;

    let init_resp = client
        .post(format!("{base}/api/mcp/sessions/{parent_id}/subagent_init"))
        .json(&json!({
            "worktree": "inherit",
            "agents": [
                {
                    "prompt": "omit-terminal-event",
                    "label": "MissingTerminal",
                    "harness": "fake",
                    "model": "fake-model"
                }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(init_resp.status(), StatusCode::OK);

    let wait_resp = client
        .post(format!("{base}/api/mcp/sessions/{parent_id}/subagent_wait"))
        .json(&json!({ "label": "MissingTerminal" }))
        .send()
        .await
        .unwrap();

    assert_eq!(wait_resp.status(), StatusCode::OK);
    let body: serde_json::Value = wait_resp.json().await.unwrap();
    assert_eq!(body["status"], "failed");
    assert_eq!(body["results"][0]["label"], "MissingTerminal");
    assert_eq!(body["results"][0]["status"], "failed");
}

#[tokio::test]
async fn subagent_wait_fails_when_child_finishes_without_reporting_outcome() {
    let repo = common::init_git_repo(&[("README.md", "ok")]).await;
    let mut providers = common::fake_providers();
    providers.insert("broken".into(), Arc::new(BrokenOutcomeProviderAdapter));
    let (_data_dir, _state, server, _store, parent_id) =
        setup_state_with_providers(repo.path(), providers).await;
    let client = &server.client;
    let base = &server.base_url;

    let init_resp = client
        .post(format!("{base}/api/mcp/sessions/{parent_id}/subagent_init"))
        .json(&json!({
            "worktree": "inherit",
            "agents": [
                {
                    "prompt": "done-without-outcome",
                    "label": "MissingOutcome",
                    "harness": "broken",
                    "model": "broken-model"
                }
            ]
        }))
        .send()
        .await
        .unwrap();
    let init_status = init_resp.status();
    let init_body = init_resp.text().await.unwrap();
    assert_eq!(init_status, StatusCode::OK, "{init_body}");

    let wait_resp = client
        .post(format!("{base}/api/mcp/sessions/{parent_id}/subagent_wait"))
        .json(&json!({ "label": "MissingOutcome" }))
        .send()
        .await
        .unwrap();

    assert_eq!(wait_resp.status(), StatusCode::OK);
    let body: serde_json::Value = wait_resp.json().await.unwrap();
    assert_eq!(body["status"], "failed");
    assert_eq!(body["results"][0]["label"], "MissingOutcome");
    assert_eq!(body["results"][0]["status"], "failed");
}

#[tokio::test]
async fn subagent_wait_fails_when_child_closes_outcome_without_reporting() {
    let repo = common::init_git_repo(&[("README.md", "ok")]).await;
    let mut providers = common::fake_providers();
    providers.insert("broken".into(), Arc::new(BrokenOutcomeProviderAdapter));
    let (_data_dir, _state, server, _store, parent_id) =
        setup_state_with_providers(repo.path(), providers).await;
    let client = &server.client;
    let base = &server.base_url;

    let init_resp = client
        .post(format!("{base}/api/mcp/sessions/{parent_id}/subagent_init"))
        .json(&json!({
            "worktree": "inherit",
            "agents": [
                {
                    "prompt": "done-close-outcome-no-abort",
                    "label": "ClosedOutcome",
                    "harness": "broken",
                    "model": "broken-model"
                }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(init_resp.status(), StatusCode::OK);

    let wait_resp = client
        .post(format!("{base}/api/mcp/sessions/{parent_id}/subagent_wait"))
        .json(&json!({ "label": "ClosedOutcome" }))
        .send()
        .await
        .unwrap();

    assert_eq!(wait_resp.status(), StatusCode::OK);
    let body: serde_json::Value = wait_resp.json().await.unwrap();
    assert_eq!(body["status"], "failed");
    assert_eq!(body["results"][0]["label"], "ClosedOutcome");
    assert_eq!(body["results"][0]["status"], "failed");
}

#[tokio::test]
async fn subagent_wait_fails_when_child_stalls_without_done_or_outcome() {
    let repo = common::init_git_repo(&[("README.md", "ok")]).await;
    let mut providers = common::fake_providers();
    providers.insert("broken".into(), Arc::new(BrokenOutcomeProviderAdapter));
    let (_data_dir, state, server, _store, parent_id) =
        setup_state_with_providers(repo.path(), providers).await;
    state
        .set_provider_inactivity_timeout(Duration::from_millis(250))
        .await;
    let client = &server.client;
    let base = &server.base_url;

    let init_resp = client
        .post(format!("{base}/api/mcp/sessions/{parent_id}/subagent_init"))
        .json(&json!({
            "worktree": "inherit",
            "agents": [
                {
                    "prompt": "stall-without-outcome",
                    "label": "StalledOutcome",
                    "harness": "broken",
                    "model": "broken-model"
                }
            ]
        }))
        .send()
        .await
        .unwrap();
    let init_status = init_resp.status();
    let init_body = init_resp.text().await.unwrap();
    assert_eq!(init_status, StatusCode::OK, "{init_body}");

    let wait_resp = client
        .post(format!("{base}/api/mcp/sessions/{parent_id}/subagent_wait"))
        .json(&json!({ "label": "StalledOutcome" }))
        .send()
        .await
        .unwrap();

    assert_eq!(wait_resp.status(), StatusCode::OK);
    let body: serde_json::Value = wait_resp.json().await.unwrap();
    assert_eq!(body["status"], "failed");
    assert_eq!(body["results"][0]["label"], "StalledOutcome");
    assert_eq!(body["results"][0]["status"], "failed");
}

#[tokio::test]
async fn subagent_interrupt_does_not_override_completed_child_outcome() {
    let repo = common::init_git_repo(&[("README.md", "ok")]).await;
    let (_data_dir, _state, server, store, parent_id) = setup_state(repo.path()).await;
    let client = &server.client;
    let base = &server.base_url;

    let init_resp = client
        .post(format!("{base}/api/mcp/sessions/{parent_id}/subagent_init"))
        .json(&json!({
            "worktree": "inherit",
            "agents": [
                {
                    "prompt": "slow-diff-test complete-on-cancel",
                    "label": "CancelCompletes",
                    "harness": "fake",
                    "model": "fake-model"
                }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(init_resp.status(), StatusCode::OK);

    sleep(Duration::from_millis(100)).await;

    let interrupt_resp = client
        .post(format!(
            "{base}/api/mcp/sessions/{parent_id}/subagent_interrupt"
        ))
        .json(&json!({ "label": "CancelCompletes" }))
        .send()
        .await
        .unwrap();
    assert_eq!(interrupt_resp.status(), StatusCode::OK);

    let wait_resp = client
        .post(format!("{base}/api/mcp/sessions/{parent_id}/subagent_wait"))
        .json(&json!({ "label": "CancelCompletes" }))
        .send()
        .await
        .unwrap();

    assert_eq!(wait_resp.status(), StatusCode::OK);
    let body: serde_json::Value = wait_resp.json().await.unwrap();
    assert_eq!(body["status"], "completed");
    assert_eq!(body["results"][0]["label"], "CancelCompletes");
    assert_eq!(body["results"][0]["status"], "completed");

    let parent = store
        .get_session(SessionId(Uuid::parse_str(&parent_id).unwrap()))
        .await
        .unwrap()
        .unwrap();
    let child = store
        .get_subagent_session_by_label(parent.id, "CancelCompletes")
        .await
        .unwrap()
        .expect("child session");
    let turn = store
        .list_session_turns_page_by_seq(child.id, None, Some(1))
        .await
        .unwrap()
        .into_iter()
        .next()
        .expect("child turn");
    assert_eq!(turn.status, SessionTurnStatus::Completed);

    let events = store
        .list_session_events_for_turn(child.id, turn.turn_id, false)
        .await
        .unwrap();
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.event_type, SessionEventType::TurnInterrupted)),
        "completed-on-cancel flow should not persist an interrupted event"
    );
}

#[tokio::test]
async fn subagent_interrupt_falls_back_to_interrupted_when_child_never_reports_outcome() {
    let repo = common::init_git_repo(&[("README.md", "ok")]).await;
    let mut providers = common::fake_providers();
    providers.insert("broken".into(), Arc::new(BrokenOutcomeProviderAdapter));
    let (_data_dir, _state, server, store, parent_id) =
        setup_state_with_providers(repo.path(), providers).await;
    let client = &server.client;
    let base = &server.base_url;

    let init_resp = client
        .post(format!("{base}/api/mcp/sessions/{parent_id}/subagent_init"))
        .json(&json!({
            "worktree": "inherit",
            "agents": [
                {
                    "prompt": "cancel-without-outcome",
                    "label": "MissingInterruptOutcome",
                    "harness": "broken",
                    "model": "broken-model"
                }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(init_resp.status(), StatusCode::OK);

    let interrupt_resp = client
        .post(format!(
            "{base}/api/mcp/sessions/{parent_id}/subagent_interrupt"
        ))
        .json(&json!({ "label": "MissingInterruptOutcome" }))
        .send()
        .await
        .unwrap();
    assert_eq!(interrupt_resp.status(), StatusCode::OK);

    let wait_resp = client
        .post(format!("{base}/api/mcp/sessions/{parent_id}/subagent_wait"))
        .json(&json!({ "label": "MissingInterruptOutcome" }))
        .send()
        .await
        .unwrap();

    assert_eq!(wait_resp.status(), StatusCode::OK);
    let body: serde_json::Value = wait_resp.json().await.unwrap();
    assert_eq!(body["status"], "interrupted");
    assert_eq!(body["results"][0]["label"], "MissingInterruptOutcome");
    assert_eq!(body["results"][0]["status"], "interrupted");

    let parent = store
        .get_session(SessionId(Uuid::parse_str(&parent_id).unwrap()))
        .await
        .unwrap()
        .unwrap();
    let child = store
        .get_subagent_session_by_label(parent.id, "MissingInterruptOutcome")
        .await
        .unwrap()
        .expect("child session");
    let turn = store
        .list_session_turns_page_by_seq(child.id, None, Some(1))
        .await
        .unwrap()
        .into_iter()
        .next()
        .expect("child turn");
    let events = store
        .list_session_events_for_turn(child.id, turn.turn_id, false)
        .await
        .unwrap();
    assert!(
        events
            .iter()
            .any(|event| matches!(event.event_type, SessionEventType::TurnInterrupted)),
        "fallback interrupt should persist TurnInterrupted"
    );
}

#[tokio::test]
async fn subagent_interrupt_falls_back_to_interrupted_when_child_closes_outcome() {
    let repo = common::init_git_repo(&[("README.md", "ok")]).await;
    let mut providers = common::fake_providers();
    providers.insert("broken".into(), Arc::new(BrokenOutcomeProviderAdapter));
    let (_data_dir, _state, server, store, parent_id) =
        setup_state_with_providers(repo.path(), providers).await;
    let client = &server.client;
    let base = &server.base_url;

    let init_resp = client
        .post(format!("{base}/api/mcp/sessions/{parent_id}/subagent_init"))
        .json(&json!({
            "worktree": "inherit",
            "agents": [
                {
                    "prompt": "cancel-close-outcome-no-abort",
                    "label": "ClosedInterruptOutcome",
                    "harness": "broken",
                    "model": "broken-model"
                }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(init_resp.status(), StatusCode::OK);

    let interrupt_resp = client
        .post(format!(
            "{base}/api/mcp/sessions/{parent_id}/subagent_interrupt"
        ))
        .json(&json!({ "label": "ClosedInterruptOutcome" }))
        .send()
        .await
        .unwrap();
    assert_eq!(interrupt_resp.status(), StatusCode::OK);

    let wait_resp = client
        .post(format!("{base}/api/mcp/sessions/{parent_id}/subagent_wait"))
        .json(&json!({ "label": "ClosedInterruptOutcome" }))
        .send()
        .await
        .unwrap();

    assert_eq!(wait_resp.status(), StatusCode::OK);
    let body: serde_json::Value = wait_resp.json().await.unwrap();
    assert_eq!(body["status"], "interrupted");
    assert_eq!(body["results"][0]["label"], "ClosedInterruptOutcome");
    assert_eq!(body["results"][0]["status"], "interrupted");

    let parent = store
        .get_session(SessionId(Uuid::parse_str(&parent_id).unwrap()))
        .await
        .unwrap()
        .unwrap();
    let child = store
        .get_subagent_session_by_label(parent.id, "ClosedInterruptOutcome")
        .await
        .unwrap()
        .expect("child session");
    let turn = store
        .list_session_turns_page_by_seq(child.id, None, Some(1))
        .await
        .unwrap()
        .into_iter()
        .next()
        .expect("child turn");
    let events = store
        .list_session_events_for_turn(child.id, turn.turn_id, false)
        .await
        .unwrap();
    assert!(
        events
            .iter()
            .any(|event| matches!(event.event_type, SessionEventType::TurnInterrupted)),
        "closed-channel fallback interrupt should persist TurnInterrupted"
    );
}

#[tokio::test]
async fn subagent_init_worktree_new_runs_bootstrap() {
    let repo = common::init_git_repo(&[("README.md", "ok")]).await;
    let (_data_dir, _state, server, store, parent_id) = setup_state(repo.path()).await;
    ctx_workspace_config::update_worktree_bootstrap_config(
        &store,
        ctx_workspace_config::WorktreeBootstrapConfigUpdate {
            setup_command: Some(
                "sh -c \"mkdir -p .ctx && echo bootstrapped > .ctx/bootstrap.txt\"".to_string(),
            ),
            timeout_sec: None,
            wait_for_completion: Some(true),
        },
    )
    .await
    .unwrap();
    let client = &server.client;
    let base = &server.base_url;

    let resp = client
        .post(format!("{base}/api/mcp/sessions/{parent_id}/subagent_init"))
        .json(&json!({
            "worktree": "new",
            "agents": [
                { "prompt": "boot", "label": "Bootstrap", "harness": "fake", "model": "fake-model" }
            ]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    let worktree_path = body["results"][0]["worktree_path"]
        .as_str()
        .expect("missing worktree_path");
    assert_ne!(worktree_path, repo.path().to_string_lossy());
    assert!(Path::new(worktree_path).exists());

    let bootstrap_path = Path::new(worktree_path).join(".ctx/bootstrap.txt");
    let mut found = false;
    for _ in 0..20 {
        if bootstrap_path.exists() {
            found = true;
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }
    assert!(found, "bootstrap did not create marker file");
}
