use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use futures::{SinkExt, StreamExt};
use serde_json::json;
use tokio::process::Command;
use tower::ServiceExt;

use ctx_execution_runtime::{ExecutionLaunchSnapshot, ExecutionLaunchState, ExecutionSetupJobKind};
use ctx_providers::adapters::ProviderStatus;
use ctx_providers::fake::FakeProviderAdapter;
use ctx_store::StoreManager;

use crate::api;
use crate::daemon::AppState;
use crate::storage_guard::{StorageGuardLevel, StorageGuardPathStatus, StorageGuardStatus};

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
    std::fs::write(root.join("file.txt"), "hello\n").unwrap();
    run_git(root, &["add", "."]).await;
    run_git(root, &["commit", "-m", "init"]).await;
    dir
}

async fn create_workspace_via_api(
    app: &axum::Router,
    root_path: &str,
) -> ctx_core::models::Workspace {
    let req = Request::builder()
        .method("POST")
        .uri("/api/workspaces")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "root_path": root_path,
                "name": "ws"
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

fn write_failing_sandbox_cli_shim(dir: &Path) -> std::path::PathBuf {
    let path = dir.join(if cfg!(windows) {
        "sandbox-cli-fail-fast.cmd"
    } else {
        "sandbox-cli-fail-fast.sh"
    });
    let script = if cfg!(windows) {
        "@echo off\r\n>&2 echo sandbox CLI unavailable\r\nexit /b 125\r\n"
    } else {
        "#!/bin/sh\necho 'sandbox CLI unavailable' >&2\nexit 125\n"
    };
    std::fs::write(&path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
}

struct EnvVarGuard {
    key: &'static str,
    prev: Option<String>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let prev = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, prev }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(v) = self.prev.take() {
            std::env::set_var(self.key, v);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

fn sandbox_cli_env_test_lock() -> &'static tokio::sync::Mutex<()> {
    crate::test_support::sandbox_cli_env_test_lock()
}

fn home_env_test_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

#[tokio::test]
async fn daemon_golden_path_with_fake_provider() {
    let _serial = home_env_test_lock().lock().await;
    let git_repo = setup_git_repo().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();

    let mut providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
        HashMap::new();
    providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));

    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:4399".to_string(),
        None,
    ));
    let app = api::router(state.clone());

    // create workspace
    let req = Request::builder()
        .method("POST")
        .uri("/api/workspaces")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "root_path": git_repo.path().to_string_lossy(),
                "name": "ws"
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let ws: ctx_core::models::Workspace = serde_json::from_slice(&body).unwrap();

    // create task (auto worktree when requested)
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/workspaces/{}/tasks", ws.id.0))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"title":"t1","description":null}).to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let task: ctx_core::models::Task = serde_json::from_slice(&body).unwrap();

    // create session
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/tasks/{}/sessions", task.id.0))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"provider_id":"fake","model_id":"fake-model"}).to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let session: ctx_core::models::Session = serde_json::from_slice(&body).unwrap();

    // post message
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/sessions/{}/messages", session.id.0))
        .header("content-type", "application/json")
        .body(Body::from(json!({"content":"hello"}).to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // wait for assistant message to be inserted
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let session_store = state.store_for_session(session.id).await.unwrap();
            let msgs = session_store
                .list_messages_for_session(session.id)
                .await
                .unwrap();
            if msgs
                .iter()
                .any(|m| matches!(m.role, ctx_core::models::MessageRole::Assistant))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("assistant message not produced"));
}

#[tokio::test]
async fn daemon_http_and_ws_streaming() {
    let _serial = home_env_test_lock().lock().await;
    let git_repo = setup_git_repo().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();

    let mut providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
        HashMap::new();
    providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));

    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:4399".to_string(),
        None,
    ));
    {
        let mut statuses = HashMap::new();
        statuses.insert(
            "fake".into(),
            ProviderStatus {
                provider_id: "fake".into(),
                installed: true,
                detected_path: None,
                version: Some("0.1.0".into()),
                capabilities: None,
                health: ctx_providers::adapters::ProviderHealth::Ok,
                diagnostics: vec![],
                details: HashMap::new(),
                usability: ctx_providers::adapters::ProviderUsability::default(),
            },
        );
        *state.providers.statuses.lock().await = statuses;
    }

    let app = api::router(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let base = format!("http://{addr}");
    let client = reqwest::Client::new();

    // providers endpoint
    let providers_res: Vec<ProviderStatus> = client
        .get(format!("{base}/api/providers"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(providers_res.len(), 1);

    // create workspace
    let ws: ctx_core::models::Workspace = client
        .post(format!("{base}/api/workspaces"))
        .json(&json!({
            "root_path": git_repo.path().to_string_lossy(),
            "name": "ws"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    // create task
    let task: ctx_core::models::Task = client
        .post(format!("{base}/api/workspaces/{}/tasks", ws.id.0))
        .json(&json!({"title":"t1"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    // create session
    let session: ctx_core::models::Session = client
        .post(format!("{base}/api/tasks/{}/sessions", task.id.0))
        .json(&json!({"provider_id":"fake","model_id":"fake-model"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    // open workspace stream before sending message
    let ws_url = format!("ws://{}/api/workspaces/{}/stream", addr, ws.id.0);
    let (mut ws_stream, _) = tokio_tungstenite::connect_async(ws_url).await.unwrap();
    let subscribe = serde_json::json!({
        "type": "subscribe",
        "sessions": [{
            "session_id": session.id.0,
            "replay": {
                "mode": "resume",
                "after_seq": 0,
            },
        }],
    })
    .to_string();
    ws_stream
        .send(tokio_tungstenite::tungstenite::Message::Text(
            subscribe.into(),
        ))
        .await
        .unwrap();

    // post message
    let _msg: ctx_core::models::Message = client
        .post(format!("{base}/api/sessions/{}/messages", session.id.0))
        .json(&json!({"content":"hello"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    // consume workspace stream until we see a Done event for the session
    let seen_done = tokio::time::timeout(Duration::from_secs(20), async {
        let mut seen_done = false;
        while let Some(Ok(frame)) = ws_stream.next().await {
            if let tokio_tungstenite::tungstenite::Message::Text(txt) = frame {
                let message: ctx_core::models::WorkspaceActiveSnapshotStreamMessage =
                    serde_json::from_str(&txt).unwrap_or_else(|err| {
                        panic!("failed to decode workspace stream message: {err}; raw={txt}")
                    });
                match message {
                    ctx_core::models::WorkspaceActiveSnapshotStreamMessage::Event {
                        event, ..
                    } => match event.as_ref() {
                        ctx_core::models::WorkspaceActiveSnapshotEvent::SessionHeadDelta {
                            delta,
                            ..
                        } => {
                            let is_done = delta.session_id == session.id
                                && delta
                                    .event
                                    .as_ref()
                                    .map(|event| {
                                        matches!(
                                            event.event_type,
                                            ctx_core::models::SessionEventType::Done
                                        )
                                    })
                                    .unwrap_or(false);
                            if is_done {
                                seen_done = true;
                                break;
                            }
                        }
                        ctx_core::models::WorkspaceActiveSnapshotEvent::SessionHeadSeed {
                            head,
                            ..
                        } => {
                            let is_done = head.session.id == session.id
                                && head.events.iter().any(|event| {
                                    matches!(
                                        event.event_type,
                                        ctx_core::models::SessionEventType::Done
                                    )
                                });
                            if is_done {
                                seen_done = true;
                                break;
                            }
                        }
                        _ => {}
                    },
                    ctx_core::models::WorkspaceActiveSnapshotStreamMessage::HeadsBatch {
                        deltas,
                        ..
                    } => {
                        for delta in deltas {
                            if delta.session_id != session.id {
                                continue;
                            }
                            let is_done = delta
                                .event
                                .as_ref()
                                .map(|event| {
                                    matches!(
                                        event.event_type,
                                        ctx_core::models::SessionEventType::Done
                                    )
                                })
                                .unwrap_or(false);
                            if is_done {
                                seen_done = true;
                                break;
                            }
                        }
                        if seen_done {
                            break;
                        }
                    }
                    _ => {}
                }
            }
        }
        seen_done
    })
    .await
    .expect("timed out waiting for Done event");
    assert!(seen_done);

    let store = state.store_for_session(session.id).await.unwrap();
    let events = store.list_session_events(session.id).await.unwrap();
    assert!(events.iter().any(|e| matches!(
        e.event_type,
        ctx_core::models::SessionEventType::UserMessage
    )));

    // mark task read/unread endpoints
    let task_after_read: ctx_core::models::Task = client
        .post(format!("{base}/api/tasks/{}/mark_read", task.id.0))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(task_after_read.assistant_seen_at.is_some());

    let task_after_unread: ctx_core::models::Task = client
        .post(format!("{base}/api/tasks/{}/mark_unread", task.id.0))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(task_after_unread.assistant_seen_at.is_none());

    server.abort();
}

#[tokio::test]
async fn create_session_rejects_unknown_provider_id() {
    let _serial = home_env_test_lock().lock().await;
    let git_repo = setup_git_repo().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();

    let mut providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
        HashMap::new();
    providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));

    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:4399".to_string(),
        None,
    ));
    let app = api::router(state);

    let req = Request::builder()
        .method("POST")
        .uri("/api/workspaces")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "root_path": git_repo.path().to_string_lossy(),
                "name": "ws"
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let ws: ctx_core::models::Workspace = serde_json::from_slice(&body).unwrap();

    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/workspaces/{}/tasks", ws.id.0))
        .header("content-type", "application/json")
        .body(Body::from(json!({"title":"t1"}).to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let task: ctx_core::models::Task = serde_json::from_slice(&body).unwrap();

    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/tasks/{}/sessions", task.id.0))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"provider_id":"not-a-provider","model_id":"fake-model"}).to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn session_state_exposes_artifact_metadata_and_session_scoped_downloads() {
    let _serial = home_env_test_lock().lock().await;
    let git_repo = setup_git_repo().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();

    let mut providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
        HashMap::new();
    providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));

    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:4399".to_string(),
        None,
    ));
    let app = api::router(state);

    let workspace = create_workspace_via_api(&app, &git_repo.path().to_string_lossy()).await;

    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/workspaces/{}/tasks", workspace.id.0))
        .header("content-type", "application/json")
        .body(Body::from(json!({"title":"t1"}).to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let task: ctx_core::models::Task = serde_json::from_slice(&body).unwrap();

    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/tasks/{}/sessions", task.id.0))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"provider_id":"fake","model_id":"fake-model"}).to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let session: ctx_core::models::Session = serde_json::from_slice(&body).unwrap();

    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/tasks/{}/sessions", task.id.0))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"provider_id":"fake","model_id":"fake-model"}).to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let wrong_session: ctx_core::models::Session = serde_json::from_slice(&body).unwrap();

    let artifact_path = git_repo.path().join("artifact.txt");
    std::fs::write(&artifact_path, b"artifact-body\n").unwrap();

    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/sessions/{}/artifacts", session.id.0))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "artifacts": [
                    {
                        "absolute_file_path": artifact_path.to_string_lossy(),
                        "name": "artifact.txt",
                        "mime_type": "text/plain"
                    }
                ]
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let artifacts: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let artifact = &artifacts[0];
    let artifact_id = artifact["id"].as_str().expect("artifact id");

    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/sessions/{}/state", session.id.0))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let session_state: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        session_state["artifacts"][0]["id"].as_str(),
        Some(artifact_id)
    );
    assert_eq!(
        session_state["artifacts"][0]["absolute_path"].as_str(),
        Some(artifact_path.to_string_lossy().as_ref())
    );

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/sessions/{}/artifacts/{}",
            wrong_session.id.0, artifact_id
        ))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/sessions/{}/artifacts/{}",
            session.id.0, artifact_id
        ))
        .header(header::RANGE, "bytes=0-7")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        res.headers()
            .get(header::ACCEPT_RANGES)
            .and_then(|value| value.to_str().ok()),
        Some("bytes")
    );
    assert_eq!(
        res.headers()
            .get(header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok()),
        Some("private, max-age=0, must-revalidate")
    );
    assert_eq!(
        res.headers()
            .get(header::CONTENT_RANGE)
            .and_then(|value| value.to_str().ok()),
        Some("bytes 0-7/14")
    );
    let etag = res
        .headers()
        .get(header::ETAG)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .expect("artifact range etag");
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    assert_eq!(body.as_ref(), b"artifact");

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/sessions/{}/artifacts/{}",
            session.id.0, artifact_id
        ))
        .header(header::RANGE, "Bytes= 0 - 7")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    assert_eq!(body.as_ref(), b"artifact");

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/sessions/{}/artifacts/{}",
            session.id.0, artifact_id
        ))
        .header(header::RANGE, "bytes=0-7")
        .header(header::IF_RANGE, etag.as_str())
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    assert_eq!(body.as_ref(), b"artifact");

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/sessions/{}/artifacts/{}",
            session.id.0, artifact_id
        ))
        .header(header::RANGE, "bytes=999-1000")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(
        res.headers()
            .get(header::CONTENT_RANGE)
            .and_then(|value| value.to_str().ok()),
        Some("bytes */14")
    );

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/sessions/{}/artifacts/{}",
            session.id.0, artifact_id
        ))
        .header(header::RANGE, "bytes=999999999999999999999-")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::RANGE_NOT_SATISFIABLE);

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/sessions/{}/artifacts/{}",
            session.id.0, artifact_id
        ))
        .header(header::RANGE, "items=0-1")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    assert_eq!(body.as_ref(), b"artifact-body\n");

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/sessions/{}/artifacts/{}",
            session.id.0, artifact_id
        ))
        .header(header::RANGE, "bytes=0-0,2-2")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    assert_eq!(body.as_ref(), b"artifact-body\n");

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/sessions/{}/artifacts/{}",
            session.id.0, artifact_id
        ))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/plain")
    );
    assert_eq!(
        res.headers()
            .get(header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok()),
        Some("private, max-age=0, must-revalidate")
    );
    assert_eq!(
        res.headers()
            .get(header::ETAG)
            .and_then(|value| value.to_str().ok()),
        Some(etag.as_str())
    );
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    assert_eq!(body.as_ref(), b"artifact-body\n");

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/sessions/{}/artifacts/{}",
            session.id.0, artifact_id
        ))
        .header(header::IF_NONE_MATCH, &etag)
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::NOT_MODIFIED);
    assert_eq!(
        res.headers()
            .get(header::ETAG)
            .and_then(|value| value.to_str().ok()),
        Some(etag.as_str())
    );
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    assert!(body.is_empty());

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/sessions/{}/artifacts/{}",
            session.id.0, artifact_id
        ))
        .header(header::RANGE, "bytes=0-7")
        .header(header::IF_NONE_MATCH, &etag)
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::NOT_MODIFIED);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    assert!(body.is_empty());
}

#[tokio::test]
async fn web_session_routes_are_registered() {
    let _serial = home_env_test_lock().lock().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();

    let providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
        HashMap::new();

    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:4399".to_string(),
        None,
    ));
    let app = api::router(state);

    let req = Request::builder()
        .method("POST")
        .uri("/api/sessions/web")
        .header("content-type", "application/json")
        .body(Body::from(json!({"url": ""}).to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
        json!({"error":"url is required"})
    );

    let req = Request::builder()
        .method("GET")
        .uri("/sessions/web/does-not-exist/view")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn execution_launch_start_and_status_host_mode() {
    let _serial = home_env_test_lock().lock().await;
    let git_repo = setup_git_repo().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();

    let mut providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
        HashMap::new();
    providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));

    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:4399".to_string(),
        None,
    ));
    let app = api::router(state.clone());

    let req = Request::builder()
        .method("POST")
        .uri("/api/workspaces")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "root_path": git_repo.path().to_string_lossy(),
                "name": "ws"
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let ws: ctx_core::models::Workspace = serde_json::from_slice(&body).unwrap();

    let req = Request::builder()
        .method("POST")
        .uri("/api/execution/launch/start")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "workspace_id": ws.id.0.to_string(),
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let snapshot: ExecutionLaunchSnapshot = serde_json::from_slice(&body).unwrap();
    assert_eq!(snapshot.workspace_id, ws.id.0.to_string());
    assert!(matches!(
        snapshot.state,
        ExecutionLaunchState::Running | ExecutionLaunchState::Ready
    ));
    assert!(!snapshot.job_id.trim().is_empty());

    let mut status_snapshot = snapshot.clone();
    for _ in 0..20 {
        let req = Request::builder()
            .method("GET")
            .uri(format!(
                "/api/execution/launch/status?job_id={}",
                snapshot.job_id
            ))
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        status_snapshot = serde_json::from_slice(&body).unwrap();
        if status_snapshot.state == ExecutionLaunchState::Ready {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(status_snapshot.job_id, snapshot.job_id);
    assert_eq!(status_snapshot.state, ExecutionLaunchState::Ready);
}

#[tokio::test]
async fn execution_launch_startup_prewarm_kind_supported() {
    let _serial = sandbox_cli_env_test_lock().lock().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let data_dir = tempfile::tempdir().unwrap();
    let sandbox_cli_path = write_failing_sandbox_cli_shim(data_dir.path());
    let _sandbox_cli = EnvVarGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );
    let stores = StoreManager::open(data_dir.path()).await.unwrap();

    let mut providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
        HashMap::new();
    providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));

    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:4399".to_string(),
        None,
    ));
    let app = api::router(state.clone());

    let req = Request::builder()
        .method("POST")
        .uri("/api/execution/launch/start")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "kind": "startup_prewarm",
                "prewarm_scope": "builder",
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let snapshot: ExecutionLaunchSnapshot = serde_json::from_slice(&body).unwrap();
    assert_eq!(snapshot.kind, ExecutionSetupJobKind::StartupPrewarm);
    assert!(!snapshot.job_id.trim().is_empty());
    assert!(matches!(
        snapshot.state,
        ExecutionLaunchState::Running | ExecutionLaunchState::Error
    ));

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/execution/launch/status?job_id={}",
            snapshot.job_id
        ))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let mut status_snapshot: ExecutionLaunchSnapshot = serde_json::from_slice(&body).unwrap();
    assert_eq!(status_snapshot.job_id, snapshot.job_id);
    assert_eq!(status_snapshot.kind, ExecutionSetupJobKind::StartupPrewarm);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    while matches!(status_snapshot.state, ExecutionLaunchState::Running)
        && tokio::time::Instant::now() < deadline
    {
        tokio::time::sleep(Duration::from_millis(25)).await;
        let req = Request::builder()
            .method("GET")
            .uri(format!(
                "/api/execution/launch/status?job_id={}",
                snapshot.job_id
            ))
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        status_snapshot = serde_json::from_slice(&body).unwrap();
    }
    assert!(matches!(
        status_snapshot.state,
        ExecutionLaunchState::Running | ExecutionLaunchState::Ready | ExecutionLaunchState::Error
    ));
}

#[tokio::test]
async fn execution_launch_start_returns_bad_request_when_execution_settings_fail() {
    let _serial = home_env_test_lock().lock().await;
    let git_repo = setup_git_repo().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();

    let mut providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
        HashMap::new();
    providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));

    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:4399".to_string(),
        None,
    ));
    let app = api::router(state.clone());

    let workspace = create_workspace_via_api(&app, &git_repo.path().to_string_lossy()).await;
    let store = state.store_for_workspace(workspace.id).await.unwrap();
    store
        .upsert_runtime_settings_document(1, "{")
        .await
        .unwrap();

    let req = Request::builder()
        .method("POST")
        .uri("/api/execution/launch/start")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "workspace_id": workspace.id.0.to_string(),
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(value["error"]
        .as_str()
        .unwrap_or_default()
        .contains("workspace runtime settings"));
}

#[tokio::test]
async fn ensure_workspace_container_returns_bad_request_when_execution_settings_fail() {
    let _serial = home_env_test_lock().lock().await;
    let git_repo = setup_git_repo().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();

    let mut providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
        HashMap::new();
    providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));

    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:4399".to_string(),
        None,
    ));
    let app = api::router(state.clone());

    let workspace = create_workspace_via_api(&app, &git_repo.path().to_string_lossy()).await;
    let store = state.store_for_workspace(workspace.id).await.unwrap();
    store
        .upsert_runtime_settings_document(1, "{")
        .await
        .unwrap();

    let req = Request::builder()
        .method("POST")
        .uri(format!(
            "/api/workspaces/{}/harness_container/ensure",
            workspace.id.0
        ))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(value["error"]
        .as_str()
        .unwrap_or_default()
        .contains("workspace runtime settings"));
}

#[tokio::test]
async fn cors_preflight_allows_archived_endpoint_for_tauri_origin() {
    let _serial = home_env_test_lock().lock().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
        HashMap::new();
    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:4399".to_string(),
        Some("desktop-token".to_string()),
    ));
    let app = api::router(state);
    let req = Request::builder()
        .method(Method::OPTIONS)
        .uri("/api/workspaces/00000000-0000-0000-0000-000000000000/archived_task_summaries")
        .header(header::ORIGIN, "tauri://localhost")
        .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
        .header(
            header::ACCESS_CONTROL_REQUEST_HEADERS,
            "authorization,content-type,traceparent,x-ctx-run-id",
        )
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert!(
        res.status().is_success(),
        "expected successful preflight, got {}",
        res.status()
    );
    let origin = res
        .headers()
        .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
        .and_then(|value| value.to_str().ok());
    assert_eq!(origin, Some("tauri://localhost"));
    let allow_headers = res
        .headers()
        .get(header::ACCESS_CONTROL_ALLOW_HEADERS)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    assert!(allow_headers.contains("authorization"));
    assert!(allow_headers.contains("content-type"));
    assert!(allow_headers.contains("traceparent"));
    assert!(allow_headers.contains("x-ctx-run-id"));
}

#[tokio::test]
async fn cors_preflight_allows_health_endpoint_for_tauri_localhost_origin() {
    let _serial = home_env_test_lock().lock().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
        HashMap::new();
    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:4399".to_string(),
        Some("desktop-token".to_string()),
    ));
    let app = api::router(state);
    let req = Request::builder()
        .method(Method::OPTIONS)
        .uri("/api/health")
        .header(header::ORIGIN, "http://tauri.localhost")
        .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
        .header(header::ACCESS_CONTROL_REQUEST_HEADERS, "content-type")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert!(
        res.status().is_success(),
        "expected successful preflight, got {}",
        res.status()
    );
    let origin = res
        .headers()
        .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
        .and_then(|value| value.to_str().ok());
    assert_eq!(origin, Some("http://tauri.localhost"));
}

#[tokio::test]
async fn health_and_diagnostics_include_storage_guard_state() {
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
        HashMap::new();
    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:4399".to_string(),
        None,
    ));
    state.core.storage_guard.publish(StorageGuardStatus {
        level: StorageGuardLevel::Warning,
        reserve_file_active: true,
        active: Some(StorageGuardPathStatus {
            label: "CTX data root".to_string(),
            path: data_dir.path().to_string_lossy().to_string(),
            mount_point: "/".to_string(),
            free_bytes: 1_800_000_000,
            total_bytes: 10_000_000_000,
        }),
        ..StorageGuardStatus::default()
    });

    let app = api::router(state);

    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/health")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let health: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        health
            .pointer("/storage/level")
            .and_then(serde_json::Value::as_str),
        Some("warning"),
        "expected storage warning state in health payload: {health:#?}"
    );
    assert_eq!(
        health
            .pointer("/storage/active/label")
            .and_then(serde_json::Value::as_str),
        Some("CTX data root"),
        "expected active storage path in health payload: {health:#?}"
    );

    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/diagnostics")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let diagnostics: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        diagnostics
            .pointer("/daemon/storage/level")
            .and_then(serde_json::Value::as_str),
        Some("warning"),
        "expected storage warning state in diagnostics payload: {diagnostics:#?}"
    );
    assert_eq!(
        diagnostics
            .pointer("/daemon/storage/active/path")
            .and_then(serde_json::Value::as_str),
        Some(data_dir.path().to_string_lossy().as_ref()),
        "expected active storage path in diagnostics payload: {diagnostics:#?}"
    );
}
