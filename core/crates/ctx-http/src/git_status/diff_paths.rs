use std::collections::HashSet;
use std::sync::Arc;

use anyhow::Result;
use ctx_core::models::{Worktree, WorktreeVcsTouchedFile};

use crate::daemon::AppState;
use crate::settings::ExecutionMode;
use crate::worktree_data_plane::resolve_worktree_data_plane;

use super::sandbox::{
    container_git_diff_name_status, container_git_diff_name_status_no_renames,
    container_git_list_untracked,
};
use super::vcs_driver_for_worktree;

pub(super) async fn load_diff_touched_entries(
    state: &Arc<AppState>,
    worktree: &Worktree,
    base_commit_sha: &str,
) -> Result<Vec<WorktreeVcsTouchedFile>> {
    let paths = load_diff_path_states(state, worktree, base_commit_sha).await?;
    let mut items = Vec::new();
    for (path, orig_path, status) in paths {
        items.push(WorktreeVcsTouchedFile {
            path,
            orig_path,
            index_status: Some(status),
            worktree_status: None,
        });
    }
    Ok(items)
}

pub(super) async fn load_diff_file_count(
    state: &Arc<AppState>,
    worktree: &Worktree,
    base_commit_sha: &str,
) -> Result<i64> {
    // Tier 1 stays merge-base diff based, but avoids rename detection so large
    // change sets do not spend the hot path scoring rename candidates.
    let (entries, untracked) =
        load_diff_path_inputs_for_summary_count(state, worktree, base_commit_sha).await?;
    count_diff_paths(entries, untracked)
}

pub(super) fn count_diff_paths(
    entries: Vec<(String, String, Option<String>)>,
    untracked: Vec<String>,
) -> Result<i64> {
    let mut seen = HashSet::new();
    for (status, path, _) in entries {
        let path = path.trim();
        if path.is_empty() {
            continue;
        }
        if !seen.insert(path.to_string()) {
            continue;
        }
        if status.chars().next().is_none() {
            anyhow::bail!("vcs diff returned an empty status for {path}");
        }
    }
    for path in untracked {
        let path = path.trim();
        if !path.is_empty() {
            seen.insert(path.to_string());
        }
    }
    Ok(seen.len() as i64)
}

pub(super) fn build_diff_path_states(
    entries: Vec<(String, String, Option<String>)>,
    untracked: Vec<String>,
) -> Result<Vec<(String, Option<String>, String)>> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (status, path, orig_path) in entries {
        let path = path.trim().to_string();
        if path.is_empty() {
            continue;
        }
        if !seen.insert(path.clone()) {
            continue;
        }
        let Some(status_kind) = status.chars().next() else {
            anyhow::bail!("vcs diff returned an empty status for {path}");
        };
        out.push((path, orig_path, status_kind.to_string()));
    }
    for path in untracked {
        let path = path.trim().to_string();
        if path.is_empty() {
            continue;
        }
        if !seen.insert(path.clone()) {
            continue;
        }
        out.push((path, None, "?".to_string()));
    }
    out.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(out)
}

async fn load_diff_path_inputs(
    state: &Arc<AppState>,
    worktree: &Worktree,
    base_commit_sha: &str,
) -> Result<(Vec<(String, String, Option<String>)>, Vec<String>)> {
    load_diff_path_inputs_with_mode(state, worktree, base_commit_sha, false).await
}

async fn load_diff_path_inputs_for_summary_count(
    state: &Arc<AppState>,
    worktree: &Worktree,
    base_commit_sha: &str,
) -> Result<(Vec<(String, String, Option<String>)>, Vec<String>)> {
    load_diff_path_inputs_with_mode(state, worktree, base_commit_sha, true).await
}

async fn load_diff_path_inputs_with_mode(
    state: &Arc<AppState>,
    worktree: &Worktree,
    base_commit_sha: &str,
    summary_count: bool,
) -> Result<(Vec<(String, String, Option<String>)>, Vec<String>)> {
    let data_plane = resolve_worktree_data_plane(state, worktree).await?;
    let root = data_plane.live_worktree_root.as_path();
    let entries: Vec<(String, String, Option<String>)> =
        if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
            if summary_count {
                container_git_diff_name_status_no_renames(state, worktree, base_commit_sha).await?
            } else {
                container_git_diff_name_status(state, worktree, base_commit_sha).await?
            }
        } else {
            let driver = vcs_driver_for_worktree(worktree);
            let entries = if summary_count {
                driver
                    .diff_name_status_for_summary(root, base_commit_sha)
                    .await?
            } else {
                driver.diff_name_status(root, base_commit_sha).await?
            };
            entries
                .into_iter()
                .map(|entry| (entry.status, entry.path, entry.orig_path))
                .collect()
        };
    let untracked = if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
        container_git_list_untracked(state, worktree).await?
    } else {
        let driver = vcs_driver_for_worktree(worktree);
        driver.list_untracked(root).await?
    };
    Ok((entries, untracked))
}

async fn load_diff_path_states(
    state: &Arc<AppState>,
    worktree: &Worktree,
    base_commit_sha: &str,
) -> Result<Vec<(String, Option<String>, String)>> {
    let (entries, untracked) = load_diff_path_inputs(state, worktree, base_commit_sha).await?;
    build_diff_path_states(entries, untracked)
}
