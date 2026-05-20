use std::path::{Path, PathBuf};

pub fn cwd_outside_worktree(
    cwd: &str,
    workdir_root: &Path,
    workdir_canonical: Option<&PathBuf>,
) -> bool {
    if cwd.trim().is_empty() {
        return false;
    }
    let cwd_path = Path::new(cwd);
    if cwd_path.is_relative() {
        return false;
    }
    if cwd_path.starts_with(workdir_root) {
        return false;
    }
    if let Some(root) = workdir_canonical {
        if cwd_path.starts_with(root) {
            return false;
        }
    }
    true
}

pub fn sanitize_spool_segment(raw: &str) -> String {
    let mut output: String = raw
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if output.is_empty() {
        output.push_str("tool_output");
    }
    if output.len() > 80 {
        output.truncate(80);
    }
    output
}

#[cfg(test)]
mod tests {
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
}
