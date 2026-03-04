use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use ctx_core::ids::WorktreeId;
use ctx_core::models::{Workspace, Worktree};

use crate::daemon::AppState;
use crate::execution_effective;
use crate::harness_sources::{self, HarnessSourceKind, ResolvedHarnessSource};
use crate::logs;
use crate::provider_accounts;
use crate::settings::ExecutionMode;

pub(crate) async fn provider_probe_env(
    state: &Arc<AppState>,
    provider_id: &str,
) -> Result<(ResolvedHarnessSource, HashMap<String, String>), String> {
    let source =
        harness_sources::resolve_provider_source_for_probe(&state.core.data_root, provider_id)
            .await
            .map_err(|e| logs::redact_sensitive(&e.to_string()))?;
    let mut env = HashMap::new();
    env.insert("CTX_DAEMON_URL".to_string(), state.core.daemon_url.clone());
    if let Some(token) = state.core.auth_token.as_ref() {
        env.insert("CTX_AUTH_TOKEN".to_string(), token.clone());
    }
    if source.source_kind == HarnessSourceKind::Subscription {
        let extra = provider_accounts::subscription_env_for_active_account(
            &state.core.data_root,
            provider_id,
        )
        .await;
        if let Ok(extra) = extra {
            for (key, value) in extra {
                env.insert(key, value);
            }
        }
    }
    for (key, value) in source.env.iter() {
        env.insert(key.clone(), value.clone());
    }
    Ok((source, env))
}

fn select_probe_worktree(workspace: &Workspace, worktrees: &[Worktree]) -> Option<Worktree> {
    if let Some(preferred) = worktrees
        .iter()
        .find(|worktree| worktree.root_path == workspace.root_path)
    {
        return Some(preferred.clone());
    }
    worktrees.first().cloned()
}

fn synthetic_probe_worktree(workspace: &Workspace) -> Worktree {
    Worktree {
        id: WorktreeId(uuid::Uuid::nil()),
        workspace_id: workspace.id,
        root_path: workspace.root_path.clone(),
        base_commit_sha: String::new(),
        git_branch: None,
        vcs_kind: workspace.vcs_kind.clone(),
        base_revision: None,
        vcs_ref: None,
        created_at: Utc::now(),
        bootstrap_status: None,
        bootstrap_started_at: None,
        bootstrap_finished_at: None,
        bootstrap_exit_code: None,
        bootstrap_timeout_sec: None,
        bootstrap_error: None,
        bootstrap_log_path: None,
        bootstrap_log_truncated: None,
        bootstrap_command: None,
        bootstrap_script_path: None,
    }
}

pub(crate) async fn provider_probe_env_for_workspace_runtime(
    state: &Arc<AppState>,
    workspace: &Workspace,
    provider_id: &str,
) -> Result<(ResolvedHarnessSource, HashMap<String, String>), String> {
    let (source, mut env) = provider_probe_env(state, provider_id).await?;
    let effective = execution_effective::effective_execution_settings(state, workspace.id)
        .await
        .map_err(|err| {
            logs::redact_sensitive(&format!("effective execution settings failed: {err}"))
        })?;
    if matches!(effective.mode, ExecutionMode::Host) {
        return Ok((source, env));
    }

    let worktrees = state
        .global_store()
        .list_worktrees(workspace.id)
        .await
        .map_err(|err| {
            logs::redact_sensitive(&format!("loading workspace worktrees failed: {err}"))
        })?;
    let worktree = select_probe_worktree(workspace, &worktrees)
        .unwrap_or_else(|| synthetic_probe_worktree(workspace));
    let runtime_plan = state
        .execution
        .harness
        .prepare(workspace, &worktree, &effective, &state.core.daemon_url)
        .await
        .map_err(|err| {
            logs::redact_sensitive(&format!("probe runtime preparation failed: {err:#}"))
        })?;
    for (key, value) in runtime_plan.env_overrides {
        env.insert(key, value);
    }
    Ok((source, env))
}

#[cfg(test)]
mod tests {
    use super::{select_probe_worktree, synthetic_probe_worktree};
    use chrono::Utc;
    use ctx_core::ids::{WorkspaceId, WorktreeId};
    use ctx_core::models::{Workspace, Worktree};
    use uuid::Uuid;

    fn sample_workspace(root_path: &str) -> Workspace {
        Workspace {
            id: WorkspaceId(Uuid::new_v4()),
            name: "ws".to_string(),
            root_path: root_path.to_string(),
            created_at: Utc::now(),
            vcs_kind: None,
        }
    }

    fn sample_worktree(workspace_id: WorkspaceId, root_path: &str) -> Worktree {
        Worktree {
            id: WorktreeId(Uuid::new_v4()),
            workspace_id,
            root_path: root_path.to_string(),
            base_commit_sha: String::new(),
            git_branch: None,
            vcs_kind: None,
            base_revision: None,
            vcs_ref: None,
            created_at: Utc::now(),
            bootstrap_status: None,
            bootstrap_started_at: None,
            bootstrap_finished_at: None,
            bootstrap_exit_code: None,
            bootstrap_timeout_sec: None,
            bootstrap_error: None,
            bootstrap_log_path: None,
            bootstrap_log_truncated: None,
            bootstrap_command: None,
            bootstrap_script_path: None,
        }
    }

    #[test]
    fn select_probe_worktree_prefers_workspace_root_match() {
        let workspace = sample_workspace("/repo");
        let first = sample_worktree(workspace.id, "/repo-alt");
        let preferred = sample_worktree(workspace.id, "/repo");
        let selected = select_probe_worktree(&workspace, &[first, preferred.clone()]);
        assert_eq!(selected.expect("selected").id, preferred.id);
    }

    #[test]
    fn select_probe_worktree_falls_back_to_first_entry() {
        let workspace = sample_workspace("/repo");
        let first = sample_worktree(workspace.id, "/repo-a");
        let second = sample_worktree(workspace.id, "/repo-b");
        let selected = select_probe_worktree(&workspace, &[first.clone(), second]);
        assert_eq!(selected.expect("selected").id, first.id);
    }

    #[test]
    fn select_probe_worktree_returns_none_for_empty_list() {
        let workspace = sample_workspace("/repo");
        assert!(select_probe_worktree(&workspace, &[]).is_none());
    }

    #[test]
    fn synthetic_probe_worktree_uses_workspace_root() {
        let workspace = sample_workspace("/repo");
        let synthetic = synthetic_probe_worktree(&workspace);
        assert_eq!(synthetic.workspace_id, workspace.id);
        assert_eq!(synthetic.root_path, workspace.root_path);
    }
}
