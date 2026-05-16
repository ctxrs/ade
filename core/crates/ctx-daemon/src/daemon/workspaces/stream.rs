mod cursor_acceptance;
mod event_routing;
mod read_model;
mod replay;
mod replay_cursor;
mod subscriptions;
mod vcs;

pub use cursor_acceptance::WorkspaceStreamCursorAcceptance;
pub(in crate::daemon) use cursor_acceptance::{
    accept_session_delta_cursor, accept_session_head_cursor, is_session_head_delta_after_cursor,
    is_session_summary_delta_after_cursor, merge_replayed_and_live_subscription_cursors,
};
pub(in crate::daemon) use event_routing::{
    event_blocks_pending_replay, event_snapshot_rev, plan_workspace_stream_event_route,
    primary_session_id_for_active_task_event,
};
#[cfg(test)]
pub(in crate::daemon) use event_routing::{
    filter_partial_delta_for_active_tasks, is_priority_control_event, should_stream_head_delta,
};
pub use event_routing::{
    WorkspaceStreamControlLane, WorkspaceStreamEventRoutePlan, WorkspaceStreamHeadLane,
};
pub use read_model::{
    initial_stream_state, load_initial_snapshot_read_model, prepare_subscription_read_model,
    WorkspaceStreamInitialState, WorkspaceStreamSnapshotReadModel,
};
pub use replay::{
    plan_workspace_stream_replay_program, plan_workspace_stream_replay_program_with_step_hook,
    replay_session_events, ReplayOutcome, WorkspaceStreamReplayProgram, WorkspaceStreamReplayStep,
    WorkspaceStreamReplayStepHook,
};
pub use replay_cursor::active_head_cursors_from_snapshot_read_model;
pub(in crate::daemon) use replay_cursor::active_task_subscription_cursor;
#[cfg(test)]
pub(in crate::daemon) use replay_cursor::{
    head_only_snapshot_cursor, plan_resume_replay_cursor, WorkspaceStreamResumeReplayCursorPlan,
};
pub use subscriptions::{
    apply_workspace_stream_subscription_event, finalize_workspace_stream_subscription_replay,
    plan_workspace_stream_subscription, plan_workspace_stream_subscription_transaction,
    resolve_workspace_active_snapshot_subscriptions, WorkspaceStreamResolvedSession,
    WorkspaceStreamSessionPinChanges, WorkspaceStreamSessionReplay,
    WorkspaceStreamSubscriptionApplyPlan, WorkspaceStreamSubscriptionEventApplication,
    WorkspaceStreamSubscriptionPlan, WorkspaceStreamSubscriptionReplayFinalization,
    WorkspaceStreamSubscriptionResolutionError, WorkspaceStreamSubscriptionTransactionPlan,
};
pub use vcs::{filter_workspace_worktree_ids, refresh_worktree_vcs_for_worktrees};

#[cfg(test)]
mod tests;
