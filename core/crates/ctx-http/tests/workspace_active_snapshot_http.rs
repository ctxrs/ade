use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};

use chrono::Utc;
use ctx_core::ids::{TurnId, WorktreeId};
use ctx_core::models::{SessionEventType, SessionTurn, SessionTurnStatus};
use ctx_http::daemon::AppState;

mod common;

fn git_status_untracked_from_message(
    message: ctx_core::models::WorkspaceActiveSnapshotStreamMessage,
    worktree_id: WorktreeId,
) -> Option<i64> {
    match message {
        ctx_core::models::WorkspaceActiveSnapshotStreamMessage::Snapshot {
            active_snapshot,
            ..
        } => active_snapshot
            .worktree_vcs_snapshots
            .into_iter()
            .find(|snapshot| snapshot.worktree_id == worktree_id)
            .map(|snapshot| snapshot.git_status.untracked),
        ctx_core::models::WorkspaceActiveSnapshotStreamMessage::Event { event, .. } => {
            let ctx_core::models::WorkspaceActiveSnapshotEvent::WorktreeVcsSnapshot {
                snapshot,
                ..
            } = event.as_ref()
            else {
                return None;
            };
            if snapshot.worktree_id != worktree_id {
                None
            } else {
                Some(snapshot.git_status.untracked)
            }
        }
        _ => None,
    }
}

async fn setup() -> (
    tempfile::TempDir,
    tempfile::TempDir,
    Arc<AppState>,
    common::TestServer,
) {
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

    (repo, data_dir, state, server)
}

#[tokio::test]
async fn workspace_active_snapshot_includes_sessions() {
    let (repo, _data_dir, _state, server) = setup().await;
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

    let task_active: ctx_core::models::Task = client
        .post(format!("{base}/api/workspaces/{}/tasks", ws.id.0))
        .json(&json!({"title":"active"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let session: ctx_core::models::Session = client
        .post(format!("{base}/api/tasks/{}/sessions", task_active.id.0))
        .json(&json!({"provider_id":"fake","model_id":"fake-model","initial_prompt":"hello"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

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
async fn workspace_active_snapshot_includes_worktree_vcs_for_active_tasks_only() {
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

    let task_active: ctx_core::models::Task = client
        .post(format!("{base}/api/workspaces/{}/tasks", ws.id.0))
        .json(&json!({"title":"active-task"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let session_active: ctx_core::models::Session = client
        .post(format!("{base}/api/tasks/{}/sessions", task_active.id.0))
        .json(&json!({"provider_id":"fake","model_id":"fake-model"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let task_archived: ctx_core::models::Task = client
        .post(format!("{base}/api/workspaces/{}/tasks", ws.id.0))
        .json(&json!({"title":"archived-task"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let session_archived: ctx_core::models::Session = client
        .post(format!("{base}/api/tasks/{}/sessions", task_archived.id.0))
        .json(&json!({"provider_id":"fake","model_id":"fake-model"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

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

    ctx_http::git_status::emit_worktree_vcs_snapshot_for_worktree(&state, &worktree_active, true)
        .await
        .unwrap();
    ctx_http::git_status::emit_worktree_vcs_snapshot_for_worktree(&state, &worktree_archived, true)
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

    let task: ctx_core::models::Task = client
        .post(format!("{base}/api/workspaces/{}/tasks", ws.id.0))
        .json(&json!({"title":"active-heads"}))
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
    let now = Utc::now();
    store
        .insert_session_turn(SessionTurn {
            turn_id: TurnId::new(),
            session_id: session.id,
            run_id: None,
            user_message_id: None,
            status: SessionTurnStatus::Completed,
            start_seq: Some(1),
            end_seq: Some(2),
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
}

#[tokio::test]
async fn session_snapshot_returns_summary_only() {
    let (repo, _data_dir, _state, server) = setup().await;
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
        .json(&json!({"title":"snapshot"}))
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
    let (repo, _data_dir, _state, server) = setup().await;
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
        .json(&json!({"title":"head"}))
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

    let task: ctx_core::models::Task = client
        .post(format!("{base}/api/workspaces/{}/tasks", ws.id.0))
        .json(&json!({"title":"replay"}))
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

    let task: ctx_core::models::Task = client
        .post(format!("{base}/api/workspaces/{}/tasks", ws.id.0))
        .json(&json!({"title":"tool-replay"}))
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
async fn workspace_stream_emits_git_status_snapshot_on_change() {
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

    let task: ctx_core::models::Task = client
        .post(format!("{base}/api/workspaces/{}/tasks", ws.id.0))
        .json(&json!({"title":"git-status"}))
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

    let task: ctx_core::models::Task = client
        .post(format!("{base}/api/workspaces/{}/tasks", ws.id.0))
        .json(&json!({"title":"git-status-new-subscriber"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let session_one: ctx_core::models::Session = client
        .post(format!("{base}/api/tasks/{}/sessions", task.id.0))
        .json(&json!({"provider_id":"fake","model_id":"fake-model"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    state.remember_session_meta(&session_one).await;
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

    let session_two: ctx_core::models::Session = client
        .post(format!("{base}/api/tasks/{}/sessions", task.id.0))
        .json(&json!({"provider_id":"fake","model_id":"fake-model"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    state.remember_session_meta(&session_two).await;
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
async fn workspace_stream_emits_worktree_vcs_snapshot_on_activation() {
    let (repo, _data_dir, _state, server) = setup().await;
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
        .json(&json!({"title":"vcs-snapshot"}))
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

    let task: ctx_core::models::Task = client
        .post(format!("{base}/api/workspaces/{}/tasks", ws.id.0))
        .json(&json!({"title":"gap"}))
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
            "after_seq": 0,
        }],
    })
    .to_string();
    socket
        .send(WsMessage::Text(subscribe.into()))
        .await
        .unwrap();

    let mut seen_reset = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
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
                if matches!(
                    message,
                    ctx_core::models::WorkspaceActiveSnapshotStreamMessage::Event { ref event, .. }
                        if matches!(
                            event.as_ref(),
                            ctx_core::models::WorkspaceActiveSnapshotEvent::SessionHeadReset { .. }
                        )
                ) {
                    seen_reset = true;
                    break;
                }
            }
        }
    }

    assert!(seen_reset, "expected session_head_reset for large replay");
}

#[tokio::test]
async fn workspace_active_snapshot_stream_pushes_updates() {
    let (repo, _data_dir, _state, server) = setup().await;
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

    let task: ctx_core::models::Task = client
        .post(format!("{base}/api/workspaces/{}/tasks", ws.id.0))
        .json(&json!({"title":"live"}))
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
async fn workspace_stream_archived_task_upsert_has_no_snapshot_payload() {
    let (repo, _data_dir, _state, server) = setup().await;
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
        .json(&json!({"title":"archive me"}))
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

    let task: ctx_core::models::Task = client
        .post(format!("{base}/api/workspaces/{}/tasks", ws.id.0))
        .json(&json!({"title":"live"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let session_a: ctx_core::models::Session = client
        .post(format!("{base}/api/tasks/{}/sessions", task.id.0))
        .json(&json!({"provider_id":"fake","model_id":"fake-model"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let session_b: ctx_core::models::Session = client
        .post(format!("{base}/api/tasks/{}/sessions", task.id.0))
        .json(&json!({"provider_id":"fake","model_id":"fake-model"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
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
