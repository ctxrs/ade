pub mod api;
pub mod daemon;
pub mod edit_plans;
pub mod installs;
pub mod installer;
pub mod logs;
pub mod scheduler;
pub mod updates;

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::Arc;
    use std::time::Duration;

    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use serde_json::json;
    use tokio::process::Command;
    use tower::ServiceExt;
    use futures::StreamExt;

    use context_providers::fake::FakeProviderAdapter;
    use context_providers::adapters::ProviderStatus;
    use context_store::Store;

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
        let db_dir = data_dir.path().join("db");
        tokio::fs::create_dir_all(&db_dir).await.unwrap();
        let db_path = db_dir.join("db.sqlite");
        let store = Store::open(&db_path).await.unwrap();

        let mut providers: HashMap<String, Arc<dyn context_providers::adapters::ProviderAdapter>> =
            HashMap::new();
        providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));

        let state = Arc::new(AppState::new(
            data_dir.path().to_path_buf(),
            store.clone(),
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
        let ws: context_core::models::Workspace = serde_json::from_slice(&body).unwrap();

        // create task (auto track + worktree)
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
        let task: context_core::models::Task = serde_json::from_slice(&body).unwrap();

        // list tracks
        let req = Request::builder()
            .method("GET")
            .uri(format!("/api/tasks/{}/tracks", task.id.0))
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let tracks: Vec<context_core::models::Track> = serde_json::from_slice(&body).unwrap();
        assert_eq!(tracks.len(), 1);
        let track = &tracks[0];

        // create session
        let req = Request::builder()
            .method("POST")
            .uri(format!("/api/tracks/{}/sessions", track.id.0))
            .header("content-type", "application/json")
            .body(Body::from(
                json!({"provider_id":"fake","model_id":"fake-model"}).to_string(),
            ))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let session: context_core::models::Session = serde_json::from_slice(&body).unwrap();

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
            let msgs = store.list_messages_for_session(session.id).await.unwrap();
            if msgs.iter().any(|m| matches!(m.role, context_core::models::MessageRole::Assistant)) {
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
        let db_dir = data_dir.path().join("db");
        tokio::fs::create_dir_all(&db_dir).await.unwrap();
        let db_path = db_dir.join("db.sqlite");
        let store = Store::open(&db_path).await.unwrap();

        let mut providers: HashMap<String, Arc<dyn context_providers::adapters::ProviderAdapter>> =
            HashMap::new();
        providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));

        let state = Arc::new(AppState::new(
            data_dir.path().to_path_buf(),
            store.clone(),
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
                    health: context_providers::adapters::ProviderHealth::Ok,
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
        let ws: context_core::models::Workspace = client
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
        let task: context_core::models::Task = client
            .post(format!("{base}/api/workspaces/{}/tasks", ws.id.0))
            .json(&json!({"title":"t1"}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();

        // list tracks
        let tracks: Vec<context_core::models::Track> = client
            .get(format!("{base}/api/tasks/{}/tracks", task.id.0))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let track = &tracks[0];

        // create session
        let session: context_core::models::Session = client
            .post(format!("{base}/api/tracks/{}/sessions", track.id.0))
            .json(&json!({"provider_id":"fake","model_id":"fake-model"}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();

        // open ws before sending message
        let ws_url = format!("ws://{}/api/sessions/{}/stream", addr, session.id.0);
        let (mut ws_stream, _) = tokio_tungstenite::connect_async(ws_url).await.unwrap();

        // post message
        let _msg: context_core::models::Message = client
            .post(format!("{base}/api/sessions/{}/messages", session.id.0))
            .json(&json!({"content":"hello"}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();

        // consume ws until Done
        let mut seen_done = false;
        while let Some(Ok(frame)) = ws_stream.next().await {
            if let tokio_tungstenite::tungstenite::Message::Text(txt) = frame {
                let ev: context_core::models::SessionEvent =
                    serde_json::from_str(&txt).unwrap();
                if matches!(ev.event_type, context_core::models::SessionEventType::Done) {
                    seen_done = true;
                    break;
                }
            }
        }
        assert!(seen_done);

        let events = store.list_session_events(session.id).await.unwrap();
        assert!(events
            .iter()
            .any(|e| matches!(e.event_type, context_core::models::SessionEventType::UserMessage)));

        server.abort();
    }
}
