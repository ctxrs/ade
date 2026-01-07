use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::json;
use tokio::process::Command;
use tokio::time::timeout;

use ctx_http::api;
use ctx_http::daemon::AppState;
use ctx_store::Store;

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

async fn setup_git_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    run_git(root, &["init"]).await;
    run_git(root, &["config", "user.email", "test@example.com"]).await;
    run_git(root, &["config", "user.name", "Test"]).await;
    std::fs::write(root.join("note.txt"), "hello\n").unwrap();
    run_git(root, &["add", "."]).await;
    run_git(root, &["commit", "-m", "init"]).await;
    dir
}

fn pick_free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .ok()
        .and_then(|listener| listener.local_addr().ok().map(|addr| addr.port()))
        .unwrap_or(0)
}

fn require_env_path(key: &str) -> Option<PathBuf> {
    let raw = std::env::var(key).ok()?;
    let path = PathBuf::from(raw);
    path.exists().then_some(path)
}

async fn wait_for_health(base: &str) -> bool {
    let client = reqwest::Client::new();
    for _ in 0..30 {
        if let Ok(resp) = client.get(format!("{base}/health")).send().await {
            if resp.status().is_success() {
                return true;
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    false
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires ctx-worker-gateway + ctx-worker-shim binaries in env"]
async fn remote_terminal_via_gateway() {
    let gateway_bin = match require_env_path("CTX_WORKER_GATEWAY_BIN") {
        Some(path) => path,
        None => {
            eprintln!("skipping: set CTX_WORKER_GATEWAY_BIN to run this test");
            return;
        }
    };
    let shim_bin = match require_env_path("CTX_WORKER_SHIM_BIN") {
        Some(path) => path,
        None => {
            eprintln!("skipping: set CTX_WORKER_SHIM_BIN to run this test");
            return;
        }
    };

    let repo = setup_git_repo().await;
    let data_dir = tempfile::tempdir().unwrap();
    let db_dir = data_dir.path().join("db");
    tokio::fs::create_dir_all(&db_dir).await.unwrap();
    let db_path = db_dir.join("db.sqlite");
    let store = Store::open(&db_path).await.unwrap();

    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        store.clone(),
        HashMap::new(),
        "http://127.0.0.1:0".to_string(),
        None,
    ));
    state.start_workspace_catchup_listener();
    let app = api::router(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let daemon_addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let daemon_base = format!("http://{}", daemon_addr);
    let client = reqwest::Client::new();

    let gateway_port = pick_free_port();
    assert!(gateway_port > 0, "failed to pick a port");
    let gateway_base = format!("http://127.0.0.1:{gateway_port}");
    let mut gateway_child = Command::new(&gateway_bin)
        .arg("--driver")
        .arg("local")
        .arg("--bind")
        .arg(format!("127.0.0.1:{gateway_port}"))
        .arg("--public-base-url")
        .arg(&gateway_base)
        .arg("--worker-shim-path")
        .arg(&shim_bin)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    assert!(wait_for_health(&gateway_base).await, "gateway not healthy");

    let ws: ctx_core::models::Workspace = client
        .post(format!("{daemon_base}/api/workspaces"))
        .json(&json!({"root_path": repo.path(), "name": "ws"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let task: ctx_core::models::Task = client
        .post(format!("{daemon_base}/api/workspaces/{}/tasks", ws.id.0))
        .json(&json!({"title":"remote-terminals"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let tracks = store.list_tracks_for_task(task.id).await.unwrap();
    let track = &tracks[0];
    let worktree = store
        .get_worktree(track.worktree_id)
        .await
        .unwrap()
        .unwrap();

    let worker_resp = client
        .post(format!("{daemon_base}/api/tracks/{}/worker", track.id.0))
        .json(&json!({
            "gateway_url": gateway_base,
            "repo": {
                "Local": { "path": worktree.root_path }
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(worker_resp.status(), reqwest::StatusCode::OK);

    let terminal_resp = client
        .post(format!(
            "{daemon_base}/api/workspaces/{}/terminals",
            ws.id.0
        ))
        .json(&json!({
            "task_id": task.id.0,
            "track_id": track.id.0,
            "worktree_id": worktree.id.0,
            "cwd": worktree.root_path
        }))
        .send()
        .await
        .unwrap();
    if terminal_resp.status() != reqwest::StatusCode::OK {
        let body = terminal_resp.text().await.unwrap_or_default();
        panic!("terminal create failed: {}", body);
    }
    let terminal: ctx_core::models::TerminalSession = terminal_resp.json().await.unwrap();

    let ws_url = format!(
        "ws://{}/api/terminals/{}/stream",
        daemon_addr, terminal.id.0
    );
    let (mut ws_stream, _) = tokio_tungstenite::connect_async(ws_url).await.unwrap();
    ws_stream
        .send(tokio_tungstenite::tungstenite::Message::Text(
            "echo terminal_e2e\n".into(),
        ))
        .await
        .unwrap();

    let saw_output = timeout(Duration::from_secs(10), async {
        while let Some(msg) = ws_stream.next().await {
            let msg = msg.unwrap();
            if let tokio_tungstenite::tungstenite::Message::Binary(data) = msg {
                let text = String::from_utf8_lossy(&data);
                if text.contains("terminal_e2e") {
                    return true;
                }
            }
        }
        false
    })
    .await
    .unwrap_or(false);
    assert!(saw_output, "terminal output not observed");

    let _ = client
        .delete(format!("{daemon_base}/api/terminals/{}", terminal.id.0))
        .send()
        .await;
    let _ = client
        .delete(format!("{daemon_base}/api/tracks/{}/worker", track.id.0))
        .send()
        .await;

    let _ = gateway_child.kill().await;
    let _ = gateway_child.wait().await;
    server.abort();
}
