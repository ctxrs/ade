use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use serde_json::json;
use tokio::process::Command;
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};

use context_http::api;
use context_http::daemon::AppState;
use context_providers::fake::FakeProviderAdapter;
use context_store::Store;

async fn setup_git_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["init"])
        .output()
        .await
        .unwrap();
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["config", "user.email", "test@example.com"])
        .output()
        .await
        .unwrap();
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["config", "user.name", "Test"])
        .output()
        .await
        .unwrap();
    tokio::fs::write(root.join("file.txt"), "hello\n")
        .await
        .unwrap();
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["add", "."])
        .output()
        .await
        .unwrap();
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["commit", "-m", "init"])
        .output()
        .await
        .unwrap();
    dir
}

#[tokio::test]
async fn workspace_index_snapshot_includes_tracks() {
    let repo = setup_git_repo().await;
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
        "http://127.0.0.1:0".to_string(),
        None,
    ));
    state.start_workspace_index_listener();
    let app = api::router(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base = format!("http://{}", addr);
    let client = reqwest::Client::new();

    let ws: context_core::models::Workspace = client
        .post(format!("{base}/api/workspaces"))
        .json(&json!({"root_path": repo.path(), "name": "ws"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let task_active: context_core::models::Task = client
        .post(format!("{base}/api/workspaces/{}/tasks", ws.id.0))
        .json(&json!({"title":"active"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let tracks: Vec<context_core::models::Track> = client
        .get(format!("{base}/api/tasks/{}/tracks", task_active.id.0))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let track = &tracks[0];
    client
        .post(format!("{base}/api/tracks/{}/sessions", track.id.0))
        .json(&json!({"provider_id":"fake","model_id":"fake-model"}))
        .send()
        .await
        .unwrap();

    let task_archived: context_core::models::Task = client
        .post(format!("{base}/api/workspaces/{}/tasks", ws.id.0))
        .json(&json!({"title":"archived"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    client
        .post(format!("{base}/api/tasks/{}/archive", task_archived.id.0))
        .send()
        .await
        .unwrap();

    let page: context_core::models::WorkspaceIndexPage = client
        .get(format!("{base}/api/workspaces/{}/index?limit=5", ws.id.0))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(page.tasks.len(), 1);
    let summary = &page.tasks[0];
    assert_eq!(summary.task.id, task_active.id);
    assert_eq!(summary.tracks.len(), 1);
    assert_eq!(summary.tracks[0].sessions.len(), 1);
    assert!(summary.provider_ids.contains(&"fake".to_string()));
    assert_eq!(page.total_active, 1);
    assert_eq!(page.total_archived, 1);

    let page_all: context_core::models::WorkspaceIndexPage = client
        .get(format!(
            "{base}/api/workspaces/{}/index?limit=10&include_archived=1",
            ws.id.0
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(page_all.tasks.len(), 2);

    server.abort();
    drop(state);
}

#[tokio::test]
async fn workspace_index_stream_pushes_updates() {
    let repo = setup_git_repo().await;
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
        "http://127.0.0.1:0".to_string(),
        None,
    ));
    state.start_workspace_index_listener();
    let app = api::router(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base = format!("http://{}", addr);
    let client = reqwest::Client::new();

    let ws: context_core::models::Workspace = client
        .post(format!("{base}/api/workspaces"))
        .json(&json!({"root_path": repo.path(), "name": "ws"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let ws_url = format!("ws://{}/api/workspaces/{}/index/stream", addr, ws.id.0);
    let (mut socket, _) = connect_async(&ws_url).await.unwrap();
    let ready_msg = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    if let WsMessage::Text(txt) = ready_msg {
        let evt: context_core::models::WorkspaceIndexEvent = serde_json::from_str(&txt).unwrap();
        match evt {
            context_core::models::WorkspaceIndexEvent::Ready { .. } => {}
            other => panic!("expected ready, got {other:?}"),
        }
    } else {
        panic!("expected ready text frame");
    }

    let task: context_core::models::Task = client
        .post(format!("{base}/api/workspaces/{}/tasks", ws.id.0))
        .json(&json!({"title":"live"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let mut saw_upsert = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        if let Some(Ok(frame)) = socket.next().await {
            if let WsMessage::Text(txt) = frame {
                let evt: context_core::models::WorkspaceIndexEvent =
                    serde_json::from_str(&txt).unwrap();
                if let context_core::models::WorkspaceIndexEvent::TaskUpsert {
                    task: summary, ..
                } = evt
                {
                    if summary.task.id == task.id {
                        saw_upsert = true;
                        break;
                    }
                }
            }
        }
    }
    assert!(saw_upsert);

    client
        .delete(format!("{base}/api/tasks/{}", task.id.0))
        .send()
        .await
        .unwrap();

    let mut saw_delete = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        if let Some(Ok(frame)) = socket.next().await {
            if let WsMessage::Text(txt) = frame {
                let evt: context_core::models::WorkspaceIndexEvent =
                    serde_json::from_str(&txt).unwrap();
                if let context_core::models::WorkspaceIndexEvent::TaskDelete { task_id, .. } = evt {
                    if task_id == task.id {
                        saw_delete = true;
                        break;
                    }
                }
            }
        }
    }
    assert!(saw_delete);

    server.abort();
    drop(state);
}
