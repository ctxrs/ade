pub mod api;
pub mod async_util;
pub mod attachments;
pub mod buffers;
pub mod completions;
pub mod daemon;
pub mod dictation_livekit;
pub mod edit_plans;
pub mod git_status;
pub mod installer;
pub mod installs;
pub mod llm;
pub mod logs;
pub mod lsp_catalog;
pub mod merge_queue;
pub mod mobile_e2ee;
pub mod mobile_tunnel;
pub mod ops_events;
pub mod oracle;
pub mod perf_telemetry;
pub mod provider_accounts;
pub mod provider_child_reclassifier;
pub mod provider_debug;
pub mod provider_guard;
pub mod provider_matrix;
pub mod provider_restart;
pub mod provider_usage;
pub mod resource_governance;
pub mod resource_telemetry;
pub mod resource_utilization;
pub mod scheduler;
pub mod settings;
pub mod telemetry;
pub mod terminals;
pub mod title_generation;
pub mod tool_cgroup;
pub mod updates;
pub mod web_sessions;
pub mod workspace_active_snapshot;
pub mod workspace_config;
pub mod worktree_bootstrap;

#[cfg(feature = "fault_injection")]
pub mod fault_injection;

#[cfg(not(feature = "fault_injection"))]
pub mod fault_injection {
    pub fn clear_failpoints() {}
    pub fn set_failpoint(_point: &'static str, _times: u32) {}
    pub fn maybe_fail(_point: &'static str) -> anyhow::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::Arc;
    use std::time::Duration;

    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use futures::{SinkExt, StreamExt};
    use serde_json::json;
    use tokio::process::Command;
    use tower::ServiceExt;

    use ctx_providers::adapters::ProviderStatus;
    use ctx_providers::fake::FakeProviderAdapter;
    use ctx_store::StoreManager;

    use crate::api;
    use crate::daemon::AppState;

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

    #[tokio::test]
    async fn daemon_golden_path_with_fake_provider() {
        let git_repo = setup_git_repo().await;
        let home = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", home.path());

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
        let mut attempts = 0;
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
            attempts += 1;
            if attempts > 50 {
                panic!("assistant message not produced");
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    #[tokio::test]
    async fn daemon_http_and_ws_streaming() {
        let git_repo = setup_git_repo().await;
        let home = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", home.path());

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
                },
            );
            *state.provider_statuses.lock().await = statuses;
        }

        let app = api::router(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let base = format!("http://{}", addr);
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
                "after_seq": 0,
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
        let mut seen_done = false;
        while let Some(Ok(frame)) = ws_stream.next().await {
            if let tokio_tungstenite::tungstenite::Message::Text(txt) = frame {
                let value: serde_json::Value = serde_json::from_str(&txt).unwrap();
                let message: ctx_core::models::WorkspaceActiveSnapshotStreamMessage =
                    serde_json::from_value(value).unwrap();
                match message {
                    ctx_core::models::WorkspaceActiveSnapshotStreamMessage::Event {
                        event:
                            ctx_core::models::WorkspaceActiveSnapshotEvent::SessionHeadDelta {
                                delta,
                                ..
                            },
                        ..
                    } => {
                        if delta.session_id == session.id
                            && delta
                                .event
                                .as_ref()
                                .map(|event| {
                                    matches!(
                                        event.event_type,
                                        ctx_core::models::SessionEventType::Done
                                    )
                                })
                                .unwrap_or(false)
                        {
                            seen_done = true;
                            break;
                        }
                    }
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
    async fn web_session_routes_are_registered() {
        let home = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", home.path());

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
}
