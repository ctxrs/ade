pub const DEFAULT_MAX_SUBAGENTS_PER_CALL: usize = 10;
pub const DEFAULT_MAX_ACTIVE_SUBAGENTS_PER_PARENT: usize = 12;
pub const DEFAULT_MAX_SUBAGENT_DEPTH: usize = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubagentWorktreeSelection {
    Inherit,
    New,
}

pub fn resolve_max_subagents_per_call(configured: Option<u32>) -> usize {
    configured
        .filter(|value| *value > 0)
        .map(|value| value as usize)
        .unwrap_or(DEFAULT_MAX_SUBAGENTS_PER_CALL)
}

pub fn parse_subagent_worktree(value: Option<&str>) -> Result<SubagentWorktreeSelection, String> {
    let trimmed = value.map(str::trim).filter(|raw| !raw.is_empty());
    match trimmed {
        Some("inherit") => Ok(SubagentWorktreeSelection::Inherit),
        Some("new") => Ok(SubagentWorktreeSelection::New),
        Some(_) => Err("worktree must be 'inherit' or 'new'".to_string()),
        None => Err("worktree is required".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_max_subagents_per_call_from_positive_config() {
        assert_eq!(resolve_max_subagents_per_call(Some(3)), 3);
        assert_eq!(
            resolve_max_subagents_per_call(Some(0)),
            DEFAULT_MAX_SUBAGENTS_PER_CALL
        );
        assert_eq!(
            resolve_max_subagents_per_call(None),
            DEFAULT_MAX_SUBAGENTS_PER_CALL
        );
    }

    #[test]
    fn parses_subagent_worktree_selection_strictly() {
        assert_eq!(
            parse_subagent_worktree(Some(" inherit ")),
            Ok(SubagentWorktreeSelection::Inherit)
        );
        assert_eq!(
            parse_subagent_worktree(Some("new")),
            Ok(SubagentWorktreeSelection::New)
        );
        assert_eq!(
            parse_subagent_worktree(Some("reuse"))
                .as_ref()
                .map_err(String::as_str),
            Err("worktree must be 'inherit' or 'new'")
        );
        assert_eq!(
            parse_subagent_worktree(None)
                .as_ref()
                .map_err(String::as_str),
            Err("worktree is required")
        );
    }
}
