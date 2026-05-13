use std::sync::Arc;

use crate::daemon::workspaces::{
    diff_worktree_summary_for_session, persist_provisioned_worktree,
    provision_worktree_for_execution,
};
use crate::daemon::AppState;
use ctx_core::ids::TaskId;
use ctx_core::models::{VcsKind, Workspace, Worktree};
use ctx_settings_model::ExecutionSettings;
use ctx_workspace_services::vcs_hooks;
use ctx_workspace_services::worktree_vcs::{
    effective_worktree_vcs_kind, WorktreeVcsCommitLookupSource,
};

use super::super::errors::{
    api_error, internal_api_error, internal_request_or_policy_error, ApiResult, SubagentErrorKind,
};
use ctx_session_service::subagents::SubagentWorktreeSelection;

pub(in crate::daemon::sessions::subagents) async fn plan_subagent_worktree_creation(
    state: &Arc<AppState>,
    parent_worktree: &Worktree,
    selection: SubagentWorktreeSelection,
) -> ApiResult<Option<(VcsKind, String)>> {
    if selection != SubagentWorktreeSelection::New {
        return Ok(None);
    }

    let source = crate::daemon::git_status::HttpWorktreeVcsSource::new(state, parent_worktree);
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
    let vcs_kind = effective_worktree_vcs_kind(parent_worktree.vcs_kind.clone());
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

    Ok(Some((vcs_kind, base_commit_sha)))
}

pub(in crate::daemon::sessions::subagents) async fn create_subagent_worktree(
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
    let (wt_path, sandbox_binding) = provision_worktree_for_execution(
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

    let worktree = persist_provisioned_worktree(state, store, workspace, worktree, sandbox_binding)
        .await
        .map_err(internal_api_error)?;
    if let Err(error) =
        vcs_hooks::ensure_task_commit_hook(state.as_ref(), workspace, &worktree, task_id).await
    {
        tracing::warn!(
            task_id = %task_id.0,
            worktree_id = %worktree.id.0,
            "failed to configure vcs hooks for subagent worktree: {error:#}"
        );
    }
    Ok(worktree)
}
