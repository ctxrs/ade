use std::sync::Arc;

use crate::api::sessions::diff_worktree_summary_for_session;
use crate::daemon::AppState;
use crate::vcs_hooks;
use ctx_core::ids::TaskId;
use ctx_core::models::{VcsKind, Workspace, Worktree};
use ctx_fs::vcs;
use ctx_settings_model::ExecutionSettings;
use ctx_workspace_services::worktree_vcs::WorktreeVcsCommitLookupSource;

use super::errors::{
    api_error, internal_api_error, internal_request_or_policy_error, ApiResult, SubagentErrorKind,
};
use ctx_session_service::subagents::SubagentWorktreeSelection;

pub(super) async fn plan_subagent_worktree_creation(
    state: &Arc<AppState>,
    parent_worktree: &Worktree,
    selection: SubagentWorktreeSelection,
) -> ApiResult<Option<(VcsKind, String)>> {
    if selection != SubagentWorktreeSelection::New {
        return Ok(None);
    }

    let source = crate::git_status::HttpWorktreeVcsSource::new(state, parent_worktree);
    let base_commit_sha = source.resolve_commit("HEAD").await.map_err(|error| {
        let msg = error.to_string().to_lowercase();
        if msg.contains("ambiguous argument 'head'")
            || msg.contains("unknown revision or path not in the working tree")
        {
            return api_error(
                SubagentErrorKind::BadRequest,
                "git repo has no commits; create an initial commit before creating a worktree",
            );
        }
        internal_api_error(error)
    })?;
    let vcs = vcs::driver_for_kind(parent_worktree.vcs_kind.clone());
    let dirty_counts = diff_worktree_summary_for_session(state, parent_worktree, &base_commit_sha)
        .await
        .map_err(internal_api_error)?;
    if dirty_counts.file_count > 0
        || dirty_counts.line_additions > 0
        || dirty_counts.line_deletions > 0
    {
        return Err(api_error(
            SubagentErrorKind::BadRequest,
            "Your worktree has uncommitted changes. Before starting new subagents in new worktree mode, you must commit or stash your changes to be explicit about whether subagents should inherit these diffs.",
        ));
    }

    Ok(Some((vcs.kind(), base_commit_sha)))
}

pub(super) async fn create_subagent_worktree(
    state: &Arc<AppState>,
    store: &ctx_store::Store,
    workspace: &Workspace,
    task_id: TaskId,
    base_commit_sha: &str,
    vcs_kind: VcsKind,
    effective: &ExecutionSettings,
) -> ApiResult<Worktree> {
    let worktree_id = ctx_core::ids::WorktreeId::new();
    let branch_name = format!("ctx/{}/{}", task_id.0, worktree_id.0);
    let (wt_path, sandbox_binding) = crate::api::tasks::provision_worktree_for_execution(
        state,
        workspace,
        worktree_id,
        base_commit_sha,
        &branch_name,
        effective,
    )
    .await
    .map_err(internal_request_or_policy_error)?;

    let worktree = Worktree {
        id: worktree_id,
        workspace_id: workspace.id,
        root_path: wt_path.to_string_lossy().to_string(),
        base_commit_sha: base_commit_sha.to_string(),
        git_branch: (vcs_kind == VcsKind::Git).then(|| branch_name.clone()),
        vcs_kind: Some(vcs_kind),
        base_revision: Some(base_commit_sha.to_string()),
        vcs_ref: Some(branch_name),
        created_at: chrono::Utc::now(),
        bootstrap_status: None,
        bootstrap_started_at: None,
        bootstrap_finished_at: None,
        bootstrap_exit_code: None,
        bootstrap_timeout_sec: None,
        bootstrap_error: None,
        bootstrap_log_path: None,
        bootstrap_log_truncated: None,
        bootstrap_command: None,
        bootstrap_script_path: None,
    };

    let worktree = crate::api::tasks::persist_provisioned_worktree(
        state,
        store,
        workspace,
        worktree,
        sandbox_binding,
    )
    .await
    .map_err(internal_api_error)?;
    if let Err(error) =
        vcs_hooks::ensure_task_commit_hook(state, workspace, &worktree, task_id).await
    {
        tracing::warn!(
            task_id = %task_id.0,
            worktree_id = %worktree.id.0,
            "failed to configure vcs hooks for subagent worktree: {error:#}"
        );
    }
    Ok(worktree)
}

pub(super) async fn cleanup_archived_subagent_worktree(
    state: &Arc<AppState>,
    store: &ctx_store::Store,
    parent: &ctx_core::models::Session,
    child: &ctx_core::models::Session,
) -> bool {
    if child.worktree_id == parent.worktree_id {
        return false;
    }

    let mut cleanup_failed = false;
    let sharing_sessions = match store
        .list_all_sessions_for_worktree(child.worktree_id)
        .await
    {
        Ok(sessions) => sessions,
        Err(error) => {
            tracing::warn!(
                parent_session_id = %parent.id.0,
                child_session_id = %child.id.0,
                worktree_id = %child.worktree_id.0,
                "failed to load archived subagent worktree session references: {error:#}"
            );
            return true;
        }
    };
    if sharing_sessions
        .iter()
        .any(|session| session.id != child.id)
    {
        tracing::warn!(
            parent_session_id = %parent.id.0,
            child_session_id = %child.id.0,
            worktree_id = %child.worktree_id.0,
            "archived subagent worktree is still referenced by another session"
        );
        return true;
    }

    let other_tasks = match store
        .count_tasks_for_worktree(child.worktree_id, Some(child.task_id))
        .await
    {
        Ok(count) => count,
        Err(error) => {
            tracing::warn!(
                parent_session_id = %parent.id.0,
                child_session_id = %child.id.0,
                worktree_id = %child.worktree_id.0,
                "failed to load archived subagent worktree task references: {error:#}"
            );
            return true;
        }
    };
    if other_tasks > 0 {
        tracing::warn!(
            parent_session_id = %parent.id.0,
            child_session_id = %child.id.0,
            worktree_id = %child.worktree_id.0,
            other_tasks,
            "archived subagent worktree is still referenced by another task"
        );
        return true;
    }

    let worktree = match store.get_worktree(child.worktree_id).await {
        Ok(Some(worktree)) => worktree,
        Ok(None) => {
            tracing::warn!(
                parent_session_id = %parent.id.0,
                child_session_id = %child.id.0,
                worktree_id = %child.worktree_id.0,
                "archived subagent worktree metadata was missing during cleanup"
            );
            return true;
        }
        Err(error) => {
            tracing::warn!(
                parent_session_id = %parent.id.0,
                child_session_id = %child.id.0,
                worktree_id = %child.worktree_id.0,
                "failed to load archived subagent worktree metadata: {error:#}"
            );
            return true;
        }
    };
    let workspace = match state.global_store().get_workspace(child.workspace_id).await {
        Ok(Some(workspace)) => workspace,
        Ok(None) => {
            tracing::warn!(
                parent_session_id = %parent.id.0,
                child_session_id = %child.id.0,
                workspace_id = %child.workspace_id.0,
                "workspace not found while cleaning archived subagent worktree"
            );
            return true;
        }
        Err(error) => {
            tracing::warn!(
                parent_session_id = %parent.id.0,
                child_session_id = %child.id.0,
                workspace_id = %child.workspace_id.0,
                "failed to load workspace while cleaning archived subagent worktree: {error:#}"
            );
            return true;
        }
    };
    let sandbox_binding = match store.get_sandbox_binding(worktree.id).await {
        Ok(binding) => binding,
        Err(error) => {
            tracing::warn!(
                parent_session_id = %parent.id.0,
                child_session_id = %child.id.0,
                worktree_id = %worktree.id.0,
                "failed to load archived subagent sandbox binding for cleanup: {error:#}"
            );
            cleanup_failed = true;
            None
        }
    };
    let cleanup_errors = crate::api::tasks::cleanup_task_worktrees(
        state.as_ref(),
        &workspace,
        child.task_id,
        &[crate::api::tasks::TaskWorktreeCleanupTarget {
            managed_root: crate::api::tasks::managed_worktree_root(state, &workspace, &worktree),
            sandbox_binding,
            worktree,
            destroy_worktree_on_cleanup: true,
        }],
        crate::api::tasks::BranchCleanupErrorMode::Report,
    )
    .await;
    if !cleanup_errors.is_empty() {
        tracing::warn!(
            parent_session_id = %parent.id.0,
            child_session_id = %child.id.0,
            worktree_id = %child.worktree_id.0,
            cleanup_errors = cleanup_errors.len(),
            "archived subagent worktree cleanup had errors"
        );
        cleanup_failed = true;
    }

    if !cleanup_failed {
        if let Err(error) = store.delete_sandbox_binding(child.worktree_id).await {
            tracing::warn!(
                parent_session_id = %parent.id.0,
                child_session_id = %child.id.0,
                worktree_id = %child.worktree_id.0,
                "failed to delete archived subagent sandbox binding after cleanup: {error:#}"
            );
            cleanup_failed = true;
        }
    }
    cleanup_failed
}
