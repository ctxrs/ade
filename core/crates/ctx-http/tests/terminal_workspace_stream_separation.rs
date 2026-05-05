use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::json;
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};

use ctx_core::models::{
    Session, SessionEventType, Task, TerminalSession, TerminalStatus, Workspace,
    WorkspaceActiveSnapshotEvent, WorkspaceActiveSnapshotStreamMessage,
};
use ctx_http::daemon::AppState;
use ctx_transport_runtime::TerminalServerMessage;

mod common;

const QUEUED_MESSAGES_ENABLED_ENV: &str = "CTX_QUEUED_MESSAGES_ENABLED";

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

fn enable_queued_messages_for_test_binary() {
    static ENABLE: std::sync::Once = std::sync::Once::new();
    ENABLE.call_once(|| std::env::set_var(QUEUED_MESSAGES_ENABLED_ENV, "1"));
}

async fn mint_terminal_stream_path(
    client: &reqwest::Client,
    base: &str,
    terminal: &TerminalSession,
) -> String {
    client
        .post(format!(
            "{base}/api/terminals/{}/stream_token",
            terminal.id.0
        ))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap()["stream_path"]
        .as_str()
        .unwrap()
        .to_string()
}

fn terminal_ws_url(base: &str, stream_path: &str) -> String {
    format!("{base}{stream_path}")
        .replacen("https://", "wss://", 1)
        .replacen("http://", "ws://", 1)
}

async fn read_terminal_status(socket: &mut WsStream) -> (TerminalStatus, Option<i32>) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let wait = remaining.min(Duration::from_millis(250));
        let next = tokio::time::timeout(wait, socket.next()).await;
        if let Ok(Some(Ok(WsMessage::Text(text)))) = next {
            if let Ok(TerminalServerMessage::Status { status, exit_code }) =
                serde_json::from_str(&text)
            {
                return (status, exit_code);
            }
        }
    }
    panic!("terminal status message not received");
}

async fn read_terminal_until_marker_with_timeout(
    socket: &mut WsStream,
    marker: &str,
    timeout: Duration,
) -> String {
    let mut buffer = Vec::new();
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let wait = remaining.min(Duration::from_millis(250));
        let next = tokio::time::timeout(wait, socket.next()).await;
        let msg = match next {
            Ok(Some(Ok(msg))) => msg,
            _ => continue,
        };
        match msg {
            WsMessage::Binary(data) => buffer.extend_from_slice(&data),
            WsMessage::Text(text) => {
                if serde_json::from_str::<TerminalServerMessage>(&text).is_ok() {
                    continue;
                }
                buffer.extend_from_slice(text.as_bytes());
            }
            WsMessage::Close(_) => break,
            _ => {}
        }
        let output = String::from_utf8_lossy(&buffer);
        if output.contains(marker) {
            return output.to_string();
        }
    }
    String::from_utf8_lossy(&buffer).to_string()
}

async fn read_terminal_until_marker(socket: &mut WsStream, marker: &str) -> String {
    read_terminal_until_marker_with_timeout(socket, marker, Duration::from_secs(12)).await
}

async fn wait_for_session_done_events_in_store(
    state: &std::sync::Arc<AppState>,
    session_id: ctx_core::ids::SessionId,
    expected_done_events: usize,
) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while tokio::time::Instant::now() < deadline {
        let store = state.store_for_session(session_id).await.unwrap();
        let events = store.list_session_events(session_id).await.unwrap();
        let done_count = events
            .iter()
            .filter(|event| matches!(event.event_type, SessionEventType::Done))
            .count();
        let saw_error = events
            .iter()
            .any(|event| matches!(event.event_type, SessionEventType::Error));
        if saw_error {
            panic!("unexpected session error while terminal churn was active: {events:#?}");
        }
        if done_count >= expected_done_events {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for {expected_done_events} done events in store");
}

async fn wait_for_session_idle_in_memory(
    state: &std::sync::Arc<AppState>,
    session_id: ctx_core::ids::SessionId,
) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while tokio::time::Instant::now() < deadline {
        if !state.is_running(session_id).await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("timed out waiting for session running flag to clear");
}

async fn assert_workspace_stream_no_gap(
    socket: &mut WsStream,
    session: &Session,
    duration: Duration,
) {
    let deadline = tokio::time::Instant::now() + duration;
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let wait = remaining.min(Duration::from_millis(250));
        match tokio::time::timeout(wait, socket.next()).await {
            Ok(Some(Ok(WsMessage::Text(text)))) => {
                let Ok(message) =
                    serde_json::from_str::<WorkspaceActiveSnapshotStreamMessage>(&text)
                else {
                    continue;
                };
                match message {
                    WorkspaceActiveSnapshotStreamMessage::ResetRequired { .. } => {
                        panic!("unexpected reset_required while terminal churn was active");
                    }
                    WorkspaceActiveSnapshotStreamMessage::Event { event, .. } => {
                        match event.as_ref() {
                            WorkspaceActiveSnapshotEvent::SessionGap { session_id, .. }
                                if *session_id == session.id =>
                            {
                                panic!("unexpected session_gap while terminal churn was active");
                            }
                            WorkspaceActiveSnapshotEvent::SessionHeadDelta { delta, .. }
                                if delta.session_id == session.id => {}
                            _ => {}
                        }
                    }
                    WorkspaceActiveSnapshotStreamMessage::HeadsBatch { .. } => {}
                    WorkspaceActiveSnapshotStreamMessage::Snapshot { .. } => {}
                }
            }
            Ok(Some(Ok(WsMessage::Close(_)))) => {
                panic!("workspace stream closed while terminal churn was active");
            }
            Ok(Some(Ok(_))) => {}
            Ok(Some(Err(err))) => panic!("workspace stream error: {err:?}"),
            Ok(None) => panic!("workspace stream ended while terminal churn was active"),
            Err(_) => {}
        }
    }
}

#[tokio::test]
async fn terminal_disconnect_and_reconnect_do_not_poison_workspace_control_plane() {
    enable_queued_messages_for_test_binary();

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

    let workspace: Workspace = client
        .post(format!("{base}/api/workspaces"))
        .json(&json!({"root_path": repo.path(), "name": "ws"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let task: Task = client
        .post(format!("{base}/api/workspaces/{}/tasks", workspace.id.0))
        .json(&json!({"title": "terminal separation"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let session: Session = common::load_primary_session_http(client, base, &task).await;

    let terminal: TerminalSession = client
        .post(format!(
            "{base}/api/workspaces/{}/terminals",
            workspace.id.0
        ))
        .json(&json!({"cwd": repo.path()}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let workspace_ws_url =
        format!("{base}/api/workspaces/{}/stream", workspace.id.0).replace("http://", "ws://");
    let (mut workspace_socket, _) = connect_async(&workspace_ws_url).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(2), workspace_socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    let subscribe = json!({
        "type": "subscribe",
        "scope": "active",
        "include_active_heads": true,
        "sessions": [{
            "session_id": session.id.0,
            "replay": {
                "mode": "resume",
                "after_seq": 0,
            },
        }],
    })
    .to_string();
    workspace_socket
        .send(WsMessage::Text(subscribe.into()))
        .await
        .unwrap();

    let initial_terminal_ws_url = terminal_ws_url(
        base,
        &mint_terminal_stream_path(client, base, &terminal).await,
    );
    let (mut terminal_socket, _) = connect_async(&initial_terminal_ws_url).await.unwrap();
    let (status, _) = read_terminal_status(&mut terminal_socket).await;
    assert!(matches!(status, TerminalStatus::Running));

    let message_count = 4usize;
    let client_clone = client.clone();
    let base_clone = base.to_string();
    let session_id = session.id;
    let state_clone = state.clone();
    let send_messages = tokio::spawn(async move {
        for index in 0..message_count {
            wait_for_session_idle_in_memory(&state_clone, session_id).await;
            let response = client_clone
                .post(format!(
                    "{base_clone}/api/sessions/{}/messages",
                    session_id.0
                ))
                .json(&json!({
                    "content": format!("terminal separation turn {index} emit-thought")
                }))
                .send()
                .await
                .unwrap();
            let status = response.status();
            let body = tokio::time::timeout(Duration::from_secs(2), response.text())
                .await
                .ok()
                .and_then(Result::ok)
                .unwrap_or_else(|| "<body unavailable>".to_string());
            assert!(
                status.is_success(),
                "message post failed with status {status}: {body}"
            );
            wait_for_session_done_events_in_store(&state_clone, session_id, index + 1).await;
            wait_for_session_idle_in_memory(&state_clone, session_id).await;
        }
    });

    let terminal_start_marker = "CTX_TERM_STREAM_START";
    let terminal_end_marker = "CTX_TERM_STREAM_END";
    let churn_command = json!({
        "type": "input",
        "data": format!(
            "printf '{terminal_start_marker}\\n'; i=1; while [ \"$i\" -le 8000 ]; do printf 'term-%05d-0123456789abcdef0123456789abcdef\\n' \"$i\"; if [ $((i % 400)) -eq 0 ]; then sleep 0.03; fi; i=$((i + 1)); done; printf '{terminal_end_marker}\\n'\n"
        ),
    })
    .to_string();
    terminal_socket
        .send(WsMessage::Text(churn_command.into()))
        .await
        .unwrap();

    let initial_output =
        read_terminal_until_marker(&mut terminal_socket, terminal_start_marker).await;
    assert!(initial_output.contains(terminal_start_marker));

    let _ = terminal_socket.close(None).await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    let list: Vec<TerminalSession> = client
        .get(format!(
            "{base}/api/workspaces/{}/terminals",
            workspace.id.0
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let refreshed = list
        .iter()
        .find(|candidate| candidate.id == terminal.id)
        .expect("terminal should still exist after client disconnect");
    assert!(matches!(refreshed.status, TerminalStatus::Running));

    let reconnect_terminal_ws_url = terminal_ws_url(
        base,
        &mint_terminal_stream_path(client, base, refreshed).await,
    );
    let (mut terminal_socket, _) = connect_async(&reconnect_terminal_ws_url).await.unwrap();
    let (status, _) = read_terminal_status(&mut terminal_socket).await;
    assert!(matches!(status, TerminalStatus::Running));

    let reconnect_output = read_terminal_until_marker_with_timeout(
        &mut terminal_socket,
        terminal_end_marker,
        Duration::from_secs(30),
    )
    .await;
    assert!(
        reconnect_output.contains(terminal_end_marker),
        "reconnected terminal should continue streaming new output"
    );

    send_messages.await.unwrap();
    wait_for_session_done_events_in_store(&state, session.id, message_count).await;
    assert_workspace_stream_no_gap(&mut workspace_socket, &session, Duration::from_secs(5)).await;

    let _ = client
        .delete(format!("{base}/api/terminals/{}", terminal.id.0))
        .send()
        .await;
}
