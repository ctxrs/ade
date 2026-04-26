use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::{routing::get, routing::post, Json, Router};
use ctx_core::models::{ExecutionEnvironment, Session, Task, VcsKind, Workspace, Worktree};
use ctx_http::daemon::AppState;
use ctx_providers::adapters::ProviderAdapter;
use ctx_providers::fake::FakeProviderAdapter;
use ctx_store::StoreManager;
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tower::ServiceBuilder;
use uuid::Uuid;

fn mcp_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ctx-mcp")
}

fn mcp_command() -> Command {
    let mut command = Command::new(mcp_bin());
    for key in [
        "CTX_AUTH_TOKEN",
        "CTX_BUNDLE_DIR",
        "CTX_BUILD_IDENTITY_PATH",
        "CTX_DAEMON_URL",
        "CTX_MCP_DEV_MODE",
        "CTX_SESSION_ID",
        "CTX_WORKTREE_ID",
        "CTX_WORKTREE_ROOT",
    ] {
        command.env_remove(key);
    }
    command
}

async fn run_git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn run_git_output(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

async fn init_git_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    run_git(root, &["init"]).await;
    run_git(root, &["config", "user.email", "test@example.com"]).await;
    run_git(root, &["config", "user.name", "Test"]).await;
    tokio::fs::write(root.join("README.md"), "ok\n")
        .await
        .unwrap();
    run_git(root, &["add", "."]).await;
    run_git(root, &["commit", "-m", "init"]).await;
    dir
}

async fn setup_daemon_backed_parent_session() -> (
    tempfile::TempDir,
    tempfile::TempDir,
    Arc<AppState>,
    String,
    String,
) {
    let repo = init_git_repo().await;
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");

    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));
    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores.clone(),
        providers,
        base_url.clone(),
        None,
    ));
    state.providers.statuses.lock().await.insert(
        "fake".into(),
        FakeProviderAdapter::new().inspect().await.unwrap(),
    );

    let app = ctx_http::api::router(state.clone());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let workspace: Workspace = stores
        .global()
        .create_workspace(
            "test".into(),
            repo.path().to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();
    let store = stores.workspace(workspace.id).await.unwrap();
    let base_commit = run_git_output(repo.path(), &["rev-parse", "HEAD"]).await;
    let worktree: Worktree = store
        .create_worktree(
            workspace.id,
            repo.path().to_string_lossy().to_string(),
            base_commit,
            None,
        )
        .await
        .unwrap();
    let task: Task = store
        .create_task(workspace.id, "task".into(), None)
        .await
        .unwrap();
    let session: Session = store
        .create_session(
            task.id,
            workspace.id,
            worktree.id,
            ExecutionEnvironment::Host,
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
        .upsert_workspace_session_index(session.id, workspace.id)
        .await
        .unwrap();
    state
        .global_store()
        .upsert_workspace_worktree_index(worktree.id, workspace.id)
        .await
        .unwrap();
    state
        .global_store()
        .upsert_workspace_task_index(task.id, workspace.id)
        .await
        .unwrap();

    (repo, data_dir, state, base_url, session.id.0.to_string())
}

async fn wait_for_response(
    reader: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    response_id: i64,
) -> serde_json::Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let next_line = tokio::time::timeout_at(deadline, reader.next_line())
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for response id {response_id}"));
        let Some(line) = next_line.unwrap() else {
            break;
        };
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        if value.get("id").and_then(|id| id.as_i64()) == Some(response_id) {
            return value;
        }
    }
    panic!("timed out waiting for response id {response_id}");
}

#[path = "subagent_tools/daemon_http.rs"]
mod daemon_http;
#[path = "subagent_tools/e2e_router.rs"]
mod e2e_router;
#[path = "subagent_tools/live_provider.rs"]
mod live_provider;
