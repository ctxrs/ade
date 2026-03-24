#![cfg(feature = "fault_injection")]

use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::json;
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};

use ctx_core::models::{
    SessionEventType, SessionHeadSnapshot, WorkspaceActiveHeadBatch, WorkspaceActiveSnapshot,
    WorkspaceActiveSnapshotClientMessage, WorkspaceActiveSnapshotEvent,
    WorkspaceActiveSnapshotStreamMessage,
};

mod common;

static FAILPOINT_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn assert_hot_endpoints_with_failpoints(failpoints: &[&'static str]) {
    let _guard = FAILPOINT_LOCK.lock().await;
    ctx_store::fault_injection::clear_failpoints();
    let repo = common::init_git_repo(&[("file.txt", "hello\n")]).await;
    let data_dir = tempfile::tempdir().unwrap();
    let stores = common::setup_store(data_dir.path()).await;

    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        common::fake_providers(),
        "http://127.0.0.1:0",
    );
    let app = common::router(state.clone());
    let server = common::spawn_http_server(app).await;
    let base = &server.base_url;
    let client = &server.client;

    let ws: ctx_core::models::Workspace = client
        .post(format!("{base}/api/workspaces"))
        .json(&json!({"root_path": repo.path(), "name": "ws"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let task: ctx_core::models::Task = client
        .post(format!("{base}/api/workspaces/{}/tasks", ws.id.0))
        .json(&json!({"title":"hot-endpoints"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let session: ctx_core::models::Session = client
        .post(format!("{base}/api/tasks/{}/sessions", task.id.0))
        .json(&json!({"provider_id":"fake","model_id":"fake-model"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let store = state.store_for_session(session.id).await.unwrap();
    let _ = store
        .append_session_event(
            session.id,
            None,
            None,
            SessionEventType::Notice,
            json!({"msg":"warm"}),
        )
        .await
        .unwrap();
    let _ = store
        .refresh_active_session_head_projection(session.id)
        .await;

    state.emit_workspace_task_upsert(task.id).await.unwrap();
    state.refresh_session_head_cache(session.id).await;

    let head_snapshot = store
        .get_session_head_snapshot(session.id, 10, true)
        .await
        .unwrap()
        .unwrap();
    state
        .cache_session_head_snapshot(session.id, 10, true, head_snapshot)
        .await;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    let mut cached_snapshot = state
        .workspaces
        .workspace_active_snapshot
        .active_snapshot(ws.id, 50)
        .await;
    while cached_snapshot.active.tasks.is_empty() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
        cached_snapshot = state
            .workspaces
            .workspace_active_snapshot
            .active_snapshot(ws.id, 50)
            .await;
    }
    assert!(
        !cached_snapshot.active.tasks.is_empty(),
        "expected active snapshot to be cached"
    );
    state
        .cache_workspace_active_snapshot(cached_snapshot.clone())
        .await;

    let mut cached_heads = state
        .workspaces
        .workspace_active_snapshot
        .active_heads(ws.id)
        .await;
    while cached_heads.heads.is_empty() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
        cached_heads = state
            .workspaces
            .workspace_active_snapshot
            .active_heads(ws.id)
            .await;
    }
    assert!(
        !cached_heads.heads.is_empty(),
        "expected active heads to be cached"
    );
    state
        .cache_workspace_active_heads(cached_heads.clone())
        .await;
    state
        .ensure_workspace_active_snapshot_hydrated(ws.id)
        .await
        .unwrap();

    ctx_store::fault_injection::clear_failpoints();
    for point in failpoints {
        ctx_store::fault_injection::set_failpoint(point, 10);
    }

    let snapshot: WorkspaceActiveSnapshot = client
        .get(format!(
            "{base}/api/workspaces/{}/active_snapshot?limit=5",
            ws.id.0
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(snapshot.workspace_id, ws.id);
    assert_eq!(snapshot.active.tasks.len(), 1);

    let heads: WorkspaceActiveHeadBatch = client
        .get(format!("{base}/api/workspaces/{}/active_heads", ws.id.0))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(heads.workspace_id, ws.id);
    assert_eq!(heads.heads.len(), 1);

    let head_response = client
        .get(format!(
            "{base}/api/sessions/{}/head?limit=10&include_events=true",
            session.id.0
        ))
        .send()
        .await
        .unwrap();
    if failpoints.contains(&"ctx_store.get_session_head_snapshot") {
        assert_eq!(
            head_response.status(),
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            "include_events=true session detail reads should fail closed when the full store head path is unavailable"
        );
    } else {
        let head: SessionHeadSnapshot = head_response.json().await.unwrap();
        assert_eq!(head.session.id, session.id);
    }

    let ws_url = format!(
        "ws://{}/api/workspaces/{}/stream",
        base.trim_start_matches("http://"),
        ws.id.0
    );
    let (mut socket, _) = connect_async(&ws_url).await.unwrap();
    let msg = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let WsMessage::Text(txt) = msg else {
        panic!("expected text frame, got {:?}", msg);
    };
    let ready: WorkspaceActiveSnapshotStreamMessage = serde_json::from_str(&txt).unwrap();
    assert!(matches!(
        ready,
        WorkspaceActiveSnapshotStreamMessage::Event { ref event, .. }
            if matches!(event.as_ref(), WorkspaceActiveSnapshotEvent::Ready { .. })
    ));

    let subscribe = WorkspaceActiveSnapshotClientMessage::Subscribe {
        session_ids: vec![session.id],
        sessions: Vec::new(),
        task_ids: Vec::new(),
        foreground_task_id: None,
        scope: None,
        include_active_heads: true,
    };
    socket
        .send(WsMessage::Text(
            serde_json::to_string(&subscribe).unwrap().into(),
        ))
        .await
        .unwrap();

    let msg = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let WsMessage::Text(txt) = msg else {
        panic!("expected text frame, got {:?}", msg);
    };
    let message: WorkspaceActiveSnapshotStreamMessage = serde_json::from_str(&txt).unwrap();
    match message {
        WorkspaceActiveSnapshotStreamMessage::Snapshot {
            active_snapshot,
            active_heads,
            ..
        } => {
            assert_eq!(active_snapshot.workspace_id, ws.id);
            assert_eq!(active_snapshot.active.tasks.len(), 1);
            let Some(active_heads) = active_heads else {
                panic!("expected active heads in snapshot payload");
            };
            assert_eq!(active_heads.workspace_id, ws.id);
            assert_eq!(active_heads.heads.len(), 1);
        }
        other => panic!("expected snapshot payload, got {other:?}"),
    }

    ctx_store::fault_injection::clear_failpoints();
}

#[tokio::test]
async fn hot_endpoints_use_cache_when_db_unavailable() {
    assert_hot_endpoints_with_failpoints(&[
        "ctx_store.get_workspace_active_snapshot_state",
        "ctx_store.list_workspace_active_head_snapshots",
        "ctx_store.get_session_head_snapshot",
    ])
    .await;
}

#[tokio::test]
async fn hot_endpoints_snapshot_cache_handles_active_snapshot_failpoints() {
    assert_hot_endpoints_with_failpoints(&["ctx_store.get_workspace_active_snapshot_state"]).await;
}

#[tokio::test]
async fn hot_endpoints_heads_cache_handles_head_failpoints() {
    assert_hot_endpoints_with_failpoints(&[
        "ctx_store.list_workspace_active_head_snapshots",
        "ctx_store.get_session_head_snapshot",
    ])
    .await;
}

#[tokio::test]
async fn cold_workspace_active_endpoints_fail_closed_when_hydration_fails() {
    let _guard = FAILPOINT_LOCK.lock().await;
    ctx_store::fault_injection::clear_failpoints();
    let repo = common::init_git_repo(&[("file.txt", "hello\n")]).await;
    let data_dir = tempfile::tempdir().unwrap();
    let stores = common::setup_store(data_dir.path()).await;

    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        common::fake_providers(),
        "http://127.0.0.1:0",
    );
    let app = common::router(state.clone());
    let server = common::spawn_http_server(app).await;
    let base = &server.base_url;
    let client = &server.client;

    let ws: ctx_core::models::Workspace = client
        .post(format!("{base}/api/workspaces"))
        .json(&json!({"root_path": repo.path(), "name": "ws"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let task: ctx_core::models::Task = client
        .post(format!("{base}/api/workspaces/{}/tasks", ws.id.0))
        .json(&json!({"title":"cold-hydration"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let _session: ctx_core::models::Session = client
        .post(format!("{base}/api/tasks/{}/sessions", task.id.0))
        .json(&json!({"provider_id":"fake","model_id":"fake-model"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    ctx_store::fault_injection::set_failpoint("ctx_store.list_workspace_active_head_snapshots", 2);

    let snapshot = client
        .get(format!("{base}/api/workspaces/{}/active_snapshot", ws.id.0))
        .send()
        .await
        .unwrap();
    assert_eq!(
        snapshot.status(),
        reqwest::StatusCode::INTERNAL_SERVER_ERROR
    );

    let heads = client
        .get(format!("{base}/api/workspaces/{}/active_heads", ws.id.0))
        .send()
        .await
        .unwrap();
    assert_eq!(heads.status(), reqwest::StatusCode::INTERNAL_SERVER_ERROR);

    ctx_store::fault_injection::clear_failpoints();
}

#[tokio::test]
async fn publish_event_does_not_trigger_full_session_head_rebuilds() {
    let _guard = FAILPOINT_LOCK.lock().await;
    ctx_store::fault_injection::clear_failpoints();
    let repo = common::init_git_repo(&[("file.txt", "hello\n")]).await;
    let data_dir = tempfile::tempdir().unwrap();
    let stores = common::setup_store(data_dir.path()).await;

    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        common::fake_providers(),
        "http://127.0.0.1:0",
    );
    let app = common::router(state.clone());
    let server = common::spawn_http_server(app).await;
    let base = &server.base_url;
    let client = &server.client;

    let ws: ctx_core::models::Workspace = client
        .post(format!("{base}/api/workspaces"))
        .json(&json!({"root_path": repo.path(), "name": "ws"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let task: ctx_core::models::Task = client
        .post(format!("{base}/api/workspaces/{}/tasks", ws.id.0))
        .json(&json!({"title":"hot-loop"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let session: ctx_core::models::Session = client
        .post(format!("{base}/api/tasks/{}/sessions", task.id.0))
        .json(&json!({"provider_id":"fake","model_id":"fake-model"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    state
        .ensure_workspace_active_snapshot_hydrated(ws.id)
        .await
        .unwrap();

    let store = state.store_for_session(session.id).await.unwrap();
    let event = store
        .append_session_event(
            session.id,
            None,
            None,
            SessionEventType::Notice,
            json!({"msg":"delta only"}),
        )
        .await
        .unwrap();

    ctx_store::fault_injection::clear_failpoints();
    ctx_store::fault_injection::set_failpoint("ctx_store.get_session_head_snapshot", 1);

    state.publish_event(event.clone()).await;
    tokio::time::sleep(Duration::from_millis(400)).await;

    let active_heads = state
        .workspaces
        .workspace_active_snapshot
        .active_heads(ws.id)
        .await;
    assert_eq!(active_heads.heads.len(), 1);
    assert_eq!(active_heads.heads[0].last_event_seq, event.seq);

    let manual_refresh = store.get_session_head_snapshot(session.id, 10, true).await;
    assert!(
        manual_refresh.is_err(),
        "manual session head fetch should consume the one-shot failpoint because publish_event no longer rebuilds full heads"
    );

    ctx_store::fault_injection::clear_failpoints();
}
