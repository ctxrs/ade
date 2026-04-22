use super::diff_paths::{build_diff_path_states, count_diff_paths};
use super::{build_large_change_set_touched_files, WORKTREE_VCS_REVIEWABLE_FILE_LIMIT};
use anyhow::Result;

#[test]
fn build_diff_path_states_deduplicates_untracked_paths_already_in_diff() -> Result<()> {
    let out = build_diff_path_states(
        vec![("D".to_string(), "src/example.rs".to_string(), None)],
        vec!["src/example.rs".to_string()],
    )?;

    assert_eq!(
        out,
        vec![("src/example.rs".to_string(), None, "D".to_string())]
    );
    Ok(())
}

#[test]
fn count_diff_paths_deduplicates_untracked_paths_already_in_diff() -> Result<()> {
    let count = count_diff_paths(
        vec![("D".to_string(), "src/example.rs".to_string(), None)],
        vec!["src/example.rs".to_string()],
    )?;

    assert_eq!(count, 1);
    Ok(())
}

#[test]
fn large_change_set_touched_files_stays_truthful_without_rows() {
    let touched_files =
        build_large_change_set_touched_files(WORKTREE_VCS_REVIEWABLE_FILE_LIMIT + 1);

    assert!(touched_files.items.is_empty());
    assert!(touched_files.truncated);
    assert_eq!(
        touched_files.total_count,
        Some(WORKTREE_VCS_REVIEWABLE_FILE_LIMIT + 1)
    );
}
