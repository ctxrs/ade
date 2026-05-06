use super::*;
use std::time::Duration;

pub(super) struct SessionCursor {
    pub(super) last_sent: SessionReplayCursor,
}

pub(crate) use crate::daemon::workspaces::stream::{
    ResolvedWorkspaceActiveSessionReplay, ResolvedWorkspaceActiveSubscriptions,
    WorkspaceActiveSubscriptionState,
};

pub(super) fn accept_session_delta(cursor: &mut SessionCursor, delta: &SessionHeadDelta) -> bool {
    if is_transient_session_delta(delta) {
        return true;
    }
    let incoming = SessionReplayCursor::from_delta(delta);
    accept_session_cursor(cursor, incoming)
}

pub(super) fn accept_session_head(cursor: &mut SessionCursor, head: &SessionHeadSnapshot) -> bool {
    accept_session_cursor(cursor, SessionReplayCursor::from_head(head))
}

fn accept_session_cursor(cursor: &mut SessionCursor, incoming: SessionReplayCursor) -> bool {
    if incoming <= cursor.last_sent {
        return false;
    }
    cursor.last_sent = incoming;
    true
}

pub(super) async fn send_secure_ws<S>(
    sink: &mut S,
    key: &ctx_transport_runtime::mobile_e2ee::E2eeKey,
    device_id: &str,
    seq: i64,
    payload: &WorkspaceActiveSnapshotStreamMessage,
) -> Result<(), anyhow::Error>
where
    S: Sink<WsMessage> + Unpin,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    let plaintext = serde_json::to_vec(payload)?;
    let envelope = ctx_transport_runtime::mobile_e2ee::encrypt(key, device_id, seq, &plaintext)?;
    let frame = SecureEnvelope {
        device_id: envelope.device_id,
        seq: envelope.seq,
        nonce: envelope.nonce_b64,
        ciphertext: envelope.ciphertext_b64,
    };
    let text = serde_json::to_string(&frame)?;
    sink.send(WsMessage::Text(text)).await?;
    Ok(())
}

pub(super) fn bump_latest_snapshot_rev(latest: &AtomicI64, rev: i64) {
    let mut current = latest.load(Ordering::Relaxed);
    while rev > current {
        match latest.compare_exchange(current, rev, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(next) => current = next,
        }
    }
}

pub(super) fn event_snapshot_rev(event: &WorkspaceActiveSnapshotEvent) -> Option<i64> {
    match event {
        WorkspaceActiveSnapshotEvent::Ready { snapshot_rev, .. }
        | WorkspaceActiveSnapshotEvent::ActiveTaskUpsert { snapshot_rev, .. }
        | WorkspaceActiveSnapshotEvent::ActiveTaskDelete { snapshot_rev, .. }
        | WorkspaceActiveSnapshotEvent::TaskDelta { snapshot_rev, .. }
        | WorkspaceActiveSnapshotEvent::SessionSummary { snapshot_rev, .. }
        | WorkspaceActiveSnapshotEvent::SessionSummaryDelta { snapshot_rev, .. }
        | WorkspaceActiveSnapshotEvent::SessionRemoved { snapshot_rev, .. }
        | WorkspaceActiveSnapshotEvent::SessionHeadDelta { snapshot_rev, .. }
        | WorkspaceActiveSnapshotEvent::SessionHeadSeed { snapshot_rev, .. }
        | WorkspaceActiveSnapshotEvent::SessionGap { snapshot_rev, .. }
        | WorkspaceActiveSnapshotEvent::WorktreeBootstrap { snapshot_rev, .. }
        | WorkspaceActiveSnapshotEvent::WorktreeVcsSnapshot { snapshot_rev, .. } => {
            Some(*snapshot_rev)
        }
        WorkspaceActiveSnapshotEvent::ArchivedTaskUpsert { .. }
        | WorkspaceActiveSnapshotEvent::ArchivedTaskDelete { .. } => None,
    }
}

pub(super) async fn sync_workspace_stream_session_pins<I, J>(
    state: &Arc<AppState>,
    current: I,
    next: J,
) where
    I: IntoIterator<Item = SessionId>,
    J: IntoIterator<Item = SessionId>,
{
    let current = current.into_iter().collect::<HashSet<_>>();
    let next = next.into_iter().collect::<HashSet<_>>();
    for session_id in next.difference(&current) {
        state.attach_session(*session_id).await;
    }
    for session_id in current.difference(&next) {
        state.detach_session(*session_id).await;
    }
}

pub(super) async fn release_workspace_stream_session_pins<I>(state: &Arc<AppState>, current: I)
where
    I: IntoIterator<Item = SessionId>,
{
    for session_id in current {
        state.detach_session(session_id).await;
    }
}

pub(super) const WORKSPACE_STREAM_QUEUE_LIMIT: usize = 256;
pub(super) const WORKSPACE_STREAM_QUEUE_MAX_AGE: Duration = Duration::from_secs(10);
pub(super) const HEAD_BATCH_FLUSH_INTERVAL: Duration = Duration::from_millis(25);
pub(super) const HEAD_BATCH_SESSION_LIMIT: usize = 200;

pub(super) struct StreamSendControl {
    disconnect_after_flush: AtomicBool,
    hydrating: AtomicBool,
}

impl StreamSendControl {
    pub(super) fn new() -> Self {
        Self {
            disconnect_after_flush: AtomicBool::new(false),
            hydrating: AtomicBool::new(false),
        }
    }

    pub(super) fn set_disconnect_after_flush(&self) {
        self.disconnect_after_flush.store(true, Ordering::Relaxed);
    }

    pub(super) fn clear_disconnect_after_flush(&self) {
        self.disconnect_after_flush.store(false, Ordering::Relaxed);
    }

    pub(super) fn should_disconnect_after_flush(&self) -> bool {
        self.disconnect_after_flush.load(Ordering::Relaxed)
    }

    pub(super) fn set_hydrating(&self) {
        self.hydrating.store(true, Ordering::Relaxed);
    }

    pub(super) fn clear_hydrating(&self) {
        self.hydrating.store(false, Ordering::Relaxed);
    }

    pub(super) fn is_hydrating(&self) -> bool {
        self.hydrating.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;

    fn transient_delta(session_id: SessionId, turn_id: TurnId) -> SessionHeadDelta {
        SessionHeadDelta {
            session_id,
            last_event_seq: 5,
            projection_rev: 7,
            state_rev: 0,
            emitted_at_ms: None,
            session: None,
            activity: None,
            event: Some(SessionEvent {
                seq: -1,
                id: SessionEventId::new(),
                session_id,
                run_id: None,
                turn_id: Some(turn_id),
                event_type: SessionEventType::AssistantChunk,
                payload_json: json!({ "content_fragment": "partial" }),
                transient: true,
                created_at: Utc::now(),
            }),
            turn: None,
            message: None,
            tool_summaries: Vec::new(),
        }
    }

    #[test]
    fn transient_delta_is_never_dropped_by_cursor_dedup() {
        let mut cursor = SessionCursor {
            last_sent: SessionReplayCursor {
                last_event_seq: 5,
                projection_rev: 7,
            },
        };

        assert!(accept_session_delta(
            &mut cursor,
            &transient_delta(SessionId::new(), TurnId::new())
        ));
        assert_eq!(
            cursor.last_sent,
            SessionReplayCursor {
                last_event_seq: 5,
                projection_rev: 7,
            }
        );
    }

    #[test]
    fn stale_durable_delta_is_dropped() {
        let mut cursor = SessionCursor {
            last_sent: SessionReplayCursor {
                last_event_seq: 5,
                projection_rev: 7,
            },
        };
        let delta = SessionHeadDelta {
            session_id: SessionId::new(),
            last_event_seq: 5,
            projection_rev: 7,
            state_rev: 0,
            emitted_at_ms: None,
            session: None,
            activity: None,
            event: None,
            turn: None,
            message: None,
            tool_summaries: Vec::new(),
        };

        assert!(!accept_session_delta(&mut cursor, &delta));
    }

    #[test]
    fn newer_durable_delta_advances_cursor() {
        let mut cursor = SessionCursor {
            last_sent: SessionReplayCursor {
                last_event_seq: 5,
                projection_rev: 7,
            },
        };
        let delta = SessionHeadDelta {
            session_id: SessionId::new(),
            last_event_seq: 5,
            projection_rev: 8,
            state_rev: 0,
            emitted_at_ms: None,
            session: None,
            activity: None,
            event: None,
            turn: None,
            message: None,
            tool_summaries: Vec::new(),
        };

        assert!(accept_session_delta(&mut cursor, &delta));
        assert_eq!(
            cursor.last_sent,
            SessionReplayCursor {
                last_event_seq: 5,
                projection_rev: 8,
            }
        );
    }
}
