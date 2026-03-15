#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
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

async fn read_output_until_quiet(socket: &mut WsStream, quiet_for: Duration) -> String {
    let mut buffer: Vec<u8> = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
    loop {
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        let wait = quiet_for.min(deadline.saturating_duration_since(tokio::time::Instant::now()));
        match tokio::time::timeout(wait, socket.next()).await {
            Ok(Some(Ok(WsMessage::Binary(data)))) => buffer.extend_from_slice(&data),
            Ok(Some(Ok(WsMessage::Text(text)))) => {
                if serde_json::from_str::<TerminalServerMessage>(&text).is_ok() {
                    continue;
                }
                buffer.extend_from_slice(text.as_bytes());
            }
            Ok(Some(Ok(WsMessage::Close(_)))) | Ok(None) | Err(_) => break,
            Ok(Some(Ok(_))) => {}
            Ok(Some(Err(_))) => break,
        }
    }
    String::from_utf8_lossy(&buffer).to_string()
}

async fn wait_for_terminal_status(
    client: &reqwest::Client,
    base: &str,
    workspace: &Workspace,
    terminal: &TerminalSession,
    expected: TerminalStatus,
) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    while tokio::time::Instant::now() < deadline {
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
        if list
            .iter()
            .find(|candidate| candidate.id == terminal.id)
            .is_some_and(|candidate| {
                std::mem::discriminant(&candidate.status) == std::mem::discriminant(&expected)
            })
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!(
        "terminal {} did not reach expected status {:?}",
        terminal.id.0, expected
    );
}

#[cfg(unix)]
async fn write_executable_script(path: &Path, contents: &str) {
    tokio::fs::write(path, contents).await.unwrap();
    let mut perms = tokio::fs::metadata(path).await.unwrap().permissions();
    perms.set_mode(0o755);
    tokio::fs::set_permissions(path, perms).await.unwrap();
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
async fn terminal_ws_reconnect_resyncs_bounded_tail_after_churn() {
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

    let start_marker = "CTX_TERM_RESYNC_START";
    let end_marker = "CTX_TERM_RESYNC_END";
    let churn = json!({
        "type": "input",
        "data": format!(
            "printf '{start_marker}\\n'; i=1; while [ \"$i\" -le 8000 ]; do printf 'term-%05d-0123456789abcdef0123456789abcdef\\n' \"$i\"; if [ $((i % 400)) -eq 0 ]; then sleep 0.03; fi; i=$((i + 1)); done; printf '{end_marker}\\n'\n"
        ),
    })
    .to_string();
    socket.send(WsMessage::Text(churn.into())).await.unwrap();

    let initial = read_until_marker(&mut socket, start_marker).await;
    assert!(initial.contains(start_marker));

    let _ = socket.close(None).await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    let bounded_ws_url = format!("{base}/api/terminals/{}/stream?tail=4096", terminal.id.0)
        .replace("http://", "ws://");
    let (mut socket, _) = connect_async(&bounded_ws_url).await.unwrap();
    let (status, _) = read_status(&mut socket).await;
    assert!(matches!(status, TerminalStatus::Running));

    let output = read_until_marker(&mut socket, end_marker).await;
    assert!(
        output.contains(end_marker),
        "reconnected terminal should resync latest bounded tail after churn"
    );

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

#[cfg(unix)]
#[tokio::test]
async fn terminal_ws_reconnect_tail_is_bounded_and_buffer_trimmed() {
    let repo = common::init_git_repo(&[("file.txt", "hello\n")]).await;
    let script_path = repo.path().join("bounded-tail.sh");
    write_executable_script(
        &script_path,
        r#"#!/bin/sh
printf 'CTX_TERM_BOUND_HEAD\n'
i=1
while [ "$i" -le 22000 ]; do
  printf 'fill-%05d-abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ\n' "$i"
  i=$((i + 1))
done
printf 'CTX_TERM_BOUND_TAIL\n'
"#,
    )
    .await;

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
        .json(&json!({"cwd": repo.path(), "shell": script_path}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    wait_for_terminal_status(client, base, &workspace, &terminal, TerminalStatus::Exited).await;

    let handle = state.transport.terminals.get(terminal.id).await.unwrap();
    let output_snapshot = handle.output_snapshot();
    let buffered = String::from_utf8_lossy(&output_snapshot).to_string();
    assert!(
        output_snapshot.len() <= 1024 * 1024,
        "terminal output buffer exceeded 1 MiB cap"
    );
    assert!(
        !buffered.contains("CTX_TERM_BOUND_HEAD"),
        "old terminal output should be trimmed from the bounded reconnect buffer"
    );
    assert!(
        buffered.contains("CTX_TERM_BOUND_TAIL"),
        "latest terminal output should remain available in the reconnect buffer"
    );

    let ws_url = format!("{base}/api/terminals/{}/stream?tail=4096", terminal.id.0)
        .replace("http://", "ws://");
    let (mut socket, _) = connect_async(&ws_url).await.unwrap();

    let (status, exit_code) = read_status(&mut socket).await;
    assert!(matches!(status, TerminalStatus::Exited));
    assert_eq!(exit_code, Some(0));

    let tail = read_output_until_quiet(&mut socket, Duration::from_millis(250)).await;
    assert!(
        tail.len() <= 4096,
        "reconnect tail should respect the requested tail limit"
    );
    assert!(tail.contains("CTX_TERM_BOUND_TAIL"));
    assert!(!tail.contains("CTX_TERM_BOUND_HEAD"));

    let _ = client
        .delete(format!("{base}/api/terminals/{}", terminal.id.0))
        .send()
        .await;
}
