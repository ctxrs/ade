#![cfg(feature = "fault_injection")]

use std::time::Duration;

use futures::StreamExt;
use serde_json::json;
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};

use ctx_core::models::{
    SessionEventType, SessionHeadSnapshot, WorkspaceActiveHeadBatch, WorkspaceActiveSnapshot,
    WorkspaceActiveSnapshotEvent,
};

mod common;

#[tokio::test]
async fn hot_endpoints_use_cache_when_db_unavailable() {
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

    let _: WorkspaceActiveSnapshot = client
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
    let _: WorkspaceActiveHeadBatch = client
        .get(format!("{base}/api/workspaces/{}/active_heads", ws.id.0))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let _: SessionHeadSnapshot = client
        .get(format!(
            "{base}/api/sessions/{}/head?limit=10&include_events=true",
            session.id.0
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    ctx_store::fault_injection::clear_failpoints();
    ctx_store::fault_injection::set_failpoint("ctx_store.get_workspace_active_snapshot_state", 10);
    ctx_store::fault_injection::set_failpoint(
        "ctx_store.list_workspace_active_page_read_model",
        10,
    );
    ctx_store::fault_injection::set_failpoint("ctx_store.list_workspace_active_head_snapshots", 10);
    ctx_store::fault_injection::set_failpoint("ctx_store.get_session_head_snapshot", 10);

    let snapshot = client
        .get(format!(
            "{base}/api/workspaces/{}/active_snapshot?limit=5",
            ws.id.0
        ))
        .send()
        .await
        .unwrap();
    assert!(snapshot.status().is_success());

    let heads = client
        .get(format!("{base}/api/workspaces/{}/active_heads", ws.id.0))
        .send()
        .await
        .unwrap();
    assert!(heads.status().is_success());

    let head = client
        .get(format!(
            "{base}/api/sessions/{}/head?limit=10&include_events=true",
            session.id.0
        ))
        .send()
        .await
        .unwrap();
    assert!(head.status().is_success());

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
    let event: WorkspaceActiveSnapshotEvent = serde_json::from_str(&txt).unwrap();
    assert!(matches!(event, WorkspaceActiveSnapshotEvent::Ready { .. }));
}
