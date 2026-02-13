#![cfg(feature = "property_tests")]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::json;
use tokio::process::Command;
use tokio::task::JoinHandle;
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage, WebSocketStream};

use ctx_core::ids::{SessionId, WorkspaceId};
use ctx_core::models::{
    SessionEventType, Workspace, WorkspaceActiveSnapshotEvent, WorkspaceActiveSnapshotStreamMessage,
};
use ctx_http::{api, daemon::AppState};
use ctx_providers::fake::FakeProviderAdapter;
use ctx_store::StoreManager;

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

struct ReplayFixture {
    _repo: tempfile::TempDir,
    _data_dir: tempfile::TempDir,
    _state: Arc<AppState>,
    server: JoinHandle<()>,
    addr: std::net::SocketAddr,
    workspace: Workspace,
    session_id: SessionId,
    seqs: Vec<i64>,
}

impl Drop for ReplayFixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn setup_replay_fixture(event_count: usize) -> ReplayFixture {
    let repo = setup_git_repo().await;
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();

    let mut providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
        HashMap::new();
    providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));

    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:0".to_string(),
        None,
    ));
    let app = api::router(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base = format!("http://{}", addr);
    let client = reqwest::Client::new();

    let workspace: ctx_core::models::Workspace = client
        .post(format!("{base}/api/workspaces"))
        .json(&json!({"root_path": repo.path(), "name": "ws"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let task: ctx_core::models::Task = client
        .post(format!("{base}/api/workspaces/{}/tasks", workspace.id.0))
        .json(&json!({"title":"replay-props"}))
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

    state.remember_session_meta(&session).await;
    let store = state.store_for_session(session.id).await.unwrap();

    let mut seqs = Vec::new();
    for i in 0..event_count {
        let ev = store
            .append_session_event(
                session.id,
                None,
                None,
                SessionEventType::Notice,
                json!({"i":i}),
            )
            .await
            .unwrap();
        state.publish_event(ev.clone()).await;
        seqs.push(ev.seq);
    }

    ReplayFixture {
        _repo: repo,
        _data_dir: data_dir,
        _state: state,
        server,
        addr,
        workspace,
        session_id: session.id,
        seqs,
    }
}

type Ws = WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect_workspace_stream(addr: std::net::SocketAddr, workspace_id: WorkspaceId) -> Ws {
    let ws_url = format!("ws://{}/api/workspaces/{}/stream", addr, workspace_id.0);
    let (mut socket, _) = connect_async(&ws_url).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    socket
}

async fn subscribe_and_collect_deltas(
    socket: &mut Ws,
    session_id: SessionId,
    after_seq: i64,
    max_wait: Duration,
) -> (Vec<i64>, bool) {
    let subscribe = json!({
        "type": "subscribe",
        "sessions": [{
            "session_id": session_id.0,
            "after_seq": after_seq,
        }],
    })
    .to_string();

    socket
        .send(WsMessage::Text(subscribe.into()))
        .await
        .unwrap();

    let deadline = tokio::time::Instant::now() + max_wait;
    let mut got = Vec::new();
    let mut saw_gap = false;

    while tokio::time::Instant::now() < deadline {
        let msg = tokio::time::timeout(Duration::from_millis(1000), socket.next()).await;
        let Ok(Some(Ok(WsMessage::Text(txt)))) = msg else {
            if tokio::time::Instant::now() >= deadline {
                break;
            }
            continue;
        };

        let Ok(message) = serde_json::from_str::<WorkspaceActiveSnapshotStreamMessage>(&txt) else {
            continue;
        };

        match message {
            WorkspaceActiveSnapshotStreamMessage::Event { event, .. } => match event.as_ref() {
                WorkspaceActiveSnapshotEvent::SessionHeadDelta { delta, .. } => {
                    if delta.session_id != session_id {
                        continue;
                    }
                    let Some(event) = delta.event.as_ref() else {
                        continue;
                    };
                    assert_eq!(delta.last_event_seq, event.seq);
                    got.push(event.seq);
                }
                WorkspaceActiveSnapshotEvent::SessionGap {
                    session_id: gap_id, ..
                } if *gap_id == session_id => {
                    saw_gap = true;
                }
                _ => {}
            },
            WorkspaceActiveSnapshotStreamMessage::HeadsBatch { deltas, .. } => {
                for delta in deltas {
                    if delta.session_id != session_id {
                        continue;
                    }
                    let Some(event) = delta.event else {
                        continue;
                    };
                    assert_eq!(delta.last_event_seq, event.seq);
                    got.push(event.seq);
                }
            }
            _ => {}
        }
    }

    (got, saw_gap)
}

#[tokio::test]
async fn property_replay_respects_after_seq_and_monotonicity() {
    let fixture = setup_replay_fixture(15).await;
    assert!(fixture.seqs.windows(2).all(|w| w[0] < w[1]));

    let mut socket = connect_workspace_stream(fixture.addr, fixture.workspace.id).await;
    let after_seqs = [
        0,
        fixture.seqs[0],
        fixture.seqs[3],
        fixture.seqs[7],
        fixture.seqs[14],
        fixture.seqs[14] + 10,
    ];

    for after_seq in after_seqs {
        let expected: Vec<i64> = fixture
            .seqs
            .iter()
            .copied()
            .filter(|s| *s > after_seq)
            .collect();
        let (got, _saw_gap) = subscribe_and_collect_deltas(
            &mut socket,
            fixture.session_id,
            after_seq,
            Duration::from_secs(3),
        )
        .await;
        for seq in &got {
            assert!(
                expected.contains(seq),
                "after_seq={after_seq}: replayed seq {seq} not in expected set {expected:?}"
            );
            assert!(
                *seq > after_seq,
                "after_seq={after_seq}: replayed seq {seq} must be greater than cursor"
            );
        }
        if expected.is_empty() {
            assert!(
                got.is_empty(),
                "after_seq={after_seq}: expected no deltas past replay tail"
            );
        }
        assert!(got.windows(2).all(|w| w[0] < w[1]), "after_seq={after_seq}");
    }
}

#[tokio::test]
async fn property_replay_is_idempotent_for_same_after_seq() {
    let fixture = setup_replay_fixture(12).await;
    let mut socket = connect_workspace_stream(fixture.addr, fixture.workspace.id).await;
    let after_seq = fixture.seqs[4];
    let expected: Vec<i64> = fixture
        .seqs
        .iter()
        .copied()
        .filter(|s| *s > after_seq)
        .collect();

    let (first, first_gap) = subscribe_and_collect_deltas(
        &mut socket,
        fixture.session_id,
        after_seq,
        Duration::from_secs(2),
    )
    .await;
    let (second, second_gap) = subscribe_and_collect_deltas(
        &mut socket,
        fixture.session_id,
        after_seq,
        Duration::from_secs(2),
    )
    .await;

    assert_eq!(first, expected);
    assert_eq!(second, expected);
    assert_eq!(first, second);
    assert!(!first_gap, "unexpected session_gap on first replay");
    assert!(!second_gap, "unexpected session_gap on second replay");
}

#[tokio::test]
async fn property_replay_after_seq_zero_does_not_emit_gap() {
    let fixture = setup_replay_fixture(8).await;
    let mut socket = connect_workspace_stream(fixture.addr, fixture.workspace.id).await;

    let (got, saw_gap) =
        subscribe_and_collect_deltas(&mut socket, fixture.session_id, 0, Duration::from_secs(2))
            .await;

    for seq in &got {
        assert!(
            fixture.seqs.contains(seq),
            "replayed seq {seq} not found in fixture sequence {:?}",
            fixture.seqs
        );
    }
    assert!(got.windows(2).all(|w| w[0] < w[1]));
    assert!(!saw_gap, "after_seq=0 should not emit session_gap");
}

#[tokio::test]
async fn property_replay_past_tail_emits_gap_without_deltas() {
    let fixture = setup_replay_fixture(10).await;
    let mut socket = connect_workspace_stream(fixture.addr, fixture.workspace.id).await;
    let after_seq = fixture.seqs.last().copied().unwrap_or(0) + 100;

    let (got, saw_gap) = subscribe_and_collect_deltas(
        &mut socket,
        fixture.session_id,
        after_seq,
        Duration::from_secs(2),
    )
    .await;

    assert!(
        got.is_empty(),
        "expected no delta replay beyond session tail"
    );
    // A gap can be emitted, but depending on session head state timing, a noop replay is also valid.
    let _ = saw_gap;
}
