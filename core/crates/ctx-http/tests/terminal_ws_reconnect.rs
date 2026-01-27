use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::json;
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};

use ctx_core::models::{TerminalSession, TerminalStatus, Workspace};
use ctx_http::terminals::TerminalServerMessage;

mod common;

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn read_status(socket: &mut WsStream) -> (TerminalStatus, Option<i32>) {
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

async fn read_until_marker(socket: &mut WsStream, marker: &str) -> String {
    let mut buffer: Vec<u8> = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
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

async fn read_pong(socket: &mut WsStream) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let wait = remaining.min(Duration::from_millis(250));
        let next = tokio::time::timeout(wait, socket.next()).await;
        if let Ok(Some(Ok(WsMessage::Text(text)))) = next {
            if let Ok(TerminalServerMessage::Pong) = serde_json::from_str(&text) {
                return true;
            }
        }
    }
    false
}

#[tokio::test]
async fn terminal_ws_reconnect_sends_status_and_tail() {
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

    let ws_url =
        format!("{base}/api/terminals/{}/stream", terminal.id.0).replace("http://", "ws://");
    let (mut socket, _) = connect_async(&ws_url).await.unwrap();

    let (status, _) = read_status(&mut socket).await;
    assert!(matches!(status, TerminalStatus::Running));

    let marker = "CTX_TERM_RECONNECT";
    let input = json!({
        "type": "input",
        "data": format!("echo {marker}\n"),
    })
    .to_string();
    socket.send(WsMessage::Text(input.into())).await.unwrap();
    let output = read_until_marker(&mut socket, marker).await;
    assert!(output.contains(marker));

    let _ = socket.close(None).await;
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
        .find(|t| t.id == terminal.id)
        .expect("terminal should still exist");
    assert!(matches!(refreshed.status, TerminalStatus::Running));

    let (mut socket, _) = connect_async(&ws_url).await.unwrap();
    let (status, _) = read_status(&mut socket).await;
    assert!(matches!(status, TerminalStatus::Running));

    let tail = read_until_marker(&mut socket, marker).await;
    assert!(tail.contains(marker));

    let _ = client
        .delete(format!("{base}/api/terminals/{}", terminal.id.0))
        .send()
        .await;
}

#[tokio::test]
async fn terminal_ws_keepalive_pong() {
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

    let ws_url =
        format!("{base}/api/terminals/{}/stream", terminal.id.0).replace("http://", "ws://");
    let (mut socket, _) = connect_async(&ws_url).await.unwrap();

    let (status, _) = read_status(&mut socket).await;
    assert!(matches!(status, TerminalStatus::Running));

    let ping = json!({ "type": "ping" }).to_string();
    socket.send(WsMessage::Text(ping.into())).await.unwrap();
    assert!(read_pong(&mut socket).await);

    let marker = "CTX_TERM_PING";
    let input = json!({
        "type": "input",
        "data": format!("echo {marker}\n"),
    })
    .to_string();
    socket.send(WsMessage::Text(input.into())).await.unwrap();
    let output = read_until_marker(&mut socket, marker).await;
    assert!(output.contains(marker));
    assert!(!output.contains("{\"type\":\"ping\"}"));

    let _ = client
        .delete(format!("{base}/api/terminals/{}", terminal.id.0))
        .send()
        .await;
}
