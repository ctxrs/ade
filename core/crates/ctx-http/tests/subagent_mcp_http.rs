use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use serde_json::json;
use tokio::time::sleep;

use ctx_core::ids::{RunId, SessionId, TurnId};
use ctx_core::models::{SessionTurn, SessionTurnStatus, VcsKind};
use ctx_http::daemon::AppState;
use ctx_providers::adapters::ProviderAdapter;
use ctx_providers::fake::FakeProviderAdapter;
use ctx_store::Store;
use uuid::Uuid;

mod common;

async fn setup_state(
    repo_root: &Path,
) -> (
    tempfile::TempDir,
    Arc<AppState>,
    common::TestServer,
    Store,
    String,
) {
    let data_dir = tempfile::tempdir().unwrap();
    let stores = common::setup_store(data_dir.path()).await;
    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores.clone(),
        common::fake_providers(),
        "http://127.0.0.1:0",
    );
    {
        let status = FakeProviderAdapter::new().inspect().await.unwrap();
        state
            .providers
            .statuses
            .lock()
            .await
            .insert("fake".into(), status);
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
async fn subagent_init_worktree_new_runs_bootstrap() {
    let repo = common::init_git_repo(&[(
        ".ctx/config.toml",
        "[worktree.bootstrap]\nsetup_worktree_unix = [\"sh -c \\\"mkdir -p .ctx && echo bootstrapped > .ctx/bootstrap.txt\\\"\"]\nwait_for_completion = true\n",
    )])
    .await;
    let (_data_dir, _state, server, _store, parent_id) = setup_state(repo.path()).await;
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
