use std::path::{Path as StdPath, PathBuf};

use ctx_core::models::{Workspace, Worktree};
use ctx_fs::worktrees::managed_worktree_path;

use crate::daemon::AppState;

pub(crate) fn managed_worktree_root(
    state: &AppState,
    workspace: &Workspace,
    worktree: &Worktree,
) -> Option<PathBuf> {
    let root = PathBuf::from(&worktree.root_path);
    let expected = managed_worktree_path(&state.core.data_root, workspace.id, worktree.id);
    if normalize_path_for_comparison(&root) == normalize_path_for_comparison(&expected) {
        Some(expected)
    } else {
        None
    }
}

fn normalize_path_for_comparison(path: &StdPath) -> PathBuf {
    let mut suffix = Vec::new();
    let mut cursor = path;
    loop {
        match std::fs::canonicalize(cursor) {
            Ok(canonical) => {
                let mut normalized = canonical;
                for component in suffix.iter().rev() {
                    normalized.push(component);
                }
                return normalized;
            }
            Err(_) => {
                let Some(parent) = cursor.parent() else {
                    return path.to_path_buf();
                };
                let Some(name) = cursor.file_name() else {
                    return path.to_path_buf();
                };
                suffix.push(name.to_os_string());
                cursor = parent;
            }
        }
    }
}
