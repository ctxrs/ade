use super::*;

pub(super) struct SessionCursor {
    pub(super) last_sent: SessionReplayCursor,
}

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

#[derive(Clone, Copy)]
pub(super) enum ResolvedWorkspaceActiveSessionReplay {
    Reset,
    Resume {
        after_seq: i64,
        after_projection_rev: i64,
    },
}

pub(super) struct ResolvedWorkspaceActiveSessionSubscription {
    pub(super) session_id: SessionId,
    pub(super) replay: ResolvedWorkspaceActiveSessionReplay,
}

#[derive(Default)]
pub(super) struct WorkspaceActiveSubscriptionState {
    pub(super) active_scope: bool,
    pub(super) explicit_sessions: HashSet<SessionId>,
    pub(super) active_task_sessions: HashMap<TaskId, SessionId>,
    pub(super) active_task_vcs_sessions: HashMap<TaskId, HashSet<SessionId>>,
    pub(super) foreground_task_id: Option<TaskId>,
    pub(super) foreground_session_ids: Option<HashSet<SessionId>>,
}

pub(super) struct ResolvedWorkspaceActiveSubscriptions {
    pub(super) sessions: Vec<ResolvedWorkspaceActiveSessionSubscription>,
    pub(super) worktree_vcs_session_ids: Vec<SessionId>,
    pub(super) state: WorkspaceActiveSubscriptionState,
}

pub(super) async fn send_secure_ws<S>(
    sink: &mut S,
    key: &crate::mobile_e2ee::E2eeKey,
    device_id: &str,
    seq: i64,
    payload: &WorkspaceActiveSnapshotStreamMessage,
) -> Result<(), anyhow::Error>
where
    S: Sink<WsMessage> + Unpin,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    let plaintext = serde_json::to_vec(payload)?;
    let envelope = crate::mobile_e2ee::encrypt(key, device_id, seq, &plaintext)?;
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
