use ctx_core::models::Worktree;

pub(crate) fn session_root_kind_for_worktree(wt: Option<&Worktree>) -> &'static str {
    match wt.and_then(|w| w.git_branch.as_ref()) {
        Some(_) => "worktree",
        None => "workspace_root",
    }
}
