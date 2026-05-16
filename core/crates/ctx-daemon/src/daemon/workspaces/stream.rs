mod read_model;
mod replay;
mod subscriptions;
mod vcs;

pub use read_model::{
    initial_stream_state, load_initial_snapshot_read_model, prepare_subscription_read_model,
    WorkspaceStreamInitialState, WorkspaceStreamSnapshotReadModel,
};
pub use replay::{replay_session_events, ReplayOutcome};
pub use subscriptions::{
    resolve_workspace_active_snapshot_subscriptions, WorkspaceStreamSubscriptionResolutionError,
};
pub use vcs::{filter_workspace_worktree_ids, refresh_worktree_vcs_for_worktrees};

#[cfg(test)]
mod tests;
