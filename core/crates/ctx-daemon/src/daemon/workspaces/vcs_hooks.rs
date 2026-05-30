#[path = "vcs_hooks/host.rs"]
mod host;
#[path = "vcs_hooks/sandbox.rs"]
mod sandbox;

use anyhow::Result;
use ctx_core::models::{Workspace, Worktree};
use ctx_worktree_vcs_service::VcsHooksHost;

pub(in crate::daemon) use host::WorkspaceVcsHookHost;

pub async fn cleanup_worktree_hooks_with_host<H: VcsHooksHost>(
    host: &H,
    workspace: &Workspace,
    worktree: &Worktree,
) -> Result<()> {
    ctx_worktree_vcs_service::cleanup_worktree_hooks(host, workspace, worktree).await
}

#[cfg(test)]
mod tests;
