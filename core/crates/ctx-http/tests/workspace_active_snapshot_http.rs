use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use reqwest::Url;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio::sync::Semaphore;
use tokio_tungstenite::{
    connect_async, tungstenite::Message as WsMessage, MaybeTlsStream, WebSocketStream,
};

use chrono::Utc;
use ctx_core::ids::{MobileDeviceId, TurnId, WorktreeId};
use ctx_core::models::{
    SessionEventType, SessionTurn, SessionTurnStatus, WorkspaceActiveSnapshotEvent,
    WorkspaceActiveSnapshotStreamMessage, WorktreeVcsFreshness, WorktreeVcsSnapshot,
};
use ctx_http::daemon::AppState;
use ctx_store::store::{MobileAccessConfig, MobileDeviceUpsert};
use ctx_transport_runtime::mobile_e2ee;

mod common;

type TestWsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

#[derive(Debug, Deserialize, Serialize)]
struct TestMobileSecureEnvelope {
    device_id: String,
    seq: i64,
    nonce: String,
    ciphertext: String,
}

fn workspace_http_test_gate() -> &'static Arc<Semaphore> {
    static GATE: OnceLock<Arc<Semaphore>> = OnceLock::new();
    GATE.get_or_init(|| Arc::new(Semaphore::new(4)))
}

fn worktree_vcs_snapshot_from_message(
    message: WorkspaceActiveSnapshotStreamMessage,
    worktree_id: WorktreeId,
) -> Option<WorktreeVcsSnapshot> {
    match message {
        WorkspaceActiveSnapshotStreamMessage::Snapshot {
            active_snapshot, ..
        } => active_snapshot
            .worktree_vcs_snapshots
            .into_iter()
            .find(|snapshot| snapshot.worktree_id == worktree_id),
        WorkspaceActiveSnapshotStreamMessage::Event { event, .. } => {
            let WorkspaceActiveSnapshotEvent::WorktreeVcsSnapshot { snapshot, .. } = event.as_ref()
            else {
                return None;
            };
            if snapshot.worktree_id != worktree_id {
                None
            } else {
                Some((**snapshot).clone())
            }
        }
        _ => None,
    }
}

fn git_status_untracked_from_message(
    message: ctx_core::models::WorkspaceActiveSnapshotStreamMessage,
    worktree_id: WorktreeId,
) -> Option<i64> {
    worktree_vcs_snapshot_from_message(message, worktree_id)
        .map(|snapshot| snapshot.git_status.untracked)
}

async fn remove_git_marker(root: &Path) {
    let git_path = root.join(".git");
    let Ok(metadata) = tokio::fs::metadata(&git_path).await else {
        return;
    };
    if metadata.is_dir() {
        tokio::fs::remove_dir_all(&git_path)
            .await
            .expect("remove .git directory");
    } else {
        tokio::fs::remove_file(&git_path)
            .await
            .expect("remove .git file");
    }
}

async fn setup_with_root(
    repo: tempfile::TempDir,
) -> (
    tempfile::TempDir,
    tempfile::TempDir,
    Arc<AppState>,
    common::TestServer,
) {
    let permit = workspace_http_test_gate()
        .clone()
        .acquire_owned()
        .await
        .unwrap();
    let data_dir = tempfile::tempdir().unwrap();
    let stores = common::setup_store(data_dir.path()).await;

    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        common::fake_providers(),
        "http://127.0.0.1:0",
    );
    let app = common::router(state.clone());
    let server = common::spawn_http_server(app)
        .await
        .with_resource_permit(permit);

    (repo, data_dir, state, server)
}

async fn setup() -> (
    tempfile::TempDir,
    tempfile::TempDir,
    Arc<AppState>,
    common::TestServer,
) {
    setup_with_root(common::init_git_repo(&[("file.txt", "hello\n")]).await).await
}

async fn setup_git() -> (
    tempfile::TempDir,
    tempfile::TempDir,
    Arc<AppState>,
    common::TestServer,
) {
    setup().await
}

#[tokio::test]
async fn workspace_active_hydration_returns_500_for_store_open_failures_and_404_for_missing_workspaces(
) {
    let (repo, data_dir, state, server) = setup().await;
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

    let missing = client
        .get(format!(
            "{base}/api/workspaces/{}/active_snapshot",
            uuid::Uuid::new_v4()
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), reqwest::StatusCode::NOT_FOUND);

    let blocked_workspace_store_dir = data_dir
        .path()
        .join("db")
        .join("workspaces")
        .join(ws.id.0.to_string());
    state.core.stores.evict_workspace(ws.id).await;
    if let Ok(metadata) = tokio::fs::metadata(&blocked_workspace_store_dir).await {
        if metadata.is_dir() {
            tokio::fs::remove_dir_all(&blocked_workspace_store_dir)
                .await
                .unwrap();
        } else {
            tokio::fs::remove_file(&blocked_workspace_store_dir)
                .await
                .unwrap();
        }
    }
    tokio::fs::create_dir_all(blocked_workspace_store_dir.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(&blocked_workspace_store_dir, b"blocked workspace store")
        .await
        .unwrap();

    let broken_snapshot = client
        .get(format!("{base}/api/workspaces/{}/active_snapshot", ws.id.0))
        .send()
        .await
        .unwrap();
    assert_eq!(
        broken_snapshot.status(),
        reqwest::StatusCode::INTERNAL_SERVER_ERROR
    );

    let broken_heads = client
        .get(format!("{base}/api/workspaces/{}/active_heads", ws.id.0))
        .send()
        .await
        .unwrap();
    assert_eq!(
        broken_heads.status(),
        reqwest::StatusCode::INTERNAL_SERVER_ERROR
    );
}

async fn decode_json_response<T: DeserializeOwned>(response: reqwest::Response) -> T {
    let status = response.status();
    let body = response.text().await.unwrap();
    serde_json::from_str(&body).unwrap_or_else(|err| {
        panic!("failed to decode JSON response (status {status}): {err}\nbody: {body}")
    })
}

async fn configure_mobile_secure_access(state: &Arc<AppState>) -> (String, mobile_e2ee::E2eeKey) {
    let profile_id = state
        .global_store()
        .create_mobile_connection_profile(
            "workspace-http-test".to_string(),
            "https://example.test".to_string(),
            "test-token-hash".to_string(),
            "testtok".to_string(),
            vec![
                "device_registration".to_string(),
                "workspace_read".to_string(),
                "workspace_stream".to_string(),
            ],
        )
        .await
        .unwrap()
        .id;
    let (daemon_public_key, daemon_private_key) = mobile_e2ee::generate_keypair();
    state
        .global_store()
        .upsert_mobile_access_config(MobileAccessConfig {
            id: "default".to_string(),
            profile_id,
            tunnel_id: uuid::Uuid::new_v4().to_string(),
            public_base_url: "http://127.0.0.1".to_string(),
            relay_base_url: "http://127.0.0.1".to_string(),
            tunnel_secret: "test-secret".to_string(),
            daemon_public_key: daemon_public_key.clone(),
            daemon_private_key,
            enabled: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();

    let device_id = uuid::Uuid::new_v4();
    let (device_public_key, device_secret_key) = mobile_e2ee::generate_keypair();
    state
        .global_store()
        .upsert_mobile_device(
            MobileDeviceId(device_id),
            profile_id,
            MobileDeviceUpsert {
                device_label: Some("workspace-http-test".to_string()),
                platform: Some("test".to_string()),
                push_token: None,
                push_provider: None,
                public_key: Some(device_public_key),
                app_version: Some("0.0.0-test".to_string()),
            },
        )
        .await
        .unwrap();

    let key = mobile_e2ee::derive_client_key(
        &device_id.to_string(),
        &device_secret_key,
        &daemon_public_key,
    )
    .unwrap();
    (device_id.to_string(), key)
}

fn build_mobile_secure_ws_url(
    base: &str,
    workspace_id: ctx_core::ids::WorkspaceId,
    device_id: &str,
    key: &mobile_e2ee::E2eeKey,
) -> String {
    let mut url = Url::parse(base).expect("parse base url");
    match url.scheme() {
        "http" => {
            url.set_scheme("ws").expect("set ws scheme");
        }
        "https" => {
            url.set_scheme("wss").expect("set wss scheme");
        }
        other => panic!("unsupported base scheme {other}"),
    }
    url.set_path(&format!(
        "/api/mobile/secure/workspaces/{}/stream",
        workspace_id.0
    ));
    let token = mobile_e2ee::derive_stream_token(key, &workspace_id.0.to_string());
    url.set_query(Some(&format!("device_id={device_id}&token={token}")));
    url.to_string()
}

async fn recv_secure_workspace_message(
    socket: &mut TestWsStream,
    key: &mobile_e2ee::E2eeKey,
    device_id: &str,
    timeout: Duration,
) -> Option<WorkspaceActiveSnapshotStreamMessage> {
    let next = tokio::time::timeout(timeout, socket.next())
        .await
        .ok()??
        .ok()?;
    let text = match next {
        WsMessage::Text(text) => text.to_string(),
        _ => return None,
    };
    let frame: TestMobileSecureEnvelope = serde_json::from_str(&text).ok()?;
    let payload =
        mobile_e2ee::decrypt(key, device_id, frame.seq, &frame.nonce, &frame.ciphertext).ok()?;
    serde_json::from_slice(&payload).ok()
}

async fn send_secure_workspace_subscribe(
    socket: &mut TestWsStream,
    key: &mobile_e2ee::E2eeKey,
    device_id: &str,
    seq: i64,
    payload: Value,
) {
    let plaintext = serde_json::to_vec(&payload).expect("serialize secure subscribe payload");
    let envelope =
        mobile_e2ee::encrypt(key, device_id, seq, &plaintext).expect("encrypt secure subscribe");
    let frame = TestMobileSecureEnvelope {
        device_id: envelope.device_id,
        seq: envelope.seq,
        nonce: envelope.nonce_b64,
        ciphertext: envelope.ciphertext_b64,
    };
    socket
        .send(WsMessage::Text(
            serde_json::to_string(&frame)
                .expect("serialize secure subscribe envelope")
                .into(),
        ))
        .await
        .expect("send secure subscribe");
}

async fn insert_worktree(
    state: &Arc<AppState>,
    workspace_id: ctx_core::ids::WorkspaceId,
    _task_id: ctx_core::ids::TaskId,
    root_path: &Path,
) -> ctx_core::models::Worktree {
    let worktree = ctx_core::models::Worktree {
        id: WorktreeId::new(),
        workspace_id,
        root_path: root_path.to_string_lossy().to_string(),
        base_commit_sha: "test-base".to_string(),
        git_branch: None,
        vcs_kind: None,
        base_revision: None,
        vcs_ref: None,
        created_at: Utc::now(),
        bootstrap_status: None,
        bootstrap_started_at: None,
        bootstrap_finished_at: None,
        bootstrap_exit_code: None,
        bootstrap_timeout_sec: None,
        bootstrap_error: None,
        bootstrap_log_path: None,
        bootstrap_log_truncated: None,
        bootstrap_command: None,
        bootstrap_script_path: None,
    };

    let store = state.store_for_workspace(workspace_id).await.unwrap();
    store.insert_worktree(worktree.clone()).await.unwrap();
    state
        .global_store()
        .upsert_workspace_worktree_index(worktree.id, workspace_id)
        .await
        .unwrap();
    worktree
}

async fn create_task_with_primary_worktree(
    client: &reqwest::Client,
    state: &Arc<AppState>,
    base: &str,
    workspace_id: ctx_core::ids::WorkspaceId,
    root_path: &Path,
    title: &str,
) -> ctx_core::models::Task {
    let task_id = ctx_core::ids::TaskId::new();
    let worktree = insert_worktree(state, workspace_id, task_id, root_path).await;
    let response = client
        .post(format!("{base}/api/workspaces/{}/tasks", workspace_id.0))
        .json(&json!({
            "id": task_id.0.to_string(),
            "title": title,
            "default_session": {
                "provider_id": "fake",
                "model_id": "fake-model",
                "worktree_id": worktree.id.0.to_string(),
            },
        }))
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "create_task_with_primary_worktree failed for title {:?}: status {}",
        title,
        response.status()
    );
    let task: ctx_core::models::Task = decode_json_response(response).await;
    assert_eq!(task.primary_worktree_id, Some(worktree.id));
    assert!(task.primary_session_id.is_some());
    task
}

async fn create_session_with_request(
    client: &reqwest::Client,
    base: &str,
    task_id: ctx_core::ids::TaskId,
    request: Value,
) -> ctx_core::models::Session {
    let request = match request {
        Value::Object(map) => map,
        _ => panic!("session request must be a JSON object"),
    };
    assert!(
        request.contains_key("parent_session_id") && request.contains_key("relationship"),
        "test session creation through /api/tasks/:id/sessions must be an explicit child session"
    );

    let response = client
        .post(format!("{base}/api/tasks/{}/sessions", task_id.0))
        .json(&Value::Object(request))
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "create_session_with_request failed for task {}: status {}",
        task_id.0,
        response.status()
    );
    decode_json_response(response).await
}

async fn create_primary_worktree_session(
    client: &reqwest::Client,
    base: &str,
    task_id: ctx_core::ids::TaskId,
) -> ctx_core::models::Session {
    let response = client
        .get(format!("{base}/api/tasks/{}/sessions", task_id.0))
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "list sessions failed for task {}: status {}",
        task_id.0,
        response.status()
    );
    let sessions: Vec<ctx_core::models::Session> = decode_json_response(response).await;
    sessions
        .into_iter()
        .find(|session| session.parent_session_id.is_none() && session.relationship.is_none())
        .expect("task should have a primary session")
}

async fn create_child_worktree_session_with_request(
    client: &reqwest::Client,
    base: &str,
    task_id: ctx_core::ids::TaskId,
    request: Value,
) -> ctx_core::models::Session {
    create_session_with_request(client, base, task_id, request).await
}

#[tokio::test]
async fn workspace_active_snapshot_includes_sessions() {
    let (repo, _data_dir, state, server) = setup().await;
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

    let task_active =
        create_task_with_primary_worktree(client, &state, base, ws.id, repo.path(), "active").await;

    let session = create_primary_worktree_session(client, base, task_active.id).await;

    let snapshot: ctx_core::models::WorkspaceActiveSnapshot = client
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
    assert_eq!(snapshot.active.tasks.len(), 1);
    let summary = &snapshot.active.tasks[0];
    assert_eq!(summary.task.id, task_active.id);
    assert_eq!(summary.primary_session.session.id, session.id);
    assert!(summary.primary_session_head.is_none());
    assert_eq!(snapshot.active.total_count, 1);
}

#[tokio::test]
async fn create_session_rejects_initial_prompt_without_client_ids() {
    let (repo, _data_dir, state, server) = setup().await;
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

    let task_active =
        create_task_with_primary_worktree(client, &state, base, ws.id, repo.path(), "active").await;
    let primary_session = create_primary_worktree_session(client, base, task_active.id).await;

    let resp = client
        .post(format!("{base}/api/tasks/{}/sessions", task_active.id.0))
        .json(&json!({
            "provider_id": "fake",
            "model_id": "fake-model",
            "parent_session_id": primary_session.id.0.to_string(),
            "relationship": "sub_agent",
            "initial_prompt": "hello"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);

    let summary: Value = client
        .get(format!(
            "{base}/api/telemetry/summary?metric=compat.payload_reject_count"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let hit = summary
        .get("metrics")
        .and_then(Value::as_array)
        .map(|metrics| {
            metrics.iter().any(|metric| {
                let labels = metric.get("labels").and_then(Value::as_object);
                metric.get("name").and_then(Value::as_str) == Some("compat.payload_reject_count")
                    && labels
                        .and_then(|obj| obj.get("surface"))
                        .and_then(Value::as_str)
                        == Some("tasks.create_session")
                    && labels
                        .and_then(|obj| obj.get("issue"))
                        .and_then(Value::as_str)
                        == Some("missing_initial_ids")
                    && metric.get("sum").and_then(Value::as_f64).unwrap_or(0.0) >= 1.0
            })
        })
        .unwrap_or(false);
    assert!(
        hit,
        "expected compat.payload_reject_count telemetry for missing initial IDs"
    );
}

#[tokio::test]
async fn workspace_active_snapshot_includes_worktree_vcs_for_active_tasks_only() {
    let (repo, _data_dir, state, server) = setup_git().await;
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

    let task_active =
        create_task_with_primary_worktree(client, &state, base, ws.id, repo.path(), "active-task")
            .await;

    let session_active = create_primary_worktree_session(client, base, task_active.id).await;

    let task_archived = create_task_with_primary_worktree(
        client,
        &state,
        base,
        ws.id,
        repo.path(),
        "archived-task",
    )
    .await;

    let session_archived = create_primary_worktree_session(client, base, task_archived.id).await;

    let store_active = state.store_for_session(session_active.id).await.unwrap();
    let worktree_active = store_active
        .get_worktree(session_active.worktree_id)
        .await
        .unwrap()
        .expect("missing active worktree");
    let store_archived = state.store_for_session(session_archived.id).await.unwrap();
    let worktree_archived = store_archived
        .get_worktree(session_archived.worktree_id)
        .await
        .unwrap()
        .expect("missing archived worktree");

    let mut next = HashSet::new();
    next.insert(worktree_active.id);
    next.insert(worktree_archived.id);
    state
        .update_worktree_vcs_activity(&HashSet::new(), &next)
        .await;

    ctx_http::daemon::git_status::emit_worktree_vcs_snapshot_for_worktree(
        &state,
        &worktree_active,
        true,
    )
    .await
    .unwrap();
    ctx_http::daemon::git_status::emit_worktree_vcs_snapshot_for_worktree(
        &state,
        &worktree_archived,
        true,
    )
    .await
    .unwrap();

    let resp = client
        .post(format!("{base}/api/tasks/{}/archive", task_archived.id.0))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());

    let snapshot: ctx_core::models::WorkspaceActiveSnapshot = client
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

    let worktree_ids: HashSet<_> = snapshot
        .worktree_vcs_snapshots
        .iter()
        .map(|snapshot| snapshot.worktree_id)
        .collect();
    assert!(worktree_ids.contains(&worktree_active.id));
    assert!(!worktree_ids.contains(&worktree_archived.id));

    assert!(snapshot
        .active
        .tasks
        .iter()
        .any(|summary| summary.task.id == task_active.id));
    assert!(!snapshot
        .active
        .tasks
        .iter()
        .any(|summary| summary.task.id == task_archived.id));
}

#[tokio::test]
async fn workspace_active_heads_batch_strips_partials() {
    let (repo, _data_dir, state, server) = setup().await;
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

    let task =
        create_task_with_primary_worktree(client, &state, base, ws.id, repo.path(), "active-heads")
            .await;

    let session = create_primary_worktree_session(client, base, task.id).await;

    let store = state.store_for_session(session.id).await.unwrap();
    let now = Utc::now();
    let turn_id = TurnId::new();
    store
        .insert_session_turn(SessionTurn {
            turn_id,
            session_id: session.id,
            run_id: None,
            user_message_id: None,
            status: SessionTurnStatus::Running,
            start_seq: Some(1),
            end_seq: None,
            started_at: now,
            updated_at: now,
            assistant_partial: Some("partial".to_string()),
            thought_partial: Some("thinking".to_string()),
            metrics_json: None,
            tool_total: 0,
            tool_pending: 0,
            tool_running: 0,
            tool_completed: 0,
            tool_failed: 0,
        })
        .await
        .unwrap();
    store
        .append_session_event(
            session.id,
            None,
            Some(turn_id),
            SessionEventType::AssistantComplete,
            json!({
                "full_content": "final answer",
                "message_id": "provider-msg-1",
                "order_seq": 2
            }),
        )
        .await
        .unwrap();
    let checkpoint_event = store
        .append_session_event(
            session.id,
            None,
            Some(turn_id),
            SessionEventType::Notice,
            json!({ "kind": "test_checkpoint", "message": "stable" }),
        )
        .await
        .unwrap();
    store
        .update_session_turn_status(
            session.id,
            turn_id,
            SessionTurnStatus::Completed,
            Some(checkpoint_event.seq),
            None,
            Utc::now(),
        )
        .await
        .unwrap();

    let batch: ctx_core::models::WorkspaceActiveHeadBatch = client
        .get(format!("{base}/api/workspaces/{}/active_heads", ws.id.0))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let head = batch
        .heads
        .iter()
        .find(|head| head.session.id == session.id)
        .expect("missing session head");
    assert_eq!(head.turns.len(), 1);
    assert!(head.turns[0].assistant_partial.is_none());
    assert!(head.turns[0].thought_partial.is_none());
    assert!(head
        .events
        .iter()
        .all(|event| { !matches!(event.event_type, SessionEventType::AssistantComplete) }));
}

#[tokio::test]
async fn session_snapshot_returns_summary_only() {
    let (repo, _data_dir, state, server) = setup().await;
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

    let task =
        create_task_with_primary_worktree(client, &state, base, ws.id, repo.path(), "snapshot")
            .await;

    let session = create_primary_worktree_session(client, base, task.id).await;

    let snapshot: ctx_core::models::SessionSnapshot = client
        .get(format!(
            "{base}/api/sessions/{}/snapshot?limit=10",
            session.id.0
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(snapshot.summary.session.id, session.id);
    assert!(snapshot.head.is_none());
}

#[tokio::test]
async fn session_head_returns_head() {
    let (repo, _data_dir, state, server) = setup().await;
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

    let task =
        create_task_with_primary_worktree(client, &state, base, ws.id, repo.path(), "head").await;

    let session = create_primary_worktree_session(client, base, task.id).await;

    let head: ctx_core::models::SessionHeadSnapshot = client
        .get(format!(
            "{base}/api/sessions/{}/head?limit=10",
            session.id.0
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(head.session.id, session.id);
}

#[tokio::test]
async fn workspace_stream_replays_from_after_seq() {
    let (repo, _data_dir, state, server) = setup().await;
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

    let task =
        create_task_with_primary_worktree(client, &state, base, ws.id, repo.path(), "replay").await;

    let session = create_primary_worktree_session(client, base, task.id).await;
    state.sessions.remember_session_meta(&session).await;

    let store = state.store_for_session(session.id).await.unwrap();
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
    state.publish_event(ev1.clone()).await;
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
    state.publish_event(ev2.clone()).await;
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
    state.publish_event(ev3.clone()).await;

    let ws_url = format!("{base}/api/workspaces/{}/stream", ws.id.0).replace("http://", "ws://");
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
            "replay": {
                "mode": "resume",
                "after_seq": ev2.seq,
            },
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
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        let wait = remaining.min(Duration::from_millis(250));
        let next = tokio::time::timeout(wait, socket.next()).await;
        if let Ok(Some(Ok(WsMessage::Text(txt)))) = next {
            if let Ok(message) =
                serde_json::from_str::<ctx_core::models::WorkspaceActiveSnapshotStreamMessage>(&txt)
            {
                match message {
                    ctx_core::models::WorkspaceActiveSnapshotStreamMessage::Event {
                        event, ..
                    } => {
                        let ctx_core::models::WorkspaceActiveSnapshotEvent::SessionHeadDelta {
                            delta,
                            ..
                        } = event.as_ref()
                        else {
                            continue;
                        };
                        if delta.session_id != session.id {
                            continue;
                        }
                        if let Some(event) = delta.event.as_ref() {
                            if event.seq == ev3.seq {
                                seen_replay = true;
                            }
                            if event.seq <= ev2.seq {
                                seen_old = true;
                            }
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
                    _ => {}
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
}

#[tokio::test]
async fn workspace_stream_reset_replay_waits_for_fresh_resume_cursor() {
    let (repo, _data_dir, state, server) = setup().await;
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

    let task =
        create_task_with_primary_worktree(client, &state, base, ws.id, repo.path(), "replay").await;

    let session = create_primary_worktree_session(client, base, task.id).await;
    state.sessions.remember_session_meta(&session).await;

    let store = state.store_for_session(session.id).await.unwrap();
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
    state.publish_event(ev1.clone()).await;
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
    state.publish_event(ev2.clone()).await;
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
    state.publish_event(ev3.clone()).await;

    fn message_contains_seq(
        message: &ctx_core::models::WorkspaceActiveSnapshotStreamMessage,
        session_id: ctx_core::ids::SessionId,
        seq: i64,
    ) -> bool {
        match message {
            ctx_core::models::WorkspaceActiveSnapshotStreamMessage::Event { event, .. } => {
                match event.as_ref() {
                    ctx_core::models::WorkspaceActiveSnapshotEvent::SessionHeadDelta {
                        delta,
                        ..
                    } => {
                        delta.session_id == session_id
                            && delta.event.as_ref().map(|event| event.seq) == Some(seq)
                    }
                    _ => false,
                }
            }
            ctx_core::models::WorkspaceActiveSnapshotStreamMessage::HeadsBatch {
                deltas, ..
            } => deltas.iter().any(|delta| {
                delta.session_id == session_id
                    && delta.event.as_ref().map(|event| event.seq) == Some(seq)
            }),
            _ => false,
        }
    }

    let ws_url = format!("{base}/api/workspaces/{}/stream", ws.id.0).replace("http://", "ws://");
    let (mut socket, _) = connect_async(&ws_url).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    let initial_subscribe = json!({
        "type": "subscribe",
        "sessions": [{
            "session_id": session.id.0,
            "replay": {
                "mode": "resume",
                "after_seq": ev2.seq,
            },
        }],
    })
    .to_string();
    socket
        .send(WsMessage::Text(initial_subscribe.into()))
        .await
        .unwrap();

    let mut saw_initial_replay = false;
    let initial_deadline = tokio::time::Instant::now() + Duration::from_secs(4);
    while tokio::time::Instant::now() < initial_deadline {
        let wait = initial_deadline
            .saturating_duration_since(tokio::time::Instant::now())
            .min(Duration::from_millis(250));
        let next = tokio::time::timeout(wait, socket.next()).await;
        if let Ok(Some(Ok(WsMessage::Text(txt)))) = next {
            if let Ok(message) =
                serde_json::from_str::<ctx_core::models::WorkspaceActiveSnapshotStreamMessage>(&txt)
            {
                if message_contains_seq(&message, session.id, ev3.seq) {
                    saw_initial_replay = true;
                    break;
                }
            }
        }
    }
    assert!(saw_initial_replay, "expected initial replay before reset");

    let reset_subscribe = json!({
        "type": "subscribe",
        "sessions": [{
            "session_id": session.id.0,
            "replay": {
                "mode": "reset",
            },
        }],
    })
    .to_string();
    socket
        .send(WsMessage::Text(reset_subscribe.into()))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;

    let ev4 = store
        .append_session_event(
            session.id,
            None,
            None,
            SessionEventType::Notice,
            json!({"msg":"four"}),
        )
        .await
        .unwrap();
    state.publish_event(ev4.clone()).await;

    let mut saw_reset_window_event = false;
    let reset_deadline = tokio::time::Instant::now() + Duration::from_millis(750);
    while tokio::time::Instant::now() < reset_deadline {
        let wait = reset_deadline
            .saturating_duration_since(tokio::time::Instant::now())
            .min(Duration::from_millis(200));
        let next = tokio::time::timeout(wait, socket.next()).await;
        if let Ok(Some(Ok(WsMessage::Text(txt)))) = next {
            if let Ok(message) =
                serde_json::from_str::<ctx_core::models::WorkspaceActiveSnapshotStreamMessage>(&txt)
            {
                if message_contains_seq(&message, session.id, ev4.seq) {
                    saw_reset_window_event = true;
                    break;
                }
            }
        }
    }
    assert!(
        !saw_reset_window_event,
        "reset replay should stay quiet until the client resumes from a fresh head cursor"
    );

    let resume_subscribe = json!({
        "type": "subscribe",
        "sessions": [{
            "session_id": session.id.0,
            "replay": {
                "mode": "resume",
                "after_seq": ev3.seq,
            },
        }],
    })
    .to_string();
    socket
        .send(WsMessage::Text(resume_subscribe.into()))
        .await
        .unwrap();

    let mut saw_resumed_event = false;
    let resume_deadline = tokio::time::Instant::now() + Duration::from_secs(4);
    while tokio::time::Instant::now() < resume_deadline {
        let wait = resume_deadline
            .saturating_duration_since(tokio::time::Instant::now())
            .min(Duration::from_millis(250));
        let next = tokio::time::timeout(wait, socket.next()).await;
        if let Ok(Some(Ok(WsMessage::Text(txt)))) = next {
            if let Ok(message) =
                serde_json::from_str::<ctx_core::models::WorkspaceActiveSnapshotStreamMessage>(&txt)
            {
                if message_contains_seq(&message, session.id, ev4.seq) {
                    saw_resumed_event = true;
                    break;
                }
            }
        }
    }
    assert!(
        saw_resumed_event,
        "expected explicit resume to replay the event that arrived during reset recovery"
    );
}

#[tokio::test]
async fn workspace_stream_replays_tool_events() {
    let (repo, _data_dir, state, server) = setup().await;
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

    let task =
        create_task_with_primary_worktree(client, &state, base, ws.id, repo.path(), "tool-replay")
            .await;

    let session = create_primary_worktree_session(client, base, task.id).await;
    state.sessions.remember_session_meta(&session).await;

    let store = state.store_for_session(session.id).await.unwrap();
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
    state.publish_event(ev1.clone()).await;
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
    state.publish_event(ev2.clone()).await;
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
    state.publish_event(ev3.clone()).await;

    let ws_url = format!("{base}/api/workspaces/{}/stream", ws.id.0).replace("http://", "ws://");
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
            "replay": {
                "mode": "resume",
                "after_seq": ev1.seq,
            },
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
            if let Ok(message) =
                serde_json::from_str::<ctx_core::models::WorkspaceActiveSnapshotStreamMessage>(&txt)
            {
                match message {
                    ctx_core::models::WorkspaceActiveSnapshotStreamMessage::Event {
                        event, ..
                    } => {
                        let ctx_core::models::WorkspaceActiveSnapshotEvent::SessionHeadDelta {
                            delta,
                            ..
                        } = event.as_ref()
                        else {
                            continue;
                        };
                        if delta.session_id != session.id {
                            continue;
                        }
                        if let Some(event) = delta.event.as_ref() {
                            match event.event_type {
                                SessionEventType::ToolCall => saw_call = true,
                                SessionEventType::ToolResult => saw_result = true,
                                _ => {}
                            }
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
                            if let Some(event) = delta.event {
                                match event.event_type {
                                    SessionEventType::ToolCall => saw_call = true,
                                    SessionEventType::ToolResult => saw_result = true,
                                    _ => {}
                                }
                            }
                        }
                    }
                    _ => {}
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
}

#[tokio::test]
async fn workspace_stream_under_load_no_gap_or_reset() {
    let (repo, _data_dir, state, server) = setup().await;
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

    let task =
        create_task_with_primary_worktree(client, &state, base, ws.id, repo.path(), "load").await;

    let session = create_primary_worktree_session(client, base, task.id).await;
    state.sessions.remember_session_meta(&session).await;

    let ws_url = format!("{base}/api/workspaces/{}/stream", ws.id.0).replace("http://", "ws://");
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
            "replay": {
                "mode": "resume",
                "after_seq": 1,
            },
        }],
    })
    .to_string();
    socket
        .send(WsMessage::Text(subscribe.into()))
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(150)).await;

    let store = state.store_for_session(session.id).await.unwrap();
    let total_events = 150usize;
    let mut last_seq = 0;
    for i in 0..total_events {
        let ev = store
            .append_session_event(
                session.id,
                None,
                None,
                SessionEventType::Notice,
                json!({"msg": format!("load-{i}")}),
            )
            .await
            .unwrap();
        last_seq = ev.seq;
        state.publish_event(ev).await;
    }

    let mut saw_reset = false;
    let mut saw_gap = false;
    let mut last_seen_seq = 0;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        let wait = remaining.min(Duration::from_millis(250));
        match tokio::time::timeout(wait, socket.next()).await {
            Ok(Some(Ok(WsMessage::Text(txt)))) => {
                if let Ok(message) = serde_json::from_str::<
                    ctx_core::models::WorkspaceActiveSnapshotStreamMessage,
                >(&txt)
                {
                    match message {
                        ctx_core::models::WorkspaceActiveSnapshotStreamMessage::ResetRequired {
                            ..
                        } => {
                            saw_reset = true;
                            break;
                        }
                        ctx_core::models::WorkspaceActiveSnapshotStreamMessage::Event {
                            event,
                            ..
                        } => match event.as_ref() {
                            ctx_core::models::WorkspaceActiveSnapshotEvent::SessionGap {
                                ..
                            } => {
                                saw_gap = true;
                                break;
                            }
                            ctx_core::models::WorkspaceActiveSnapshotEvent::SessionHeadDelta {
                                delta,
                                ..
                            } if delta.session_id == session.id => {
                                last_seen_seq = last_seen_seq.max(delta.last_event_seq);
                            }
                            _ => {}
                        },
                        ctx_core::models::WorkspaceActiveSnapshotStreamMessage::HeadsBatch {
                            deltas,
                            ..
                        } => {
                            for delta in deltas {
                                if delta.session_id == session.id {
                                    last_seen_seq = last_seen_seq.max(delta.last_event_seq);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            Ok(Some(Ok(WsMessage::Close(_)))) => {
                panic!("workspace stream closed under load");
            }
            Ok(Some(Ok(_))) => {}
            Ok(Some(Err(err))) => panic!("workspace stream error: {err:?}"),
            Ok(None) => panic!("workspace stream closed under load"),
            Err(_) => {}
        }

        if last_seen_seq >= last_seq {
            break;
        }
    }

    assert!(!saw_reset, "unexpected reset_required under load");
    assert!(!saw_gap, "unexpected session_gap under load");
    assert!(
        last_seen_seq >= last_seq,
        "stream did not deliver events up to last seq"
    );
}

#[tokio::test]
async fn workspace_stream_emits_git_status_snapshot_on_change() {
    let (repo, _data_dir, state, server) = setup_git().await;
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

    let task =
        create_task_with_primary_worktree(client, &state, base, ws.id, repo.path(), "git-status")
            .await;

    let session = create_primary_worktree_session(client, base, task.id).await;
    state.sessions.remember_session_meta(&session).await;
    let store = state.store_for_session(session.id).await.unwrap();
    let worktree = store
        .get_worktree(session.worktree_id)
        .await
        .unwrap()
        .expect("missing worktree");
    let worktree_id = worktree.id;

    let ws_url = format!("{base}/api/workspaces/{}/stream", ws.id.0).replace("http://", "ws://");
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
            "replay": {
                "mode": "auto",
            },
        }],
    })
    .to_string();
    socket
        .send(WsMessage::Text(subscribe.into()))
        .await
        .unwrap();

    let mut saw_clean_snapshot = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let wait = remaining.min(Duration::from_millis(250));
        let next = tokio::time::timeout(wait, socket.next()).await;
        if let Ok(Some(Ok(WsMessage::Text(txt)))) = next {
            if let Ok(message) =
                serde_json::from_str::<ctx_core::models::WorkspaceActiveSnapshotStreamMessage>(&txt)
            {
                if let Some(untracked) = git_status_untracked_from_message(message, worktree_id) {
                    if untracked == 0 {
                        saw_clean_snapshot = true;
                        break;
                    }
                }
            }
        }
    }
    assert!(
        saw_clean_snapshot,
        "expected initial clean git status snapshot"
    );

    let file_path = Path::new(&worktree.root_path).join("git-status-live.txt");
    tokio::fs::write(&file_path, "change\n").await.unwrap();

    let mut saw_untracked = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let wait = remaining.min(Duration::from_millis(250));
        let next = tokio::time::timeout(wait, socket.next()).await;
        if let Ok(Some(Ok(WsMessage::Text(txt)))) = next {
            if let Ok(message) =
                serde_json::from_str::<ctx_core::models::WorkspaceActiveSnapshotStreamMessage>(&txt)
            {
                if let Some(untracked) = git_status_untracked_from_message(message, worktree_id) {
                    if untracked >= 1 {
                        saw_untracked = true;
                        break;
                    }
                }
            }
        }
    }
    assert!(
        saw_untracked,
        "expected git status update after file change"
    );
}

#[tokio::test]
async fn workspace_stream_emits_git_status_snapshot_for_new_subscriber() {
    let (repo, _data_dir, state, server) = setup_git().await;
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

    let task = create_task_with_primary_worktree(
        client,
        &state,
        base,
        ws.id,
        repo.path(),
        "git-status-new-subscriber",
    )
    .await;

    let session_one = create_primary_worktree_session(client, base, task.id).await;
    state.sessions.remember_session_meta(&session_one).await;
    let store_one = state.store_for_session(session_one.id).await.unwrap();
    let worktree_one = store_one
        .get_worktree(session_one.worktree_id)
        .await
        .unwrap()
        .expect("missing worktree");
    let worktree_one_id = worktree_one.id;

    let ws_url = format!("{base}/api/workspaces/{}/stream", ws.id.0).replace("http://", "ws://");
    let (mut socket_one, _) = connect_async(&ws_url).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(2), socket_one.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    let subscribe_one = json!({
        "type": "subscribe",
        "sessions": [{
            "session_id": session_one.id.0,
            "replay": {
                "mode": "auto",
            },
        }],
    })
    .to_string();
    socket_one
        .send(WsMessage::Text(subscribe_one.into()))
        .await
        .unwrap();

    let mut saw_initial = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let wait = remaining.min(Duration::from_millis(250));
        let next = tokio::time::timeout(wait, socket_one.next()).await;
        if let Ok(Some(Ok(WsMessage::Text(txt)))) = next {
            if let Ok(message) =
                serde_json::from_str::<ctx_core::models::WorkspaceActiveSnapshotStreamMessage>(&txt)
            {
                if let Some(untracked) = git_status_untracked_from_message(message, worktree_one_id)
                {
                    if untracked == 0 {
                        saw_initial = true;
                        break;
                    }
                }
            }
        }
    }
    assert!(saw_initial, "expected initial git status snapshot");

    let session_two = create_primary_worktree_session(client, base, task.id).await;
    state.sessions.remember_session_meta(&session_two).await;
    let store_two = state.store_for_session(session_two.id).await.unwrap();
    let worktree_two = store_two
        .get_worktree(session_two.worktree_id)
        .await
        .unwrap()
        .expect("missing worktree");
    let worktree_two_id = worktree_two.id;

    let (mut socket_two, _) = connect_async(&ws_url).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(2), socket_two.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    let subscribe_two = json!({
        "type": "subscribe",
        "sessions": [{
            "session_id": session_two.id.0,
            "replay": {
                "mode": "auto",
            },
        }],
    })
    .to_string();
    socket_two
        .send(WsMessage::Text(subscribe_two.into()))
        .await
        .unwrap();

    let mut saw_second = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let wait = remaining.min(Duration::from_millis(250));
        let next = tokio::time::timeout(wait, socket_two.next()).await;
        if let Ok(Some(Ok(WsMessage::Text(txt)))) = next {
            if let Ok(message) =
                serde_json::from_str::<ctx_core::models::WorkspaceActiveSnapshotStreamMessage>(&txt)
            {
                if let Some(untracked) = git_status_untracked_from_message(message, worktree_two_id)
                {
                    if untracked == 0 {
                        saw_second = true;
                        break;
                    }
                }
            }
        }
    }
    assert!(
        saw_second,
        "expected git status snapshot for new subscriber"
    );
}

#[tokio::test]
async fn workspace_stream_delivers_snapshot_before_worktree_vcs_summary_refresh() {
    let (repo, _data_dir, state, server) = setup_git().await;
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

    let task =
        create_task_with_primary_worktree(client, &state, base, ws.id, repo.path(), "hydrate-vcs")
            .await;
    let session = create_primary_worktree_session(client, base, task.id).await;

    let store = state.store_for_session(session.id).await.unwrap();
    let worktree = store
        .get_worktree(session.worktree_id)
        .await
        .unwrap()
        .expect("missing worktree");
    tokio::fs::write(
        Path::new(&worktree.root_path).join("file.txt"),
        "hello\nchanged\n",
    )
    .await
    .unwrap();

    let ws_url = format!("{base}/api/workspaces/{}/stream", ws.id.0).replace("http://", "ws://");
    let (mut socket, _) = connect_async(&ws_url).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    let subscribe = json!({
        "type": "subscribe",
        "scope": "active",
        "include_active_heads": true,
    })
    .to_string();
    socket
        .send(WsMessage::Text(subscribe.into()))
        .await
        .unwrap();

    let first = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let WsMessage::Text(first_text) = first else {
        panic!("expected text frame after subscribe");
    };
    let first_message: ctx_core::models::WorkspaceActiveSnapshotStreamMessage =
        serde_json::from_str(&first_text).unwrap();
    let ctx_core::models::WorkspaceActiveSnapshotStreamMessage::Snapshot {
        active_snapshot, ..
    } = first_message
    else {
        panic!("expected initial snapshot after subscribe");
    };
    let initial_worktree = active_snapshot
        .worktree_vcs_snapshots
        .into_iter()
        .find(|snapshot| snapshot.worktree_id == worktree.id);
    assert!(
        initial_worktree
            .as_ref()
            .and_then(|snapshot| snapshot.summary.file_count)
            .is_none(),
        "initial snapshot should not wait for ready worktree vcs summary"
    );

    let mut hydrated_file_count = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let wait = remaining.min(Duration::from_millis(250));
        let next = tokio::time::timeout(wait, socket.next()).await;
        if let Ok(Some(Ok(WsMessage::Text(txt)))) = next {
            if let Ok(message) =
                serde_json::from_str::<ctx_core::models::WorkspaceActiveSnapshotStreamMessage>(&txt)
            {
                if let Some(snapshot) = worktree_vcs_snapshot_from_message(message, worktree.id) {
                    if snapshot.freshness == WorktreeVcsFreshness::Fresh {
                        hydrated_file_count = snapshot.summary.file_count;
                        break;
                    }
                }
            }
        }
    }

    assert_eq!(
        hydrated_file_count,
        Some(1),
        "expected later worktree vcs event to include ready worktree vcs counts"
    );
}

#[tokio::test]
async fn workspace_stream_repeat_subscribe_preserves_ready_worktree_vcs_state() {
    let (repo, _data_dir, state, server) = setup_git().await;
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

    let task =
        create_task_with_primary_worktree(client, &state, base, ws.id, repo.path(), "ready-vcs")
            .await;
    let session = create_primary_worktree_session(client, base, task.id).await;
    let store = state.store_for_session(session.id).await.unwrap();
    let worktree = store
        .get_worktree(session.worktree_id)
        .await
        .unwrap()
        .expect("missing worktree");

    let mut next = HashSet::new();
    next.insert(worktree.id);
    state
        .update_worktree_vcs_activity(&HashSet::new(), &next)
        .await;
    ctx_http::daemon::git_status::refresh_worktree_vcs_summary(state.clone(), worktree.clone())
        .await
        .unwrap();

    let seeded = state
        .get_worktree_vcs_snapshot(worktree.id)
        .await
        .expect("expected seeded worktree vcs snapshot");
    assert_eq!(seeded.freshness, WorktreeVcsFreshness::Fresh);

    let ws_url = format!("{base}/api/workspaces/{}/stream", ws.id.0).replace("http://", "ws://");
    let (mut socket, _) = connect_async(&ws_url).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    let subscribe = json!({
        "type": "subscribe",
        "scope": "active",
        "include_active_heads": true,
    })
    .to_string();
    socket
        .send(WsMessage::Text(subscribe.into()))
        .await
        .unwrap();

    let first = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let WsMessage::Text(first_text) = first else {
        panic!("expected text frame after subscribe");
    };
    let first_message: ctx_core::models::WorkspaceActiveSnapshotStreamMessage =
        serde_json::from_str(&first_text).unwrap();
    let initial_worktree =
        worktree_vcs_snapshot_from_message(first_message, worktree.id).expect("missing worktree");
    assert_eq!(initial_worktree.freshness, WorktreeVcsFreshness::Fresh);

    let watcher_deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        {
            let watchers = state.workspaces.git_status_watchers.lock().await;
            if watchers.contains(&worktree.id) {
                break;
            }
        }
        assert!(
            tokio::time::Instant::now() < watcher_deadline,
            "repeat subscribe should register the worktree VCS watcher"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // The regression this test guards is subscription warm-up downgrading an
    // already-ready snapshot. Keep the stability window shorter than the
    // filesystem watcher debounce so Linux watcher startup noise is tested by
    // watcher-specific coverage instead of this subscribe contract.
    let deadline = tokio::time::Instant::now() + Duration::from_millis(250);
    while tokio::time::Instant::now() < deadline {
        let snapshot = state
            .workspaces
            .workspace_active_snapshot
            .active_snapshot(ws.id, 10)
            .await;
        let current = snapshot
            .worktree_vcs_snapshots
            .into_iter()
            .find(|candidate| candidate.worktree_id == worktree.id)
            .expect("expected worktree snapshot to remain published");
        assert_eq!(
            current.freshness,
            WorktreeVcsFreshness::Fresh,
            "repeat subscribe should not downgrade ready worktree vcs state"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn workspace_stream_subscribe_does_not_reemit_when_worktree_vcs_is_already_computing() {
    let (repo, _data_dir, state, server) = setup_git().await;
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

    let task = create_task_with_primary_worktree(
        client,
        &state,
        base,
        ws.id,
        repo.path(),
        "computing-vcs",
    )
    .await;
    let session = create_primary_worktree_session(client, base, task.id).await;
    let store = state.store_for_session(session.id).await.unwrap();
    let worktree = store
        .get_worktree(session.worktree_id)
        .await
        .unwrap()
        .expect("missing worktree");

    let mut next = HashSet::new();
    next.insert(worktree.id);
    state
        .update_worktree_vcs_activity(&HashSet::new(), &next)
        .await;
    ctx_http::daemon::git_status::refresh_worktree_vcs_summary(state.clone(), worktree.clone())
        .await
        .unwrap();
    tokio::fs::write(
        Path::new(&worktree.root_path).join("file.txt"),
        "hello\nchanged\n",
    )
    .await
    .unwrap();

    let refresh_lock = state.worktree_vcs_refresh_lock(worktree.id).await;
    let _refresh_guard = refresh_lock.lock().await;
    ctx_http::daemon::git_status::request_worktree_vcs_refresh(&state, &worktree, true, false)
        .await
        .unwrap();

    let seeded = state
        .get_worktree_vcs_snapshot(worktree.id)
        .await
        .expect("expected seeded worktree vcs snapshot");
    assert_eq!(seeded.freshness, WorktreeVcsFreshness::Stale);
    let seeded_rev = seeded.rev;

    let ws_url = format!("{base}/api/workspaces/{}/stream", ws.id.0).replace("http://", "ws://");
    let (mut socket, _) = connect_async(&ws_url).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    let subscribe = json!({
        "type": "subscribe",
        "scope": "active",
        "include_active_heads": true,
    })
    .to_string();
    socket
        .send(WsMessage::Text(subscribe.into()))
        .await
        .unwrap();

    let first = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let WsMessage::Text(first_text) = first else {
        panic!("expected text frame after subscribe");
    };
    let first_message: ctx_core::models::WorkspaceActiveSnapshotStreamMessage =
        serde_json::from_str(&first_text).unwrap();
    let initial_worktree =
        worktree_vcs_snapshot_from_message(first_message, worktree.id).expect("missing worktree");
    assert_eq!(initial_worktree.freshness, WorktreeVcsFreshness::Stale);
    assert_eq!(
        initial_worktree.rev, seeded_rev,
        "subscribe should reuse the in-flight computing snapshot instead of force-emitting again"
    );

    let deadline = tokio::time::Instant::now() + Duration::from_millis(300);
    while tokio::time::Instant::now() < deadline {
        let snapshot = state
            .get_worktree_vcs_snapshot(worktree.id)
            .await
            .expect("expected worktree vcs snapshot to remain present");
        assert_eq!(
            snapshot.rev, seeded_rev,
            "subscribe should not bump the worktree vcs rev while the summary refresh is already in flight"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn worktree_vcs_summary_refresh_reloads_live_inventory_before_ready_publish() {
    let (repo, _data_dir, state, server) = setup_git().await;
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

    let task = create_task_with_primary_worktree(
        client,
        &state,
        base,
        ws.id,
        repo.path(),
        "summary-refresh-live-inventory",
    )
    .await;
    let session = create_primary_worktree_session(client, base, task.id).await;
    let store = state.store_for_session(session.id).await.unwrap();
    let worktree = store
        .get_worktree(session.worktree_id)
        .await
        .unwrap()
        .expect("missing worktree");

    let first_path = Path::new(&worktree.root_path).join("first.txt");
    let second_path = Path::new(&worktree.root_path).join("second.txt");
    tokio::fs::write(&first_path, "first\n").await.unwrap();

    let mut next = HashSet::new();
    next.insert(worktree.id);
    state
        .update_worktree_vcs_activity(&HashSet::new(), &next)
        .await;
    state
        .update_worktree_vcs_open_panes(&HashSet::new(), &next)
        .await;
    ctx_http::daemon::git_status::emit_worktree_vcs_snapshot_for_worktree(&state, &worktree, true)
        .await
        .unwrap();

    tokio::fs::remove_file(&first_path).await.unwrap();
    tokio::fs::write(&second_path, "second\n").await.unwrap();
    ctx_http::daemon::git_status::request_worktree_vcs_refresh(&state, &worktree, true, true)
        .await
        .unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        let snapshot = state
            .get_worktree_vcs_snapshot(worktree.id)
            .await
            .expect("expected worktree vcs snapshot");
        if snapshot.freshness == WorktreeVcsFreshness::Fresh {
            let touched_paths = snapshot
                .touched_files
                .items
                .iter()
                .map(|item| item.path.as_str())
                .collect::<Vec<_>>();
            assert!(
                touched_paths.contains(&"second.txt"),
                "summary refresh should recompute live inventory before publishing ready state"
            );
            assert!(
                !touched_paths.contains(&"first.txt"),
                "ready snapshot should not publish stale touched-file inventory"
            );
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    panic!("timed out waiting for ready worktree vcs snapshot");
}

#[tokio::test]
async fn worktree_vcs_emit_and_summary_refresh_share_refresh_lock() {
    let (repo, _data_dir, state, server) = setup_git().await;
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

    let task = create_task_with_primary_worktree(
        client,
        &state,
        base,
        ws.id,
        repo.path(),
        "shared-refresh-lock",
    )
    .await;
    let session = create_primary_worktree_session(client, base, task.id).await;
    let store = state.store_for_session(session.id).await.unwrap();
    let worktree = store
        .get_worktree(session.worktree_id)
        .await
        .unwrap()
        .expect("worktree");

    state
        .update_worktree_vcs_activity(&HashSet::new(), &HashSet::from([worktree.id]))
        .await;

    let refresh_lock = state.worktree_vcs_refresh_lock(worktree.id).await;
    let refresh_guard = refresh_lock.lock().await;

    let emit_state = state.clone();
    let emit_worktree = worktree.clone();
    let emit_handle = tokio::spawn(async move {
        ctx_http::daemon::git_status::emit_worktree_vcs_snapshot_for_worktree(
            &emit_state,
            &emit_worktree,
            false,
        )
        .await
    });

    let summary_state = state.clone();
    let summary_worktree = worktree.clone();
    let summary_handle = tokio::spawn(async move {
        ctx_http::daemon::git_status::refresh_worktree_vcs_summary(summary_state, summary_worktree)
            .await
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        !emit_handle.is_finished(),
        "emit path should wait on the shared per-worktree refresh lock"
    );
    assert!(
        !summary_handle.is_finished(),
        "summary refresh should wait on the shared per-worktree refresh lock"
    );

    drop(refresh_guard);

    emit_handle.await.unwrap().unwrap();
    summary_handle.await.unwrap().unwrap();
}

#[tokio::test]
async fn worktree_vcs_activity_eviction_drops_refresh_lock() {
    let (repo, _data_dir, state, server) = setup_git().await;
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

    let task = create_task_with_primary_worktree(
        client,
        &state,
        base,
        ws.id,
        repo.path(),
        "refresh-lock-eviction",
    )
    .await;
    let session = create_primary_worktree_session(client, base, task.id).await;
    let store = state.store_for_session(session.id).await.unwrap();
    let worktree = store
        .get_worktree(session.worktree_id)
        .await
        .unwrap()
        .expect("worktree");

    let active = HashSet::from([worktree.id]);
    state
        .update_worktree_vcs_activity(&HashSet::new(), &active)
        .await;

    let initial_lock = state.worktree_vcs_refresh_lock(worktree.id).await;
    state
        .update_worktree_vcs_activity(&active, &HashSet::new())
        .await;

    let next_lock = state.worktree_vcs_refresh_lock(worktree.id).await;
    assert!(
        Arc::ptr_eq(&initial_lock, &next_lock),
        "worktree VCS reactivation should reuse an in-flight refresh lock"
    );

    let old_lock = Arc::downgrade(&initial_lock);
    drop(next_lock);
    drop(initial_lock);

    let replacement_lock = state.worktree_vcs_refresh_lock(worktree.id).await;
    assert!(
        old_lock.upgrade().is_none(),
        "evicted refresh lock should eventually be released once no refreshes are using it"
    );
    assert_eq!(Arc::strong_count(&replacement_lock), 1);
}

#[tokio::test]
async fn workspace_stream_replay_only_subscribe_reseeds_cached_worktree_vcs_snapshot() {
    let (repo, _data_dir, state, server) = setup_git().await;
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

    let task =
        create_task_with_primary_worktree(client, &state, base, ws.id, repo.path(), "reseed-vcs")
            .await;
    let session = create_primary_worktree_session(client, base, task.id).await;
    let store = state.store_for_session(session.id).await.unwrap();
    let worktree = store
        .get_worktree(session.worktree_id)
        .await
        .unwrap()
        .expect("missing worktree");

    let mut next = HashSet::new();
    next.insert(worktree.id);
    state
        .update_worktree_vcs_activity(&HashSet::new(), &next)
        .await;
    ctx_http::daemon::git_status::refresh_worktree_vcs_summary(state.clone(), worktree.clone())
        .await
        .unwrap();

    let ws_url = format!("{base}/api/workspaces/{}/stream", ws.id.0).replace("http://", "ws://");
    let (mut socket, _) = connect_async(&ws_url).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    let after_seq = state
        .workspaces
        .workspace_active_snapshot
        .session_last_event_seq(ws.id, session.id)
        .await;
    let subscribe = json!({
        "type": "subscribe",
        "include_active_heads": false,
        "sessions": [{
            "session_id": session.id.0,
            "replay": {
                "mode": "resume",
                "after_seq": after_seq,
            },
        }],
    })
    .to_string();
    socket
        .send(WsMessage::Text(subscribe.into()))
        .await
        .unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    let mut saw_seed = false;
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let wait = remaining.min(Duration::from_millis(250));
        let next = tokio::time::timeout(wait, socket.next()).await;
        if let Ok(Some(Ok(WsMessage::Text(txt)))) = next {
            let Ok(message) = serde_json::from_str::<
                ctx_core::models::WorkspaceActiveSnapshotStreamMessage,
            >(&txt) else {
                continue;
            };
            let Some(snapshot) = worktree_vcs_snapshot_from_message(message, worktree.id) else {
                continue;
            };
            assert_eq!(snapshot.freshness, WorktreeVcsFreshness::Fresh);
            saw_seed = true;
            break;
        }
    }

    assert!(
        saw_seed,
        "expected replay-only subscribe to re-seed cached worktree vcs snapshot"
    );
}

#[tokio::test]
async fn mobile_secure_workspace_stream_replay_only_subscribe_reseeds_cached_worktree_vcs_snapshot()
{
    let (repo, _data_dir, state, server) = setup_git().await;
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

    let task =
        create_task_with_primary_worktree(client, &state, base, ws.id, repo.path(), "secure-vcs")
            .await;
    let session = create_primary_worktree_session(client, base, task.id).await;
    let store = state.store_for_session(session.id).await.unwrap();
    let worktree = store
        .get_worktree(session.worktree_id)
        .await
        .unwrap()
        .expect("missing worktree");

    let mut next = HashSet::new();
    next.insert(worktree.id);
    state
        .update_worktree_vcs_activity(&HashSet::new(), &next)
        .await;
    ctx_http::daemon::git_status::refresh_worktree_vcs_summary(state.clone(), worktree.clone())
        .await
        .unwrap();

    let (device_id, key) = configure_mobile_secure_access(&state).await;
    let ws_url = build_mobile_secure_ws_url(base, ws.id, &device_id, &key);
    let (mut socket, _) = connect_async(&ws_url).await.unwrap();
    let ready =
        recv_secure_workspace_message(&mut socket, &key, &device_id, Duration::from_secs(2))
            .await
            .expect("expected secure ready frame");
    assert!(matches!(
        ready,
        WorkspaceActiveSnapshotStreamMessage::Event { event, .. }
            if matches!(event.as_ref(), WorkspaceActiveSnapshotEvent::Ready { .. })
    ));

    let after_seq = state
        .workspaces
        .workspace_active_snapshot
        .session_last_event_seq(ws.id, session.id)
        .await;
    send_secure_workspace_subscribe(
        &mut socket,
        &key,
        &device_id,
        1,
        json!({
            "type": "subscribe",
            "include_active_heads": false,
            "sessions": [{
                "session_id": session.id.0,
                "replay": {
                    "mode": "resume",
                    "after_seq": after_seq,
                },
            }],
        }),
    )
    .await;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    let mut saw_seed = false;
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let wait = remaining.min(Duration::from_millis(250));
        let Some(message) =
            recv_secure_workspace_message(&mut socket, &key, &device_id, wait).await
        else {
            continue;
        };
        let Some(snapshot) = worktree_vcs_snapshot_from_message(message, worktree.id) else {
            continue;
        };
        assert_eq!(snapshot.freshness, WorktreeVcsFreshness::Fresh);
        saw_seed = true;
        break;
    }

    assert!(
        saw_seed,
        "expected secure replay-only subscribe to re-seed cached worktree vcs snapshot"
    );
}

#[tokio::test]
async fn workspace_stream_repeat_subscribe_rescans_fresh_unavailable_worktree_vcs() {
    let (repo, _data_dir, state, server) = setup_git().await;
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

    let task = create_task_with_primary_worktree(
        client,
        &state,
        base,
        ws.id,
        repo.path(),
        "unavailable-refresh",
    )
    .await;
    let session = create_primary_worktree_session(client, base, task.id).await;
    let store = state.store_for_session(session.id).await.unwrap();
    let worktree = store
        .get_worktree(session.worktree_id)
        .await
        .unwrap()
        .expect("missing worktree");
    let mut next = HashSet::new();
    next.insert(worktree.id);
    state
        .update_worktree_vcs_activity(&HashSet::new(), &next)
        .await;

    let worktree_root = Path::new(&worktree.root_path);
    let saved_git_marker = worktree_root.join(".git.ctx-test-saved");
    tokio::fs::rename(worktree_root.join(".git"), &saved_git_marker)
        .await
        .expect("save original .git marker");
    tokio::fs::write(
        worktree_root.join(".git"),
        "gitdir: /definitely/missing/ctx-test-gitdir\n",
    )
    .await
    .expect("write poisoned .git marker");
    ctx_http::daemon::git_status::emit_worktree_vcs_snapshot_for_worktree(&state, &worktree, true)
        .await
        .unwrap();
    let unavailable = state
        .get_worktree_vcs_snapshot(worktree.id)
        .await
        .expect("expected unavailable snapshot");
    assert_eq!(unavailable.freshness, WorktreeVcsFreshness::Fresh);
    assert!(!unavailable.available);

    let ws_url = format!("{base}/api/workspaces/{}/stream", ws.id.0).replace("http://", "ws://");
    let (mut socket, _) = connect_async(&ws_url).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    let subscribe = json!({
        "type": "subscribe",
        "include_active_heads": false,
        "sessions": [{
            "session_id": session.id.0,
            "replay": {
                "mode": "resume",
                "after_seq": 0,
            },
        }],
    })
    .to_string();
    socket
        .send(WsMessage::Text(subscribe.clone().into()))
        .await
        .unwrap();

    let mut saw_unavailable = false;
    let first_deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < first_deadline {
        let remaining = first_deadline.saturating_duration_since(tokio::time::Instant::now());
        let wait = remaining.min(Duration::from_millis(250));
        let next = tokio::time::timeout(wait, socket.next()).await;
        if let Ok(Some(Ok(WsMessage::Text(txt)))) = next {
            let Ok(message) = serde_json::from_str::<
                ctx_core::models::WorkspaceActiveSnapshotStreamMessage,
            >(&txt) else {
                continue;
            };
            let Some(snapshot) = worktree_vcs_snapshot_from_message(message, worktree.id) else {
                continue;
            };
            if !snapshot.available {
                saw_unavailable = true;
                break;
            }
        }
    }
    assert!(
        saw_unavailable,
        "expected initial replay-only subscribe to surface cached unavailable worktree vcs state"
    );

    remove_git_marker(worktree_root).await;
    tokio::fs::rename(&saved_git_marker, worktree_root.join(".git"))
        .await
        .expect("restore original .git marker");

    socket
        .send(WsMessage::Text(subscribe.into()))
        .await
        .unwrap();

    let mut recovered = None;
    let second_deadline = tokio::time::Instant::now() + Duration::from_secs(6);
    while tokio::time::Instant::now() < second_deadline {
        let remaining = second_deadline.saturating_duration_since(tokio::time::Instant::now());
        let wait = remaining.min(Duration::from_millis(250));
        let next = tokio::time::timeout(wait, socket.next()).await;
        if let Ok(Some(Ok(WsMessage::Text(txt)))) = next {
            let Ok(message) = serde_json::from_str::<
                ctx_core::models::WorkspaceActiveSnapshotStreamMessage,
            >(&txt) else {
                continue;
            };
            let Some(snapshot) = worktree_vcs_snapshot_from_message(message, worktree.id) else {
                continue;
            };
            if snapshot.available {
                recovered = Some(snapshot);
                break;
            }
        }
    }

    let recovered =
        recovered.expect("expected repeat subscribe to refresh unavailable worktree vcs");
    assert!(recovered.available);
    assert_eq!(recovered.freshness, WorktreeVcsFreshness::Fresh);
}

#[tokio::test]
async fn workspace_stream_initial_snapshot_includes_worktree_vcs_for_explicit_archived_session() {
    let (repo, _data_dir, state, server) = setup_git().await;
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

    let task =
        create_task_with_primary_worktree(client, &state, base, ws.id, repo.path(), "archived-vcs")
            .await;
    let session = create_primary_worktree_session(client, base, task.id).await;
    let store = state.store_for_session(session.id).await.unwrap();
    let worktree = store
        .get_worktree(session.worktree_id)
        .await
        .unwrap()
        .expect("missing worktree");

    let mut next = HashSet::new();
    next.insert(worktree.id);
    state
        .update_worktree_vcs_activity(&HashSet::new(), &next)
        .await;
    ctx_http::daemon::git_status::refresh_worktree_vcs_summary(state.clone(), worktree.clone())
        .await
        .unwrap();

    let resp = client
        .post(format!("{base}/api/tasks/{}/archive", task.id.0))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());

    let ws_url = format!("{base}/api/workspaces/{}/stream", ws.id.0).replace("http://", "ws://");
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
            "replay": {
                "mode": "auto",
            },
        }],
        "include_active_heads": true,
    })
    .to_string();
    socket
        .send(WsMessage::Text(subscribe.into()))
        .await
        .unwrap();

    let first = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let WsMessage::Text(first_text) = first else {
        panic!("expected text frame after subscribe");
    };
    let first_message: ctx_core::models::WorkspaceActiveSnapshotStreamMessage =
        serde_json::from_str(&first_text).unwrap();
    let ctx_core::models::WorkspaceActiveSnapshotStreamMessage::Snapshot {
        active_snapshot, ..
    } = first_message
    else {
        panic!("expected initial snapshot after subscribe");
    };

    assert!(
        active_snapshot.active.tasks.is_empty(),
        "archived explicit session should not reappear in active task snapshot"
    );
    let snapshot = active_snapshot
        .worktree_vcs_snapshots
        .into_iter()
        .find(|candidate| candidate.worktree_id == worktree.id)
        .expect("expected explicit archived session worktree vcs in initial snapshot");
    assert_eq!(snapshot.freshness, WorktreeVcsFreshness::Fresh);
}

#[tokio::test]
async fn workspace_stream_initial_snapshot_excludes_secondary_worktree_vcs_for_active_task_by_default(
) {
    let (repo, _data_dir, state, server) = setup_git().await;
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

    let task = create_task_with_primary_worktree(
        client,
        &state,
        base,
        ws.id,
        repo.path(),
        "multi-worktree",
    )
    .await;
    let primary_session = create_primary_worktree_session(client, base, task.id).await;
    let primary_store = state.store_for_session(primary_session.id).await.unwrap();
    let primary_worktree = primary_store
        .get_worktree(primary_session.worktree_id)
        .await
        .unwrap()
        .expect("missing primary worktree");
    let secondary_worktree = insert_worktree(&state, ws.id, task.id, repo.path()).await;
    let secondary_session = create_child_worktree_session_with_request(
        client,
        base,
        task.id,
        json!({
            "provider_id": "fake",
            "model_id": "fake-model",
            "worktree_id": secondary_worktree.id.0.to_string(),
            "parent_session_id": primary_session.id.0.to_string(),
            "relationship": "secondary",
        }),
    )
    .await;
    assert_eq!(
        secondary_session.worktree_id, secondary_worktree.id,
        "secondary session must use the non-primary worktree for this test"
    );
    assert_ne!(
        primary_worktree.id, secondary_worktree.id,
        "primary and secondary worktrees must be distinct for this test"
    );
    let secondary_worktree = primary_store
        .get_worktree(secondary_session.worktree_id)
        .await
        .unwrap()
        .expect("missing secondary worktree");

    let mut next = HashSet::new();
    next.insert(primary_worktree.id);
    next.insert(secondary_worktree.id);
    state
        .update_worktree_vcs_activity(&HashSet::new(), &next)
        .await;
    ctx_http::daemon::git_status::refresh_worktree_vcs_summary(
        state.clone(),
        primary_worktree.clone(),
    )
    .await
    .unwrap();
    ctx_http::daemon::git_status::refresh_worktree_vcs_summary(
        state.clone(),
        secondary_worktree.clone(),
    )
    .await
    .unwrap();

    let seeded_snapshot = state
        .workspaces
        .workspace_active_snapshot
        .active_snapshot(ws.id, 10)
        .await;
    let seeded_ids: HashSet<_> = seeded_snapshot
        .worktree_vcs_snapshots
        .iter()
        .map(|snapshot| snapshot.worktree_id)
        .collect();
    assert!(seeded_ids.contains(&primary_worktree.id));
    assert!(seeded_ids.contains(&secondary_worktree.id));

    let ws_url = format!("{base}/api/workspaces/{}/stream", ws.id.0).replace("http://", "ws://");
    let (mut socket, _) = connect_async(&ws_url).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    let subscribe = json!({
        "type": "subscribe",
        "scope": "active",
        "include_active_heads": true,
    })
    .to_string();
    socket
        .send(WsMessage::Text(subscribe.into()))
        .await
        .unwrap();

    let first = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let WsMessage::Text(first_text) = first else {
        panic!("expected text frame after subscribe");
    };
    let first_message: ctx_core::models::WorkspaceActiveSnapshotStreamMessage =
        serde_json::from_str(&first_text).unwrap();
    let ctx_core::models::WorkspaceActiveSnapshotStreamMessage::Snapshot {
        active_snapshot, ..
    } = first_message
    else {
        panic!("expected initial snapshot after subscribe");
    };

    assert_eq!(active_snapshot.active.tasks.len(), 1);
    let streamed_ids: HashSet<_> = active_snapshot
        .worktree_vcs_snapshots
        .iter()
        .map(|snapshot| snapshot.worktree_id)
        .collect();
    assert!(streamed_ids.contains(&primary_worktree.id));
    assert!(!streamed_ids.contains(&secondary_worktree.id));
}

#[tokio::test]
async fn workspace_stream_active_subscribe_includes_secondary_worktree_vcs_when_vcs_open_session_is_requested(
) {
    let (repo, _data_dir, state, server) = setup_git().await;
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

    let task = create_task_with_primary_worktree(
        client,
        &state,
        base,
        ws.id,
        repo.path(),
        "multi-worktree-reconnect",
    )
    .await;
    let primary_session = create_primary_worktree_session(client, base, task.id).await;
    let primary_store = state.store_for_session(primary_session.id).await.unwrap();
    let primary_worktree = primary_store
        .get_worktree(primary_session.worktree_id)
        .await
        .unwrap()
        .expect("missing primary worktree");
    let secondary_worktree = insert_worktree(&state, ws.id, task.id, repo.path()).await;
    let secondary_session = create_child_worktree_session_with_request(
        client,
        base,
        task.id,
        json!({
            "provider_id": "fake",
            "model_id": "fake-model",
            "worktree_id": secondary_worktree.id.0.to_string(),
            "parent_session_id": primary_session.id.0.to_string(),
            "relationship": "secondary",
        }),
    )
    .await;
    assert_ne!(
        primary_worktree.id, secondary_worktree.id,
        "primary and secondary worktrees must be distinct for this test"
    );
    let secondary_worktree = primary_store
        .get_worktree(secondary_session.worktree_id)
        .await
        .unwrap()
        .expect("missing secondary worktree");

    let mut next = HashSet::new();
    next.insert(primary_worktree.id);
    next.insert(secondary_worktree.id);
    state
        .update_worktree_vcs_activity(&HashSet::new(), &next)
        .await;
    ctx_http::daemon::git_status::refresh_worktree_vcs_summary(
        state.clone(),
        primary_worktree.clone(),
    )
    .await
    .unwrap();
    ctx_http::daemon::git_status::refresh_worktree_vcs_summary(
        state.clone(),
        secondary_worktree.clone(),
    )
    .await
    .unwrap();

    let ws_url = format!("{base}/api/workspaces/{}/stream", ws.id.0).replace("http://", "ws://");
    let (mut socket, _) = connect_async(&ws_url).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    let subscribe = json!({
        "type": "subscribe",
        "scope": "active",
        "vcs_open_session_ids": [secondary_session.id],
        "include_active_heads": true,
    })
    .to_string();
    socket
        .send(WsMessage::Text(subscribe.into()))
        .await
        .unwrap();

    let _ = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    ctx_http::daemon::git_status::emit_worktree_vcs_snapshot_for_worktree(
        &state,
        &secondary_worktree,
        true,
    )
    .await
    .unwrap();

    let mut saw_secondary_publish = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let wait = remaining.min(Duration::from_millis(250));
        let next = tokio::time::timeout(wait, socket.next()).await;
        if let Ok(Some(Ok(WsMessage::Text(txt)))) = next {
            let Ok(message) = serde_json::from_str::<
                ctx_core::models::WorkspaceActiveSnapshotStreamMessage,
            >(&txt) else {
                continue;
            };
            let Some(snapshot) = worktree_vcs_snapshot_from_message(message, secondary_worktree.id)
            else {
                continue;
            };
            if snapshot.worktree_id == secondary_worktree.id {
                saw_secondary_publish = true;
                break;
            }
        }
    }

    assert!(
        saw_secondary_publish,
        "expected active-scope subscribe with vcs_open_session_ids to keep secondary worktree vcs publishable"
    );
}

#[tokio::test]
async fn mobile_secure_workspace_stream_active_subscribe_includes_secondary_worktree_vcs_when_vcs_open_session_is_requested(
) {
    let (repo, _data_dir, state, server) = setup_git().await;
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

    let task = create_task_with_primary_worktree(
        client,
        &state,
        base,
        ws.id,
        repo.path(),
        "secure-multi-worktree",
    )
    .await;
    let primary_session = create_primary_worktree_session(client, base, task.id).await;
    let primary_store = state.store_for_session(primary_session.id).await.unwrap();
    let primary_worktree = primary_store
        .get_worktree(primary_session.worktree_id)
        .await
        .unwrap()
        .expect("missing primary worktree");
    let secondary_worktree = insert_worktree(&state, ws.id, task.id, repo.path()).await;
    let secondary_session = create_child_worktree_session_with_request(
        client,
        base,
        task.id,
        json!({
            "provider_id": "fake",
            "model_id": "fake-model",
            "worktree_id": secondary_worktree.id.0.to_string(),
            "parent_session_id": primary_session.id.0.to_string(),
            "relationship": "secondary",
        }),
    )
    .await;
    assert_ne!(
        primary_worktree.id, secondary_worktree.id,
        "primary and secondary worktrees must be distinct for this test"
    );
    let secondary_worktree = primary_store
        .get_worktree(secondary_session.worktree_id)
        .await
        .unwrap()
        .expect("missing secondary worktree");

    let mut next = HashSet::new();
    next.insert(primary_worktree.id);
    next.insert(secondary_worktree.id);
    state
        .update_worktree_vcs_activity(&HashSet::new(), &next)
        .await;
    ctx_http::daemon::git_status::refresh_worktree_vcs_summary(
        state.clone(),
        primary_worktree.clone(),
    )
    .await
    .unwrap();
    ctx_http::daemon::git_status::refresh_worktree_vcs_summary(
        state.clone(),
        secondary_worktree.clone(),
    )
    .await
    .unwrap();

    let (device_id, key) = configure_mobile_secure_access(&state).await;
    let ws_url = build_mobile_secure_ws_url(base, ws.id, &device_id, &key);
    let (mut socket, _) = connect_async(&ws_url).await.unwrap();
    let ready =
        recv_secure_workspace_message(&mut socket, &key, &device_id, Duration::from_secs(2))
            .await
            .expect("expected secure ready frame");
    assert!(matches!(
        ready,
        WorkspaceActiveSnapshotStreamMessage::Event { event, .. }
            if matches!(event.as_ref(), WorkspaceActiveSnapshotEvent::Ready { .. })
    ));

    send_secure_workspace_subscribe(
        &mut socket,
        &key,
        &device_id,
        1,
        json!({
            "type": "subscribe",
            "scope": "active",
            "vcs_open_session_ids": [secondary_session.id],
            "include_active_heads": true,
        }),
    )
    .await;

    let _ = recv_secure_workspace_message(&mut socket, &key, &device_id, Duration::from_secs(2))
        .await
        .expect("expected secure snapshot after subscribe");

    ctx_http::daemon::git_status::emit_worktree_vcs_snapshot_for_worktree(
        &state,
        &secondary_worktree,
        true,
    )
    .await
    .unwrap();

    let mut saw_secondary_publish = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let wait = remaining.min(Duration::from_millis(250));
        let Some(message) =
            recv_secure_workspace_message(&mut socket, &key, &device_id, wait).await
        else {
            continue;
        };
        let Some(snapshot) = worktree_vcs_snapshot_from_message(message, secondary_worktree.id)
        else {
            continue;
        };
        if snapshot.worktree_id == secondary_worktree.id {
            saw_secondary_publish = true;
            break;
        }
    }

    assert!(
        saw_secondary_publish,
        "expected secure active-scope subscribe with vcs_open_session_ids to keep secondary worktree vcs publishable"
    );
}

#[tokio::test]
async fn workspace_stream_emits_worktree_vcs_snapshot_on_activation() {
    let (repo, _data_dir, state, server) = setup_git().await;
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

    let task =
        create_task_with_primary_worktree(client, &state, base, ws.id, repo.path(), "vcs-snapshot")
            .await;

    let session = create_primary_worktree_session(client, base, task.id).await;

    let ws_url = format!("{base}/api/workspaces/{}/stream", ws.id.0).replace("http://", "ws://");
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
            "replay": {
                "mode": "auto",
            },
        }],
    })
    .to_string();
    socket
        .send(WsMessage::Text(subscribe.into()))
        .await
        .unwrap();

    let worktree_id = session.worktree_id.0.to_string();
    let mut saw_snapshot = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        let wait = remaining.min(Duration::from_millis(250));
        let next = tokio::time::timeout(wait, socket.next()).await;
        if let Ok(Some(Ok(WsMessage::Text(txt)))) = next {
            let value: Value = serde_json::from_str(&txt).unwrap();
            let event = match value.get("event") {
                Some(event) => event,
                None => continue,
            };
            if event.get("type").and_then(|value| value.as_str()) != Some("worktree_vcs_snapshot") {
                continue;
            }
            let snapshot = event.get("snapshot").expect("missing snapshot");
            let event_worktree_id = snapshot.get("worktree_id").and_then(|value| value.as_str());
            if event_worktree_id != Some(worktree_id.as_str()) {
                continue;
            }
            assert!(snapshot
                .get("compute_state")
                .and_then(|value| value.as_str())
                .is_some());
            assert!(snapshot.get("summary").is_some());
            saw_snapshot = true;
            break;
        }
    }

    assert!(saw_snapshot, "expected worktree_vcs_snapshot on activation");
}

#[tokio::test]
async fn workspace_stream_emits_gap_on_large_replay() {
    let (repo, _data_dir, state, server) = setup().await;
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

    let task =
        create_task_with_primary_worktree(client, &state, base, ws.id, repo.path(), "gap").await;

    let session = create_primary_worktree_session(client, base, task.id).await;
    state.sessions.remember_session_meta(&session).await;
    let store = state.store_for_task(task.id).await.unwrap();
    let sessions = store.list_sessions_for_task(task.id).await.unwrap();
    assert!(
        sessions.iter().any(|stored| stored.id == session.id),
        "expected session to be stored"
    );

    for _ in 0..2105 {
        let event = store
            .append_session_event(
                session.id,
                None,
                None,
                SessionEventType::Notice,
                json!({"msg":"spam"}),
            )
            .await
            .unwrap();
        state.publish_event(event).await;
    }

    let ws_url = format!("{base}/api/workspaces/{}/stream", ws.id.0).replace("http://", "ws://");
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
            "replay": {
                "mode": "resume",
                "after_seq": 1,
            },
        }],
    })
    .to_string();
    socket
        .send(WsMessage::Text(subscribe.into()))
        .await
        .unwrap();

    let mut seen_gap = false;
    let mut seen_seed_after_gap = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        let wait = remaining.min(Duration::from_millis(250));
        let next = tokio::time::timeout(wait, socket.next()).await;
        if let Ok(Some(Ok(WsMessage::Text(txt)))) = next {
            if let Ok(ctx_core::models::WorkspaceActiveSnapshotStreamMessage::Event {
                event, ..
            }) =
                serde_json::from_str::<ctx_core::models::WorkspaceActiveSnapshotStreamMessage>(&txt)
            {
                match event.as_ref() {
                    ctx_core::models::WorkspaceActiveSnapshotEvent::SessionGap {
                        session_id,
                        ..
                    } if *session_id == session.id => {
                        seen_gap = true;
                    }
                    ctx_core::models::WorkspaceActiveSnapshotEvent::SessionHeadSeed {
                        head,
                        ..
                    } if seen_gap && head.session.id == session.id => {
                        seen_seed_after_gap = true;
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    assert!(seen_gap, "expected session_gap for large replay");
    assert!(
        seen_seed_after_gap,
        "expected session_head_seed after session_gap for large replay"
    );
}

#[tokio::test]
async fn workspace_active_snapshot_stream_pushes_updates() {
    let (repo, _data_dir, state, server) = setup().await;
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

    let ws_url = format!("{base}/api/workspaces/{}/stream", ws.id.0).replace("http://", "ws://");
    let (mut socket, _) = connect_async(&ws_url).await.unwrap();
    let ready_msg = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    if let WsMessage::Text(txt) = ready_msg {
        let message: ctx_core::models::WorkspaceActiveSnapshotStreamMessage =
            serde_json::from_str(&txt).unwrap();
        match message {
            ctx_core::models::WorkspaceActiveSnapshotStreamMessage::Event { event, .. } => {
                match event.as_ref() {
                    ctx_core::models::WorkspaceActiveSnapshotEvent::Ready { .. } => {}
                    other => panic!("expected ready, got {other:?}"),
                }
            }
            other => panic!("expected ready, got {other:?}"),
        }
    } else {
        panic!("expected ready text frame");
    }

    let task =
        create_task_with_primary_worktree(client, &state, base, ws.id, repo.path(), "live").await;

    let _session = create_primary_worktree_session(client, base, task.id).await;

    let mut saw_upsert = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        let wait = remaining.min(Duration::from_millis(250));
        let next = tokio::time::timeout(wait, socket.next()).await;
        if let Ok(Some(Ok(WsMessage::Text(txt)))) = next {
            if let Ok(ctx_core::models::WorkspaceActiveSnapshotStreamMessage::Event {
                event, ..
            }) =
                serde_json::from_str::<ctx_core::models::WorkspaceActiveSnapshotStreamMessage>(&txt)
            {
                if let ctx_core::models::WorkspaceActiveSnapshotEvent::ActiveTaskUpsert {
                    task: summary,
                    ..
                } = event.as_ref()
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
}

#[tokio::test]
async fn workspace_stream_session_updates_emit_task_delta_without_full_active_task_upsert() {
    let (repo, _data_dir, state, server) = setup().await;
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

    let task =
        create_task_with_primary_worktree(client, &state, base, ws.id, repo.path(), "delta").await;
    let session = create_primary_worktree_session(client, base, task.id).await;
    state.sessions.remember_session_meta(&session).await;

    let ws_url = format!("{base}/api/workspaces/{}/stream", ws.id.0).replace("http://", "ws://");
    let (mut socket, _) = connect_async(&ws_url).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    let subscribe = json!({
        "type": "subscribe",
        "scope": "active",
        "include_active_heads": true,
    })
    .to_string();
    socket
        .send(WsMessage::Text(subscribe.into()))
        .await
        .unwrap();

    let mut saw_snapshot = false;
    let snapshot_deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while tokio::time::Instant::now() < snapshot_deadline {
        let remaining = snapshot_deadline.saturating_duration_since(tokio::time::Instant::now());
        let wait = remaining.min(Duration::from_millis(250));
        let next = tokio::time::timeout(wait, socket.next()).await;
        if let Ok(Some(Ok(WsMessage::Text(txt)))) = next {
            if let Ok(WorkspaceActiveSnapshotStreamMessage::Snapshot {
                active_snapshot, ..
            }) = serde_json::from_str::<WorkspaceActiveSnapshotStreamMessage>(&txt)
            {
                if active_snapshot
                    .active
                    .tasks
                    .iter()
                    .any(|summary| summary.task.id == task.id)
                {
                    saw_snapshot = true;
                    break;
                }
            }
        }
    }
    assert!(
        saw_snapshot,
        "expected initial active snapshot after subscribe"
    );

    let store = state.store_for_task(task.id).await.unwrap();
    let event = store
        .append_session_event(
            session.id,
            None,
            Some(TurnId::new()),
            SessionEventType::UserMessage,
            json!({
                "message_id": uuid::Uuid::new_v4().to_string(),
                "content": "hello from test",
            }),
        )
        .await
        .unwrap();
    state.publish_event(event).await;

    let mut saw_summary_delta = false;
    let mut saw_task_delta = false;
    let mut saw_upsert = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let wait = remaining.min(Duration::from_millis(250));
        let next = tokio::time::timeout(wait, socket.next()).await;
        if let Ok(Some(Ok(WsMessage::Text(txt)))) = next {
            if let Ok(WorkspaceActiveSnapshotStreamMessage::Event { event, .. }) =
                serde_json::from_str::<WorkspaceActiveSnapshotStreamMessage>(&txt)
            {
                match event.as_ref() {
                    WorkspaceActiveSnapshotEvent::SessionSummaryDelta { delta, .. }
                        if delta.task_id == task.id && delta.session_id == session.id =>
                    {
                        saw_summary_delta = true;
                    }
                    WorkspaceActiveSnapshotEvent::TaskDelta { delta, .. }
                        if delta.task.id == task.id
                            && matches!(delta.kind, ctx_core::models::TaskDeltaKind::Updated) =>
                    {
                        assert!(
                            delta.task.has_active_session,
                            "session-driven task delta must preserve has_active_session"
                        );
                        saw_task_delta = true;
                    }
                    WorkspaceActiveSnapshotEvent::ActiveTaskUpsert { task: summary, .. }
                        if summary.task.id == task.id =>
                    {
                        saw_upsert = true;
                    }
                    _ => {}
                }
            }
        }
        if saw_summary_delta && saw_task_delta && saw_upsert {
            break;
        }
    }

    assert!(
        saw_summary_delta,
        "expected session_summary_delta for active task session"
    );
    assert!(
        saw_task_delta,
        "expected task_delta update instead of a full task upsert"
    );
    assert!(
        !saw_upsert,
        "session updates should not emit a full active_task_upsert after the roster is already hydrated"
    );
}

#[tokio::test]
async fn workspace_stream_archived_task_upsert_has_no_snapshot_payload() {
    let (repo, _data_dir, state, server) = setup().await;
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

    let task =
        create_task_with_primary_worktree(client, &state, base, ws.id, repo.path(), "archive me")
            .await;

    let _session = create_primary_worktree_session(client, base, task.id).await;

    let ws_url = format!("{base}/api/workspaces/{}/stream", ws.id.0).replace("http://", "ws://");
    let (mut socket, _) = connect_async(&ws_url).await.unwrap();
    let ready_msg = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    if let WsMessage::Text(txt) = ready_msg {
        let message: ctx_core::models::WorkspaceActiveSnapshotStreamMessage =
            serde_json::from_str(&txt).unwrap();
        match message {
            ctx_core::models::WorkspaceActiveSnapshotStreamMessage::Event { event, .. } => {
                match event.as_ref() {
                    ctx_core::models::WorkspaceActiveSnapshotEvent::Ready { .. } => {}
                    other => panic!("expected ready, got {other:?}"),
                }
            }
            other => panic!("expected ready, got {other:?}"),
        }
    } else {
        panic!("expected ready text frame");
    }

    let subscribe = json!({ "type": "subscribe" }).to_string();
    socket
        .send(WsMessage::Text(subscribe.into()))
        .await
        .unwrap();

    let resp = client
        .post(format!("{base}/api/tasks/{}/archive", task.id.0))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());

    let task_id = task.id.0.to_string();
    let mut archived_event: Option<Value> = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        let wait = remaining.min(Duration::from_millis(250));
        let next = tokio::time::timeout(wait, socket.next()).await;
        if let Ok(Some(Ok(WsMessage::Text(txt)))) = next {
            let value: Value = serde_json::from_str(&txt).unwrap();
            let event = match value.get("event") {
                Some(event) => event,
                None => continue,
            };
            if event.get("type").and_then(|value| value.as_str()) != Some("archived_task_upsert") {
                continue;
            }
            let event_task_id = event
                .get("task")
                .and_then(|value| value.get("task"))
                .and_then(|value| value.get("id"))
                .and_then(|value| value.as_str());
            if event_task_id == Some(task_id.as_str()) {
                archived_event = Some(value);
                break;
            }
        }
    }

    let archived_event = archived_event.expect("expected archived_task_upsert event");
    assert_eq!(
        archived_event.get("type").and_then(|value| value.as_str()),
        Some("event")
    );
    assert!(archived_event.get("active_snapshot").is_none());
    assert!(archived_event.get("active_heads").is_none());
    let event = archived_event.get("event").expect("missing event");
    assert_eq!(
        event.get("type").and_then(|value| value.as_str()),
        Some("archived_task_upsert")
    );
    assert!(event.get("snapshot_rev").is_none());
    assert!(event.get("snapshot").is_none());
}

#[tokio::test]
async fn workspace_active_snapshot_stream_filters_session_head_deltas() {
    let (repo, _data_dir, state, server) = setup().await;
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

    let task =
        create_task_with_primary_worktree(client, &state, base, ws.id, repo.path(), "live").await;

    let session_a = create_primary_worktree_session(client, base, task.id).await;
    let session_b_worktree = insert_worktree(&state, ws.id, task.id, repo.path()).await;
    let session_b = create_child_worktree_session_with_request(
        client,
        base,
        task.id,
        json!({
            "provider_id": "fake",
            "model_id": "fake-model",
            "worktree_id": session_b_worktree.id.0.to_string(),
            "parent_session_id": session_a.id.0.to_string(),
            "relationship": "secondary",
        }),
    )
    .await;
    assert_ne!(
        session_a.id, session_b.id,
        "filter coverage requires a distinct unsubscribed session"
    );
    let store = state.store_for_task(task.id).await.unwrap();
    let sessions = store.list_sessions_for_task(task.id).await.unwrap();
    assert!(
        sessions.iter().any(|stored| stored.id == session_a.id),
        "expected session a to be stored"
    );
    assert!(
        sessions.iter().any(|stored| stored.id == session_b.id),
        "expected session b to be stored"
    );

    let ws_url = format!("{base}/api/workspaces/{}/stream", ws.id.0).replace("http://", "ws://");
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
            "replay": {
                "mode": "resume",
                "after_seq": 0,
            },
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
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        let wait = remaining.min(Duration::from_millis(250));
        let next = tokio::time::timeout(wait, socket.next()).await;
        if let Ok(Some(Ok(WsMessage::Text(txt)))) = next {
            if let Ok(message) =
                serde_json::from_str::<ctx_core::models::WorkspaceActiveSnapshotStreamMessage>(&txt)
            {
                match message {
                    ctx_core::models::WorkspaceActiveSnapshotStreamMessage::Event {
                        event, ..
                    } => {
                        let ctx_core::models::WorkspaceActiveSnapshotEvent::SessionHeadDelta {
                            delta,
                            ..
                        } = event.as_ref()
                        else {
                            continue;
                        };
                        if delta.session_id == session_a.id {
                            seen_a = true;
                        }
                        if delta.session_id == session_b.id {
                            seen_b = true;
                        }
                    }
                    ctx_core::models::WorkspaceActiveSnapshotStreamMessage::HeadsBatch {
                        deltas,
                        ..
                    } => {
                        for delta in deltas {
                            if delta.session_id == session_a.id {
                                seen_a = true;
                            }
                            if delta.session_id == session_b.id {
                                seen_b = true;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        if seen_a {
            break;
        }
    }

    assert!(seen_a, "expected delta for subscribed session");
    assert!(!seen_b, "did not expect delta for unsubscribed session");
}
