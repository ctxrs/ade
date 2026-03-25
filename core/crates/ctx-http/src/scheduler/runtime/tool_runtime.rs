use std::path::{Path, PathBuf};

use tokio::fs;

use crate::daemon::AppState;

use super::super::tools::normalize::NormalizedToolEvent;

pub(super) fn cwd_outside_worktree(
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

pub(super) async fn maybe_spool_tool_output(
    state: &AppState,
    tool_event: &NormalizedToolEvent,
    session_id: ctx_core::ids::SessionId,
    turn_id: ctx_core::ids::TurnId,
) -> Option<String> {
    if !state.core.tool_output_spool_enabled {
        return None;
    }
    let tool_call_id = tool_event.tool_call_id.as_deref()?;
    let output = tool_event.raw_output_text.as_deref()?;
    if output.trim().is_empty() {
        return None;
    }

    let dir = state
        .core
        .tool_output_spool_dir
        .join(session_id.0.to_string())
        .join(turn_id.0.to_string());
    if let Err(err) = fs::create_dir_all(&dir).await {
        tracing::warn!(
            "failed to create tool output spool dir {}: {err}",
            dir.to_string_lossy()
        );
        return None;
    }

    let file_name = format!("{}.txt", sanitize_spool_segment(tool_call_id));
    let path = dir.join(file_name);
    if let Err(err) = fs::write(&path, output.as_bytes()).await {
        tracing::warn!(
            "failed to write tool output spool {}: {err}",
            path.to_string_lossy()
        );
        return None;
    }
    Some(path.to_string_lossy().to_string())
}

fn sanitize_spool_segment(raw: &str) -> String {
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
