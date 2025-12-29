use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::json;
use tokio::process::Command;
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};

use context_core::models::SessionEventType;
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
async fn workspace_catchup_snapshot_includes_tracks() {
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
    state.start_workspace_catchup_listener();
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

    let tracks = store.list_tracks_for_task(task_active.id).await.unwrap();
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

    let snapshot: context_core::models::WorkspaceCatchupSnapshot = client
        .get(format!("{base}/api/workspaces/{}/catchup?limit=5", ws.id.0))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(snapshot.active.tasks.len(), 1);
    let summary = &snapshot.active.tasks[0];
    assert_eq!(summary.task.id, task_active.id);
    assert_eq!(summary.tracks.len(), 1);
    assert_eq!(summary.tracks[0].sessions.len(), 1);
    assert_eq!(snapshot.active.total_count, 1);

    let snapshot_all: context_core::models::WorkspaceCatchupSnapshot = client
        .get(format!(
            "{base}/api/workspaces/{}/catchup?limit=10&include_archived=1",
            ws.id.0
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let archived = snapshot_all.archived.expect("archived page");
    assert_eq!(archived.tasks.len(), 1);
    assert_eq!(archived.tasks[0].task.id, task_archived.id);

    let snapshot_archived_only: context_core::models::WorkspaceCatchupSnapshot = client
        .get(format!(
            "{base}/api/workspaces/{}/catchup?limit=5&archived_only=1",
            ws.id.0
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(snapshot_archived_only.active.tasks.len(), 0);
    let archived_only = snapshot_archived_only.archived.expect("archived page");
    assert_eq!(archived_only.tasks.len(), 1);
    assert_eq!(archived_only.tasks[0].task.id, task_archived.id);

    server.abort();
    drop(state);
}

#[tokio::test]
async fn workspace_stream_replays_from_after_seq() {
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
    state.start_workspace_catchup_listener();
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

    let task: context_core::models::Task = client
        .post(format!("{base}/api/workspaces/{}/tasks", ws.id.0))
        .json(&json!({"title":"replay"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let tracks = store.list_tracks_for_task(task.id).await.unwrap();
    let track = &tracks[0];
    let session: context_core::models::Session = client
        .post(format!("{base}/api/tracks/{}/sessions", track.id.0))
        .json(&json!({"provider_id":"fake","model_id":"fake-model"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let ev1 = store
        .append_session_event(
            session.id,
            None,
            None,
            SessionEventType::Notice,
            json!({"msg":"one"}),
        )
        .await
        .unwrap();
    let ev2 = store
        .append_session_event(
            session.id,
            None,
            None,
            SessionEventType::Notice,
            json!({"msg":"two"}),
        )
        .await
        .unwrap();
    let ev3 = store
        .append_session_event(
            session.id,
            None,
            None,
            SessionEventType::Notice,
            json!({"msg":"three"}),
        )
        .await
        .unwrap();

    let ws_url = format!("ws://{}/api/workspaces/{}/stream", addr, ws.id.0);
    let (mut socket, _) = connect_async(&ws_url).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    let subscribe = json!({
        "type": "subscribe",
        "sessions": [{
            "session_id": session.id.0,
            "after_seq": ev2.seq,
        }],
    })
    .to_string();
    socket
        .send(WsMessage::Text(subscribe.into()))
        .await
        .unwrap();

    let mut seen_replay = false;
    let mut seen_old = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
    while tokio::time::Instant::now() < deadline {
        if let Some(Ok(WsMessage::Text(txt))) = socket.next().await {
            if let Ok(context_core::models::WorkspaceCatchupEvent::SessionHeadDelta {
                delta, ..
            }) = serde_json::from_str::<context_core::models::WorkspaceCatchupEvent>(&txt)
            {
                if delta.session_id != session.id {
                    continue;
                }
                if let Some(event) = delta.event {
                    if event.seq == ev3.seq {
                        seen_replay = true;
                    }
                    if event.seq <= ev2.seq {
                        seen_old = true;
                    }
                }
            }
        }
        if seen_replay {
            break;
        }
    }

    assert!(seen_replay, "expected replay of newest event");
    assert!(!seen_old, "did not expect events at/before after_seq");
    assert!(ev1.seq < ev2.seq && ev2.seq < ev3.seq);

    server.abort();
    drop(state);
}

#[tokio::test]
async fn workspace_stream_replays_tool_events() {
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
    state.start_workspace_catchup_listener();
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

    let task: context_core::models::Task = client
        .post(format!("{base}/api/workspaces/{}/tasks", ws.id.0))
        .json(&json!({"title":"tool-replay"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let tracks = store.list_tracks_for_task(task.id).await.unwrap();
    let track = &tracks[0];
    let session: context_core::models::Session = client
        .post(format!("{base}/api/tracks/{}/sessions", track.id.0))
        .json(&json!({"provider_id":"fake","model_id":"fake-model"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let ev1 = store
        .append_session_event(
            session.id,
            None,
            None,
            SessionEventType::Notice,
            json!({"msg":"seed"}),
        )
        .await
        .unwrap();
    let tool_call_id = "tool-1";
    let ev2 = store
        .append_session_event(
            session.id,
            None,
            None,
            SessionEventType::ToolCall,
            json!({"tool_call_id": tool_call_id, "name": "fake_tool", "args": {}}),
        )
        .await
        .unwrap();
    let ev3 = store
        .append_session_event(
            session.id,
            None,
            None,
            SessionEventType::ToolResult,
            json!({"tool_call_id": tool_call_id, "result": "ok"}),
        )
        .await
        .unwrap();

    let ws_url = format!("ws://{}/api/workspaces/{}/stream", addr, ws.id.0);
    let (mut socket, _) = connect_async(&ws_url).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    let subscribe = json!({
        "type": "subscribe",
        "sessions": [{
            "session_id": session.id.0,
            "after_seq": ev1.seq,
        }],
    })
    .to_string();
    socket
        .send(WsMessage::Text(subscribe.into()))
        .await
        .unwrap();

    let mut saw_call = false;
    let mut saw_result = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        let wait = remaining.min(Duration::from_millis(250));
        let next = tokio::time::timeout(wait, socket.next()).await;
        if let Ok(Some(Ok(WsMessage::Text(txt)))) = next {
            if let Ok(context_core::models::WorkspaceCatchupEvent::SessionHeadDelta {
                delta, ..
            }) = serde_json::from_str::<context_core::models::WorkspaceCatchupEvent>(&txt)
            {
                if delta.session_id != session.id {
                    continue;
                }
                if let Some(event) = delta.event {
                    match event.event_type {
                        SessionEventType::ToolCall => saw_call = true,
                        SessionEventType::ToolResult => saw_result = true,
                        _ => {}
                    }
                }
            }
        }
        if saw_call && saw_result {
            break;
        }
    }

    assert!(saw_call, "expected tool_call event replay");
    assert!(saw_result, "expected tool_result event replay");
    assert!(ev1.seq < ev2.seq && ev2.seq < ev3.seq);

    server.abort();
    drop(state);
}

#[tokio::test]
async fn workspace_stream_emits_gap_on_large_replay() {
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
    state.start_workspace_catchup_listener();
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

    let task: context_core::models::Task = client
        .post(format!("{base}/api/workspaces/{}/tasks", ws.id.0))
        .json(&json!({"title":"gap"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let tracks = store.list_tracks_for_task(task.id).await.unwrap();
    let track = &tracks[0];
    let session: context_core::models::Session = client
        .post(format!("{base}/api/tracks/{}/sessions", track.id.0))
        .json(&json!({"provider_id":"fake","model_id":"fake-model"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    for _ in 0..2105 {
        store
            .append_session_event(
                session.id,
                None,
                None,
                SessionEventType::Notice,
                json!({"msg":"spam"}),
            )
            .await
            .unwrap();
    }

    let ws_url = format!("ws://{}/api/workspaces/{}/stream", addr, ws.id.0);
    let (mut socket, _) = connect_async(&ws_url).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    let subscribe = json!({
        "type": "subscribe",
        "sessions": [{
            "session_id": session.id.0,
            "after_seq": 0,
        }],
    })
    .to_string();
    socket
        .send(WsMessage::Text(subscribe.into()))
        .await
        .unwrap();

    let mut seen_gap = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
    while tokio::time::Instant::now() < deadline {
        if let Some(Ok(WsMessage::Text(txt))) = socket.next().await {
            if let Ok(context_core::models::WorkspaceCatchupEvent::SessionGap {
                session_id, ..
            }) = serde_json::from_str::<context_core::models::WorkspaceCatchupEvent>(&txt)
            {
                if session_id == session.id {
                    seen_gap = true;
                    break;
                }
            }
        }
    }

    assert!(seen_gap, "expected gap event for large replay");

    server.abort();
    drop(state);
}

#[tokio::test]
async fn workspace_catchup_stream_pushes_updates() {
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
    state.start_workspace_catchup_listener();
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

    let ws_url = format!("ws://{}/api/workspaces/{}/stream", addr, ws.id.0);
    let (mut socket, _) = connect_async(&ws_url).await.unwrap();
    let ready_msg = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    if let WsMessage::Text(txt) = ready_msg {
        let evt: context_core::models::WorkspaceCatchupEvent = serde_json::from_str(&txt).unwrap();
        match evt {
            context_core::models::WorkspaceCatchupEvent::Ready { .. } => {}
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
        if let Some(Ok(WsMessage::Text(txt))) = socket.next().await {
            if let context_core::models::WorkspaceCatchupEvent::TaskUpsert {
                task: summary, ..
            } =
                serde_json::from_str::<context_core::models::WorkspaceCatchupEvent>(&txt).unwrap()
            {
                if summary.task.id == task.id {
                    saw_upsert = true;
                    break;
                }
            }
        }
    }
    assert!(saw_upsert);

    server.abort();
    drop(state);
}

#[tokio::test]
async fn workspace_catchup_stream_filters_session_head_deltas() {
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
    state.start_workspace_catchup_listener();
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

    let task: context_core::models::Task = client
        .post(format!("{base}/api/workspaces/{}/tasks", ws.id.0))
        .json(&json!({"title":"live"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let tracks = store.list_tracks_for_task(task.id).await.unwrap();
    let track = &tracks[0];
    let session_a: context_core::models::Session = client
        .post(format!("{base}/api/tracks/{}/sessions", track.id.0))
        .json(&json!({"provider_id":"fake","model_id":"fake-model"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let session_b: context_core::models::Session = client
        .post(format!("{base}/api/tracks/{}/sessions", track.id.0))
        .json(&json!({"provider_id":"fake","model_id":"fake-model"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let ws_url = format!("ws://{}/api/workspaces/{}/stream", addr, ws.id.0);
    let (mut socket, _) = connect_async(&ws_url).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    let subscribe = json!({
        "type": "subscribe",
        "sessions": [{
            "session_id": session_a.id.0,
            "after_seq": 0,
        }],
    })
    .to_string();
    socket
        .send(WsMessage::Text(subscribe.into()))
        .await
        .unwrap();

    client
        .post(format!("{base}/api/sessions/{}/messages", session_a.id.0))
        .json(&json!({"content":"hello a"}))
        .send()
        .await
        .unwrap();
    client
        .post(format!("{base}/api/sessions/{}/messages", session_b.id.0))
        .json(&json!({"content":"hello b"}))
        .send()
        .await
        .unwrap();

    let mut seen_a = false;
    let mut seen_b = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
    while tokio::time::Instant::now() < deadline {
        if let Some(Ok(WsMessage::Text(txt))) = socket.next().await {
            if let Ok(context_core::models::WorkspaceCatchupEvent::SessionHeadDelta {
                delta, ..
            }) = serde_json::from_str::<context_core::models::WorkspaceCatchupEvent>(&txt)
            {
                if delta.session_id == session_a.id {
                    seen_a = true;
                }
                if delta.session_id == session_b.id {
                    seen_b = true;
                }
            }
        }
        if seen_a {
            break;
        }
    }

    assert!(seen_a, "expected delta for subscribed session");
    assert!(!seen_b, "did not expect delta for unsubscribed session");

    server.abort();
    drop(state);
}
