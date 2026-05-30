pub mod attachments;
mod cache_stats;
mod deletion;
mod diff_exec;
mod execution;
mod execution_config;
mod execution_config_route_host;
mod file_completions;
mod harness_container;
mod hydration;
mod merge_queue_config;
mod model_preferences;
mod primary_branch;
mod primary_branch_route_host;
mod prompt_bootstrap_config;
mod provider_model_preferences_route;
mod registry;
mod retry;
mod route_config;
mod route_contract;
mod run_archive;
mod runtime;
pub mod stream;
mod task_worktree_host;
pub mod vcs_hooks;
mod workspace_file_completions_route;
mod worktree_cleanup;
mod worktrees;

pub use cache_stats::WorkspaceCacheDebugStats;
pub(in crate::daemon) use cache_stats::{
    workspace_cache_debug_stats_host_from_runtime, WorkspaceCacheDebugStatsHost,
};
pub use deletion::WorkspaceDeleteError;
pub(in crate::daemon) use deletion::{WorkspaceDeletionRuntime, WorkspaceDeletionRuntimeDeps};
pub(in crate::daemon) use diff_exec::{
    diff_worktree_for_session, diff_worktree_summary_for_session,
};
pub use execution::{execution_environment_from_settings, ResolvedExistingWorktreeExecution};
pub use file_completions::{FileCompletionsError, FileCompletionsErrorKind};
pub use harness_container::WorkspaceHarnessContainerError;
pub(in crate::daemon) use hydration::WorkspaceActiveHydrationRuntime;
pub use hydration::{WorkspaceHydrationError, WorkspaceHydrationErrorKind};
pub use model_preferences::{
    WorkspaceProviderModelPreference, WorkspaceProviderModelPreferenceError,
};
pub use retry::retry_global_index_write;
pub(in crate::daemon::workspaces) use route_config::workspace_store_route_error;
pub(in crate::daemon::workspaces) use route_config::WorkspaceRouteError;
pub use run_archive::RunArchiveIngestError;
pub(in crate::daemon) use runtime::WorkspaceActiveCacheRuntime;
pub use stream::{WorkspaceStreamAccessError, WorkspaceStreamRouteAdmission};
pub(in crate::daemon) use task_worktree_host::{TaskWorktreeHost, TaskWorktreeHostParts};
pub use worktree_cleanup::{
    cleanup_task_worktrees_with_host, managed_worktree_root_for_data_root, BranchCleanupErrorMode,
    TaskWorktreeCleanupTarget,
};
