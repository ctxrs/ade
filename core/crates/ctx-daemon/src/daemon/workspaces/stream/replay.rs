use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use ctx_core::ids::{SessionId, WorkspaceId};
use ctx_core::models::{
    WorkspaceActiveSnapshotEvent, WorkspaceActiveSnapshotSessionIntent,
    WorkspaceActiveSnapshotStreamMessage, WorkspaceActiveSnapshotStreamSource,
};
use ctx_workspace_active_snapshot::{
    SessionReplayCursor, WorkspaceSessionReplay, WorkspaceSessionReplayItem,
};

use crate::daemon::DaemonState;

use super::replay_cursor::{
    head_only_snapshot_cursor, plan_resume_replay_cursor, WorkspaceStreamResumeReplayCursorPlan,
};
use super::subscriptions::{WorkspaceStreamResolvedSession, WorkspaceStreamSessionReplay};

pub enum ReplayOutcome {
    Replay { last_sent: SessionReplayCursor },
    ResetRequired,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceStreamReplayProgram {
    pub pending_replay_sessions: HashSet<SessionId>,
    pub steps: Vec<WorkspaceStreamReplayStep>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkspaceStreamReplayStep {
    HeadOnly {
        session_id: SessionId,
        cursor: SessionReplayCursor,
    },
    Replay {
        session_id: SessionId,
        after_seq: i64,
        after_projection_rev: i64,
        replay_cursor: SessionReplayCursor,
    },
    NoReplayRequired {
        session_id: SessionId,
    },
}

#[async_trait::async_trait]
pub trait WorkspaceStreamReplayStepHook {
    type Error;

    async fn before_workspace_stream_replay_step(
        &mut self,
        pending_replay_sessions: &HashSet<SessionId>,
    ) -> Result<(), Self::Error>;

    fn live_subscription_cursor(&self, _session_id: SessionId) -> Option<SessionReplayCursor> {
        None
    }
}

struct NoopWorkspaceStreamReplayStepHook;

#[async_trait::async_trait]
impl WorkspaceStreamReplayStepHook for NoopWorkspaceStreamReplayStepHook {
    type Error = std::convert::Infallible;

    async fn before_workspace_stream_replay_step(
        &mut self,
        _pending_replay_sessions: &HashSet<SessionId>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

const SESSION_REPLAY_HEAD_SEED_LIMIT: u32 = 60;
// Workspace streams render a bounded session head, not an audit-log replay. If a
// subscriber misses more deltas than a recoverable head window, replaying each
// stale delta floods the per-socket head queue and delays fresh foreground
// traffic. Let replay_session_stream turn larger gaps into gap+seed recovery.
const SESSION_REPLAY_DELTA_LIMIT: usize = SESSION_REPLAY_HEAD_SEED_LIMIT as usize;

pub async fn plan_workspace_stream_replay_program(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
    resolved_sessions: &[WorkspaceStreamResolvedSession],
    live_subscriptions: &HashMap<SessionId, SessionReplayCursor>,
    active_head_cursors: &HashMap<SessionId, SessionReplayCursor>,
    include_initial_snapshot: bool,
) -> WorkspaceStreamReplayProgram {
    let mut hook = NoopWorkspaceStreamReplayStepHook;
    plan_workspace_stream_replay_program_with_step_hook(
        state,
        workspace_id,
        resolved_sessions,
        live_subscriptions,
        active_head_cursors,
        include_initial_snapshot,
        &mut hook,
    )
    .await
    .unwrap_or_else(|never| match never {})
}

pub async fn plan_workspace_stream_replay_program_with_step_hook<H>(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
    resolved_sessions: &[WorkspaceStreamResolvedSession],
    live_subscriptions: &HashMap<SessionId, SessionReplayCursor>,
    active_head_cursors: &HashMap<SessionId, SessionReplayCursor>,
    include_initial_snapshot: bool,
    step_hook: &mut H,
) -> Result<WorkspaceStreamReplayProgram, H::Error>
where
    H: WorkspaceStreamReplayStepHook,
{
    let pending_replay_sessions = replay_pending_sessions(resolved_sessions);
    let mut steps = Vec::new();
    for subscription in resolved_sessions {
        step_hook
            .before_workspace_stream_replay_step(&pending_replay_sessions)
            .await?;
        match subscription.intent {
            WorkspaceActiveSnapshotSessionIntent::Head => {
                let session_id = subscription.session_id;
                let live_cursor = step_hook
                    .live_subscription_cursor(session_id)
                    .or_else(|| live_subscriptions.get(&session_id).copied());
                let cursor = head_only_snapshot_cursor(
                    state,
                    workspace_id,
                    session_id,
                    live_cursor,
                    active_head_cursors.get(&session_id).copied(),
                    include_initial_snapshot,
                )
                .await;
                steps.push(WorkspaceStreamReplayStep::HeadOnly { session_id, cursor });
            }
            WorkspaceActiveSnapshotSessionIntent::Replay => {
                let WorkspaceStreamSessionReplay::Resume {
                    after_seq,
                    after_projection_rev,
                } = subscription.replay
                else {
                    continue;
                };
                let session_id = subscription.session_id;
                let live_cursor = step_hook
                    .live_subscription_cursor(session_id)
                    .or_else(|| live_subscriptions.get(&session_id).copied());
                match plan_resume_replay_cursor(live_cursor, after_seq, after_projection_rev) {
                    WorkspaceStreamResumeReplayCursorPlan::Replay { cursor } => {
                        steps.push(WorkspaceStreamReplayStep::Replay {
                            session_id,
                            after_seq,
                            after_projection_rev,
                            replay_cursor: cursor,
                        });
                    }
                    WorkspaceStreamResumeReplayCursorPlan::NoReplayRequired => {
                        steps.push(WorkspaceStreamReplayStep::NoReplayRequired { session_id });
                    }
                }
            }
        }
    }
    Ok(WorkspaceStreamReplayProgram {
        pending_replay_sessions,
        steps,
    })
}

fn replay_pending_sessions(
    resolved_sessions: &[WorkspaceStreamResolvedSession],
) -> HashSet<SessionId> {
    resolved_sessions
        .iter()
        .filter_map(|subscription| {
            if subscription.intent == WorkspaceActiveSnapshotSessionIntent::Replay
                && matches!(
                    subscription.replay,
                    WorkspaceStreamSessionReplay::Resume { .. }
                )
            {
                Some(subscription.session_id)
            } else {
                None
            }
        })
        .collect()
}

pub async fn replay_session_events<F, Fut>(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
    session_id: SessionId,
    after_cursor: SessionReplayCursor,
    list_failpoint: &'static str,
    send_failpoint: Option<&'static str>,
    mut emit: F,
) -> Result<ReplayOutcome, ()>
where
    F: FnMut(WorkspaceActiveSnapshotStreamMessage) -> Fut,
    Fut: std::future::Future<Output = Result<(), ()>>,
{
    let (snapshot_rev, _) =
        crate::daemon::workspaces::load_workspace_active_snapshot_state(state, workspace_id).await;
    if crate::fault_injection::maybe_fail(list_failpoint).is_err() {
        return Ok(ReplayOutcome::ResetRequired);
    }
    let replay = state
        .workspaces
        .workspace_active_snapshot
        .replay_session_stream(
            workspace_id,
            session_id,
            after_cursor.last_event_seq,
            after_cursor.projection_rev,
            SESSION_REPLAY_DELTA_LIMIT,
        )
        .await;
    match replay {
        WorkspaceSessionReplay::Replay {
            mut items,
            mut last_sent,
        } => {
            let saw_gap = items
                .iter()
                .any(|item| matches!(item, WorkspaceSessionReplayItem::Gap { .. }));
            let saw_seed = items
                .iter()
                .any(|item| matches!(item, WorkspaceSessionReplayItem::Seed(_)));
            if saw_gap && !saw_seed {
                let store = state.store_for_session(session_id).await.map_err(|_| ())?;
                if let Ok(Some(head)) = store
                    .get_session_head_snapshot(session_id, SESSION_REPLAY_HEAD_SEED_LIMIT, true)
                    .await
                {
                    state
                        .workspaces
                        .workspace_active_snapshot
                        .update_session_head(head.clone())
                        .await;
                    last_sent = SessionReplayCursor::from_head(&head);
                    items.push(WorkspaceSessionReplayItem::Seed(Box::new(head)));
                }
            }
            let seeded_session_ids = items
                .iter()
                .filter_map(|item| match item {
                    WorkspaceSessionReplayItem::Seed(head) => Some(head.session.id),
                    _ => None,
                })
                .collect::<Vec<_>>();
            for item in items {
                if matches!(item, WorkspaceSessionReplayItem::Delta(_)) {
                    if let Some(label) = send_failpoint {
                        crate::fault_injection::maybe_fail(label).map_err(|_| ())?;
                    }
                }
                let event = match item {
                    WorkspaceSessionReplayItem::Delta(delta) => {
                        WorkspaceActiveSnapshotEvent::SessionHeadDelta {
                            workspace_id,
                            snapshot_rev,
                            delta,
                        }
                    }
                    WorkspaceSessionReplayItem::Gap {
                        session_id,
                        after_seq,
                        reason,
                    } => WorkspaceActiveSnapshotEvent::SessionGap {
                        workspace_id,
                        snapshot_rev,
                        session_id,
                        after_seq,
                        reason,
                        seed_follows: seeded_session_ids.contains(&session_id),
                    },
                    WorkspaceSessionReplayItem::Seed(head) => {
                        WorkspaceActiveSnapshotEvent::SessionHeadSeed {
                            workspace_id,
                            snapshot_rev,
                            head,
                        }
                    }
                };
                emit(WorkspaceActiveSnapshotStreamMessage::Event {
                    rev: 0,
                    event: Box::new(event),
                    stream_source: Some(WorkspaceActiveSnapshotStreamSource::Replay),
                })
                .await?;
            }
            Ok(ReplayOutcome::Replay { last_sent })
        }
        WorkspaceSessionReplay::ResetRequired => Ok(ReplayOutcome::ResetRequired),
    }
}
