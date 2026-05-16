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
    event_blocks_pending_replay, event_snapshot_rev, filter_partial_delta_for_active_tasks,
    is_foreground_session, is_priority_control_event, primary_session_id_for_active_task_event,
    should_stream_head_delta,
};
pub use read_model::{
    initial_stream_state, load_initial_snapshot_read_model, prepare_subscription_read_model,
    WorkspaceStreamInitialState, WorkspaceStreamSnapshotReadModel,
};
pub use replay::{replay_session_events, ReplayOutcome};
pub use replay_cursor::{
    active_head_cursors_from_snapshot_read_model, active_task_subscription_cursor,
    head_only_snapshot_cursor, plan_resume_replay_cursor, WorkspaceStreamResumeReplayCursorPlan,
};
pub use subscriptions::{
    apply_workspace_stream_subscription_event, plan_workspace_stream_subscription,
    resolve_workspace_active_snapshot_subscriptions, WorkspaceStreamResolvedSession,
    WorkspaceStreamSessionReplay, WorkspaceStreamSubscriptionEventApplication,
    WorkspaceStreamSubscriptionPlan, WorkspaceStreamSubscriptionResolutionError,
};
pub use vcs::{filter_workspace_worktree_ids, refresh_worktree_vcs_for_worktrees};

#[cfg(test)]
mod tests;
