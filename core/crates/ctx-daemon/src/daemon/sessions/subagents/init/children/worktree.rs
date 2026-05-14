use ctx_session_service::subagents::SubagentWorktreeSelection;

use super::SubagentChildInit;
use crate::daemon::sessions::subagents::errors::{api_error, ApiResult, SubagentErrorKind};
use crate::daemon::sessions::subagents::worktrees::create_subagent_worktree;

pub(super) async fn resolve_child_worktree(
    init: &SubagentChildInit,
    store: &ctx_store::Store,
) -> ApiResult<(ctx_core::ids::WorktreeId, Option<String>)> {
    match init.worktree_selection {
        SubagentWorktreeSelection::Inherit => Ok((init.parent.worktree_id, None)),
        SubagentWorktreeSelection::New => {
            let (vcs_kind, base_commit_sha) = init
                .worktree_plan
                .clone()
                .ok_or_else(|| api_error(SubagentErrorKind::Internal, "worktree plan missing"))?;
            let worktree = create_subagent_worktree(
                &init.state,
                store,
                &init.workspace,
                init.parent.task_id,
                &base_commit_sha,
                vcs_kind,
                &init.parent_effective,
            )
            .await?;
            Ok((worktree.id, Some(worktree.root_path)))
        }
    }
}
