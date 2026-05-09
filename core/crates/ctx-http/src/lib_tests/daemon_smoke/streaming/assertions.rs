use super::*;

type WorkspaceStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

pub(super) async fn wait_for_subscription_seed(
    ws_stream: &mut WorkspaceStream,
    session_id: ctx_core::ids::SessionId,
) {
    let subscribed = tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(Ok(frame)) = ws_stream.next().await {
            if let tokio_tungstenite::tungstenite::Message::Text(txt) = frame {
                let message = decode_workspace_stream_message(&txt);
                if workspace_stream_subscription_seed_received(&message, session_id) {
                    return true;
                }
            }
        }
        false
    })
    .await
    .expect("timed out waiting for workspace stream subscription seed");
    assert!(
        subscribed,
        "workspace stream ended before subscription seed"
    );
}

pub(super) async fn wait_for_done_event(
    ws_stream: &mut WorkspaceStream,
    session_id: ctx_core::ids::SessionId,
) {
    let seen_done = tokio::time::timeout(Duration::from_secs(20), async {
        while let Some(Ok(frame)) = ws_stream.next().await {
            if let tokio_tungstenite::tungstenite::Message::Text(txt) = frame {
                let message = decode_workspace_stream_message(&txt);
                if workspace_stream_message_has_done_event(message, session_id) {
                    return true;
                }
            }
        }
        false
    })
    .await
    .expect("timed out waiting for Done event");
    assert!(seen_done);
}

pub(super) async fn assert_user_message_persisted(
    state: &Arc<AppState>,
    session_id: ctx_core::ids::SessionId,
) {
    let store = state.store_for_session(session_id).await.unwrap();
    let events = store.list_session_events(session_id).await.unwrap();
    assert!(events.iter().any(|event| matches!(
        event.event_type,
        ctx_core::models::SessionEventType::UserMessage
    )));
}

pub(super) async fn assert_task_read_unread_round_trip(
    client: &reqwest::Client,
    base: &str,
    task_id: ctx_core::ids::TaskId,
) {
    let task_after_read: ctx_core::models::Task = client
        .post(format!("{base}/api/tasks/{}/mark_read", task_id.0))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(task_after_read.assistant_seen_at.is_some());

    let task_after_unread: ctx_core::models::Task = client
        .post(format!("{base}/api/tasks/{}/mark_unread", task_id.0))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(task_after_unread.assistant_seen_at.is_none());
}

fn decode_workspace_stream_message(
    txt: &str,
) -> ctx_core::models::WorkspaceActiveSnapshotStreamMessage {
    serde_json::from_str(txt)
        .unwrap_or_else(|err| panic!("failed to decode workspace stream message: {err}; raw={txt}"))
}

fn workspace_stream_subscription_seed_received(
    message: &ctx_core::models::WorkspaceActiveSnapshotStreamMessage,
    session_id: ctx_core::ids::SessionId,
) -> bool {
    match message {
        ctx_core::models::WorkspaceActiveSnapshotStreamMessage::Snapshot { .. } => true,
        ctx_core::models::WorkspaceActiveSnapshotStreamMessage::Event { event, .. } => {
            matches!(
                event.as_ref(),
                ctx_core::models::WorkspaceActiveSnapshotEvent::SessionHeadSeed { head, .. }
                    if head.session.id == session_id
            )
        }
        ctx_core::models::WorkspaceActiveSnapshotStreamMessage::HeadsBatch { deltas, .. } => {
            deltas.iter().any(|delta| delta.session_id == session_id)
        }
        _ => false,
    }
}

fn workspace_stream_message_has_done_event(
    message: ctx_core::models::WorkspaceActiveSnapshotStreamMessage,
    session_id: ctx_core::ids::SessionId,
) -> bool {
    match message {
        ctx_core::models::WorkspaceActiveSnapshotStreamMessage::Event { event, .. } => {
            workspace_active_event_has_done_event(event.as_ref(), session_id)
        }
        ctx_core::models::WorkspaceActiveSnapshotStreamMessage::HeadsBatch { deltas, .. } => {
            deltas.into_iter().any(|delta| {
                delta.session_id == session_id && session_delta_has_done_event(delta.event.as_ref())
            })
        }
        _ => false,
    }
}

fn workspace_active_event_has_done_event(
    event: &ctx_core::models::WorkspaceActiveSnapshotEvent,
    session_id: ctx_core::ids::SessionId,
) -> bool {
    match event {
        ctx_core::models::WorkspaceActiveSnapshotEvent::SessionHeadDelta { delta, .. } => {
            delta.session_id == session_id && session_delta_has_done_event(delta.event.as_ref())
        }
        ctx_core::models::WorkspaceActiveSnapshotEvent::SessionHeadSeed { head, .. } => {
            head.session.id == session_id
                && head.events.iter().any(|event| {
                    matches!(event.event_type, ctx_core::models::SessionEventType::Done)
                })
        }
        _ => false,
    }
}

fn session_delta_has_done_event(event: Option<&ctx_core::models::SessionEvent>) -> bool {
    event
        .map(|event| matches!(event.event_type, ctx_core::models::SessionEventType::Done))
        .unwrap_or(false)
}
