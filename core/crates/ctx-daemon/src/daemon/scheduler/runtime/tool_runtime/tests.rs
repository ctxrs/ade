use std::path::PathBuf;

use super::{cwd_outside_worktree, sanitize_spool_segment};

#[test]
fn cwd_guardrail_allows_in_tree_and_relative_paths() {
    let worktree_root = PathBuf::from("/repo/worktree");
    let canonical_root = PathBuf::from("/private/repo/worktree");

    assert!(!cwd_outside_worktree(
        "",
        &worktree_root,
        Some(&canonical_root)
    ));
    assert!(!cwd_outside_worktree(
        "relative/path",
        &worktree_root,
        Some(&canonical_root)
    ));
    assert!(!cwd_outside_worktree(
        "/repo/worktree/subdir",
        &worktree_root,
        Some(&canonical_root)
    ));
    assert!(!cwd_outside_worktree(
        "/private/repo/worktree/subdir",
        &worktree_root,
        Some(&canonical_root)
    ));
}

#[test]
fn cwd_guardrail_flags_absolute_paths_outside_worktree() {
    let worktree_root = PathBuf::from("/repo/worktree");
    assert!(cwd_outside_worktree("/tmp/outside", &worktree_root, None));
}

#[test]
fn spool_segment_is_sanitized_and_bounded() {
    assert_eq!(sanitize_spool_segment(""), "tool_output");
    assert_eq!(sanitize_spool_segment("tool/id:1"), "tool_id_1");
    assert_eq!(sanitize_spool_segment(&"x".repeat(120)).len(), 80);
}
