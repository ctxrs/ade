use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::api::provider_probe_auth::provider_has_active_auth_config_with_runtime_root;
use crate::daemon::AppState;
use crate::execution_effective;
use crate::logs;
use crate::settings::{ContainerMountMode, ExecutionMode};
use chrono::Utc;
use ctx_core::ids::WorktreeId;
use ctx_core::models::{Workspace, Worktree};
use ctx_harness_sources::{HarnessSourceKind, ResolvedHarnessSource};
use ctx_provider_accounts as provider_accounts;
use ctx_worktree_data_plane::{
    apply_data_plane_to_execution_settings, workspace_data_plane, WorktreeDataPlane,
};

use crate::worktree_data_plane::resolve_worktree_data_plane;

pub(crate) struct WorkspaceRuntimeProbeContext {
    pub(crate) source: ResolvedHarnessSource,
    pub(crate) env: HashMap<String, String>,
    pub(crate) cwd: PathBuf,
}

pub(crate) async fn provider_probe_env(
    state: &Arc<AppState>,
    provider_id: &str,
) -> Result<(ResolvedHarnessSource, HashMap<String, String>), String> {
    provider_env_with_runtime_root(state, provider_id, None, true).await
}

async fn provider_env_with_runtime_root(
    state: &Arc<AppState>,
    provider_id: &str,
    runtime_data_root: Option<&Path>,
    require_subscription_account_env: bool,
) -> Result<(ResolvedHarnessSource, HashMap<String, String>), String> {
    let source = ctx_harness_sources::resolve_provider_source_for_probe_with_runtime_root(
        &state.core.data_root,
        provider_id,
        runtime_data_root,
    )
    .await
    .map_err(|e| logs::redact_sensitive(&e.to_string()))?;
    let mut env = HashMap::new();
    env.insert("CTX_DAEMON_URL".to_string(), state.core.daemon_url.clone());
    if let Some(token) = state.core.auth_token.as_ref() {
        env.insert("CTX_AUTH_TOKEN".to_string(), token.clone());
    }
    if source.source_kind == HarnessSourceKind::Subscription {
        if require_subscription_account_env && provider_id == "codex" {
            let has_auth = match runtime_data_root {
                Some(runtime_root) => {
                    provider_accounts::codex_has_active_auth_with_runtime_root(
                        &state.core.data_root,
                        runtime_root,
                    )
                    .await
                }
                None => provider_accounts::codex_has_active_auth(&state.core.data_root).await,
            }
            .map_err(|err| {
                logs::redact_sensitive(&format!(
                    "probe subscription env preparation failed: {err:#}"
                ))
            })?;
            if !has_auth {
                return Err(format!(
                    "subscription account env is missing for provider '{provider_id}'; configure an active account or select an endpoint"
                ));
            }
        }
        let extra = match runtime_data_root {
            Some(runtime_root) => {
                provider_accounts::subscription_env_for_active_account_with_runtime_root(
                    &state.core.data_root,
                    runtime_root,
                    provider_id,
                )
                .await
            }
            None => {
                provider_accounts::subscription_env_for_active_account(
                    &state.core.data_root,
                    provider_id,
                )
                .await
            }
        }
        .map_err(|err| {
            logs::redact_sensitive(&format!(
                "probe subscription env preparation failed: {err:#}"
            ))
        })?;
        if require_subscription_account_env
            && subscription_probe_requires_account_env(provider_id)
            && extra.is_empty()
        {
            return Err(format!(
                "subscription account env is missing for provider '{provider_id}'; configure an active account or select an endpoint"
            ));
        }
        for (key, value) in extra {
            env.insert(key, value);
        }
    }
    for (key, value) in source.env.iter() {
        env.insert(key.clone(), value.clone());
    }
    Ok((source, env))
}

fn subscription_probe_requires_account_env(provider_id: &str) -> bool {
    matches!(
        provider_id,
        "claude-crp" | "gemini" | "qwen" | "kimi" | "mistral" | "copilot" | "cursor" | "amp"
    )
}

#[cfg(test)]
fn probe_worktree_priority(
    data_root: &Path,
    workspace: &Workspace,
    worktree: &Worktree,
    sandbox_bound_worktree_ids: &std::collections::HashSet<WorktreeId>,
) -> Option<u8> {
    if worktree.root_path == workspace.root_path {
        return Some(0);
    }

    if sandbox_bound_worktree_ids.contains(&worktree.id)
        || Path::new(&worktree.root_path)
            == ctx_fs::worktrees::managed_worktree_path(data_root, workspace.id, worktree.id)
    {
        return Some(1);
    }

    None
}

#[cfg(test)]
fn select_probe_worktree(
    data_root: &Path,
    workspace: &Workspace,
    worktrees: &[Worktree],
    sandbox_bound_worktree_ids: &std::collections::HashSet<WorktreeId>,
) -> Result<Option<Worktree>, String> {
    if worktrees.is_empty() {
        return Ok(None);
    }

    let mut selected: Option<(u8, Worktree)> = None;
    let mut stale_paths = Vec::new();
    for worktree in worktrees {
        match probe_worktree_priority(data_root, workspace, worktree, sandbox_bound_worktree_ids) {
            Some(priority) => {
                if selected
                    .as_ref()
                    .map(|(best_priority, _)| priority < *best_priority)
                    .unwrap_or(true)
                {
                    selected = Some((priority, worktree.clone()));
                }
            }
            None => stale_paths.push(worktree.root_path.clone()),
        }
    }

    if let Some((_, worktree)) = selected {
        return Ok(Some(worktree));
    }

    stale_paths.sort();
    Err(format!(
        "container provider probe requires an explicit workspace-root or sandbox-managed worktree; workspace '{}' has no eligible probe root (found: {})",
        workspace.root_path,
        stale_paths.join(", ")
    ))
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

fn probe_cwd_for_workspace_runtime(
    data_plane: &WorktreeDataPlane,
    worktree: &Worktree,
    mode: ExecutionMode,
    mount_mode: ContainerMountMode,
) -> PathBuf {
    if matches!(mode, ExecutionMode::Sandbox)
        && matches!(mount_mode, ContainerMountMode::DiskIsolated)
    {
        return data_plane.live_worktree_root.clone();
    }
    PathBuf::from(&worktree.root_path)
}

async fn finalize_workspace_probe_env(
    source: &ResolvedHarnessSource,
    provider_id: &str,
    env: &mut HashMap<String, String>,
) -> Result<(), String> {
    // Mirror scheduler behavior for Codex endpoint sources in container mode:
    // ensure CODEX_HOME points at container-accessible runtime root, not host endpoint-home paths.
    if provider_id == "codex" && source.source_kind == HarnessSourceKind::Endpoint {
        if let Some(root) = env.get("CTX_DATA_ROOT").cloned() {
            provider_accounts::ensure_codex_endpoint_runtime_home_from_env(Path::new(&root), env)
                .await
                .map_err(|err| {
                    logs::redact_sensitive(&format!(
                        "probe codex endpoint runtime-home preparation failed: {err:#}"
                    ))
                })?;
        }
    }

    if let Some(root) = env.get("CTX_DATA_ROOT").cloned() {
        provider_accounts::ensure_provider_runtime_home_env(Path::new(&root), provider_id, env)
            .await
            .map_err(|err| {
                logs::redact_sensitive(&format!(
                    "probe provider runtime-home preparation failed: {err:#}"
                ))
            })?;
    }
    Ok(())
}

async fn provider_context_for_workspace_runtime(
    state: &Arc<AppState>,
    workspace: &Workspace,
    provider_id: &str,
    require_subscription_account_env: bool,
) -> Result<WorkspaceRuntimeProbeContext, String> {
    let effective = execution_effective::effective_execution_settings(state, workspace.id)
        .await
        .map_err(|err| {
            logs::redact_sensitive(&format!("effective execution settings failed: {err}"))
        })?;
    if matches!(effective.mode, ExecutionMode::Host) {
        let (source, env) = if require_subscription_account_env {
            provider_probe_env(state, provider_id).await
        } else {
            provider_env_with_runtime_root(state, provider_id, None, false).await
        }?;
        return Ok(WorkspaceRuntimeProbeContext {
            source,
            env,
            cwd: PathBuf::from(&workspace.root_path),
        });
    }

    let worktree = synthetic_probe_worktree(workspace);
    let worktree_data_plane = workspace_data_plane(workspace, effective.mode.clone());
    let effective = apply_data_plane_to_execution_settings(&effective, &worktree_data_plane)
        .map_err(|err| {
            logs::redact_sensitive(&format!(
                "applying workspace probe data plane failed: {err:#}"
            ))
        })?;
    let cwd = probe_cwd_for_workspace_runtime(
        &worktree_data_plane,
        &worktree,
        effective.mode.clone(),
        effective.container.mount_mode.clone(),
    );
    let runtime_plan = state
        .execution
        .harness
        .prepare(workspace, &worktree, &effective, &state.core.daemon_url)
        .await
        .map_err(|err| {
            logs::redact_sensitive(&format!("probe runtime preparation failed: {err:#}"))
        })?;
    let sandbox_mode = crate::workspace_runtime::selected_sandbox_command_mode(&state.core.data_root)
        .map_err(|err| logs::redact_sensitive(&format!("sandbox command selection failed: {err:#}")))?;
    ctx_sandbox_materialization::ensure_workspace_root_from_host_copy(
        &state.core.data_root,
        &sandbox_mode,
        workspace,
    )
    .await
    .map_err(|err| {
        logs::redact_sensitive(&format!(
            "sandbox workspace root materialization failed: {err:#}"
        ))
    })?;
    let runtime_root = runtime_plan
        .env_overrides
        .get("CTX_DATA_ROOT")
        .map(|value| Path::new(value).to_path_buf());
    let (source, mut env) = provider_env_with_runtime_root(
        state,
        provider_id,
        runtime_root.as_deref(),
        require_subscription_account_env,
    )
    .await?;
    for (key, value) in runtime_plan.env_overrides {
        env.insert(key, value);
    }
    finalize_workspace_probe_env(&source, provider_id, &mut env).await?;
    Ok(WorkspaceRuntimeProbeContext { source, env, cwd })
}

pub(crate) async fn provider_auth_context_for_worktree_runtime(
    state: &Arc<AppState>,
    worktree: &Worktree,
    provider_id: &str,
) -> Result<WorkspaceRuntimeProbeContext, String> {
    let workspace = state
        .global_store()
        .get_workspace(worktree.workspace_id)
        .await
        .map_err(|err| logs::redact_sensitive(&format!("loading workspace failed: {err:#}")))?
        .ok_or_else(|| "workspace not found".to_string())?;
    let effective = execution_effective::effective_execution_settings(state, workspace.id)
        .await
        .map_err(|err| {
            logs::redact_sensitive(&format!("effective execution settings failed: {err}"))
        })?;
    let worktree_data_plane =
        resolve_worktree_data_plane(state, worktree)
            .await
            .map_err(|err| {
                logs::redact_sensitive(&format!(
                    "resolving session auth worktree data plane failed: {err:#}"
                ))
            })?;
    let effective = apply_data_plane_to_execution_settings(&effective, &worktree_data_plane)
        .map_err(|err| {
            logs::redact_sensitive(&format!(
                "applying session auth worktree data plane failed: {err:#}"
            ))
        })?;
    let cwd = probe_cwd_for_workspace_runtime(
        &worktree_data_plane,
        worktree,
        effective.mode.clone(),
        effective.container.mount_mode.clone(),
    );
    let runtime_plan = state
        .execution
        .harness
        .prepare(&workspace, worktree, &effective, &state.core.daemon_url)
        .await
        .map_err(|err| {
            logs::redact_sensitive(&format!("session auth runtime preparation failed: {err:#}"))
        })?;
    let runtime_root = runtime_plan
        .env_overrides
        .get("CTX_DATA_ROOT")
        .map(|value| Path::new(value).to_path_buf());
    let (source, mut env) =
        provider_env_with_runtime_root(state, provider_id, runtime_root.as_deref(), false).await?;
    for (key, value) in runtime_plan.env_overrides {
        env.insert(key, value);
    }
    finalize_workspace_probe_env(&source, provider_id, &mut env).await?;
    Ok(WorkspaceRuntimeProbeContext { source, env, cwd })
}

pub(crate) async fn provider_probe_context_for_workspace_runtime(
    state: &Arc<AppState>,
    workspace: &Workspace,
    provider_id: &str,
) -> Result<WorkspaceRuntimeProbeContext, String> {
    provider_context_for_workspace_runtime(state, workspace, provider_id, true).await
}

pub(crate) async fn provider_auth_context_for_workspace_runtime(
    state: &Arc<AppState>,
    workspace: &Workspace,
    provider_id: &str,
) -> Result<WorkspaceRuntimeProbeContext, String> {
    provider_context_for_workspace_runtime(state, workspace, provider_id, false).await
}

pub(crate) async fn provider_has_active_auth_for_workspace_runtime(
    state: &Arc<AppState>,
    workspace: &Workspace,
    provider_id: &str,
    source_config: Option<&ctx_harness_sources::HarnessProviderSourceConfig>,
) -> bool {
    let runtime_root = provider_context_for_workspace_runtime(state, workspace, provider_id, false)
        .await
        .ok()
        .and_then(|context| context.env.get("CTX_DATA_ROOT").map(PathBuf::from));
    provider_has_active_auth_config_with_runtime_root(
        &state.core.data_root,
        runtime_root.as_deref(),
        provider_id,
        source_config,
    )
    .await
}

pub(crate) async fn provider_probe_env_for_workspace_runtime(
    state: &Arc<AppState>,
    workspace: &Workspace,
    provider_id: &str,
) -> Result<(ResolvedHarnessSource, HashMap<String, String>), String> {
    let context =
        provider_probe_context_for_workspace_runtime(state, workspace, provider_id).await?;
    Ok((context.source, context.env))
}

#[cfg(test)]
mod tests {
    use super::{
        finalize_workspace_probe_env, probe_cwd_for_workspace_runtime,
        provider_env_with_runtime_root, select_probe_worktree, synthetic_probe_worktree,
    };
    use std::collections::{HashMap, HashSet};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use chrono::Utc;
    use ctx_core::ids::{WorkspaceId, WorktreeId};
    use ctx_core::models::{Workspace, Worktree};
    use ctx_fs::worktrees::managed_worktree_path;
    use ctx_sandbox_contract::container_worktree_root;
    use ctx_store::StoreManager;
    use uuid::Uuid;

    use crate::daemon::AppState;
    use crate::settings::{ContainerMountMode, ExecutionMode};
    use ctx_harness_sources::{HarnessSourceKind, ResolvedHarnessSource};
    use ctx_provider_accounts as provider_accounts;
    use ctx_provider_accounts::KIMI_SHARE_DIR_ENV;
    use ctx_worktree_data_plane::WorktreeDataPlane;

    static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    async fn lock_env() -> tokio::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().await
    }

    struct EnvGuard {
        key: &'static str,
        prev: Option<String>,
    }

    impl EnvGuard {
        fn without(key: &'static str) -> Self {
            let prev = std::env::var(key).ok();
            std::env::remove_var(key);
            Self { key, prev }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(value) = self.prev.as_deref() {
                std::env::set_var(self.key, value);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    async fn test_state(data_root: &Path) -> Arc<AppState> {
        let stores = StoreManager::open(data_root).await.expect("open stores");
        Arc::new(AppState::new(
            data_root.to_path_buf(),
            stores,
            HashMap::new(),
            "http://127.0.0.1:0".to_string(),
            None,
        ))
    }

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
    fn select_probe_worktree_accepts_workspace_root_match() {
        let workspace = sample_workspace("/repo");
        let first = sample_worktree(workspace.id, "/repo-alt");
        let preferred = sample_worktree(workspace.id, "/repo");
        let selected = select_probe_worktree(
            Path::new("/ctx"),
            &workspace,
            &[first, preferred.clone()],
            &HashSet::new(),
        )
        .expect("selection should succeed");
        assert_eq!(selected.expect("selected").id, preferred.id);
    }

    #[test]
    fn select_probe_worktree_accepts_ctx_managed_worktree_match() {
        let data_root = tempfile::tempdir().expect("tempdir");
        let workspace = sample_workspace("/repo");
        let worktree_id = WorktreeId(Uuid::new_v4());
        let managed_path = managed_worktree_path(data_root.path(), workspace.id, worktree_id);
        let managed = Worktree {
            id: worktree_id,
            workspace_id: workspace.id,
            root_path: managed_path.to_string_lossy().to_string(),
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
        };

        let selected = select_probe_worktree(
            data_root.path(),
            &workspace,
            std::slice::from_ref(&managed),
            &HashSet::new(),
        )
        .expect("selection should succeed");
        assert_eq!(selected.expect("selected").id, managed.id);
    }

    #[test]
    fn select_probe_worktree_accepts_sandbox_bound_worktree() {
        let workspace = sample_workspace("/repo");
        let sandbox_worktree = sample_worktree(workspace.id, "/shadow/worktrees/one");
        let mut sandbox_bound_worktree_ids = HashSet::new();
        sandbox_bound_worktree_ids.insert(sandbox_worktree.id);

        let selected = select_probe_worktree(
            Path::new("/ctx"),
            &workspace,
            std::slice::from_ref(&sandbox_worktree),
            &sandbox_bound_worktree_ids,
        )
        .expect("selection should succeed");

        assert_eq!(selected.expect("selected").id, sandbox_worktree.id);
    }

    #[test]
    fn select_probe_worktree_rejects_only_stale_worktrees() {
        let workspace = sample_workspace("/repo");
        let first = sample_worktree(workspace.id, "/repo-a");
        let second = sample_worktree(workspace.id, "/repo-b");
        let err = select_probe_worktree(
            Path::new("/ctx"),
            &workspace,
            &[first, second],
            &HashSet::new(),
        )
        .expect_err("stale worktrees should be rejected explicitly");
        assert!(err.contains("sandbox-managed worktree"));
    }

    #[test]
    fn select_probe_worktree_returns_none_for_empty_list() {
        let workspace = sample_workspace("/repo");
        assert!(
            select_probe_worktree(Path::new("/ctx"), &workspace, &[], &HashSet::new())
                .expect("empty selection should not error")
                .is_none()
        );
    }

    #[test]
    fn select_probe_worktree_prefers_workspace_root_when_multiple_valid_worktrees_exist() {
        let data_root = tempfile::tempdir().expect("tempdir");
        let workspace = sample_workspace("/repo");
        let root_worktree = sample_worktree(workspace.id, "/repo");
        let managed_id = WorktreeId(Uuid::new_v4());
        let managed = Worktree {
            id: managed_id,
            workspace_id: workspace.id,
            root_path: managed_worktree_path(data_root.path(), workspace.id, managed_id)
                .to_string_lossy()
                .to_string(),
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
        };

        let selected = select_probe_worktree(
            data_root.path(),
            &workspace,
            &[managed, root_worktree.clone()],
            &HashSet::new(),
        )
        .expect("multiple valid candidates should be accepted");
        assert_eq!(selected.expect("selected").id, root_worktree.id);
    }

    #[test]
    fn select_probe_worktree_accepts_multiple_valid_ctx_managed_worktrees() {
        let data_root = tempfile::tempdir().expect("tempdir");
        let workspace = sample_workspace("/repo");
        let first_id = WorktreeId(Uuid::new_v4());
        let first = Worktree {
            id: first_id,
            workspace_id: workspace.id,
            root_path: managed_worktree_path(data_root.path(), workspace.id, first_id)
                .to_string_lossy()
                .to_string(),
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
        };
        let second_id = WorktreeId(Uuid::new_v4());
        let second = Worktree {
            id: second_id,
            workspace_id: workspace.id,
            root_path: managed_worktree_path(data_root.path(), workspace.id, second_id)
                .to_string_lossy()
                .to_string(),
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
        };

        let selected = select_probe_worktree(
            data_root.path(),
            &workspace,
            &[first.clone(), second],
            &HashSet::new(),
        )
        .expect("multiple valid managed worktrees should be accepted");
        assert_eq!(selected.expect("selected").id, first.id);
    }

    #[test]
    fn synthetic_probe_worktree_uses_workspace_root() {
        let workspace = sample_workspace("/repo");
        let synthetic = synthetic_probe_worktree(&workspace);
        assert_eq!(synthetic.workspace_id, workspace.id);
        assert_eq!(synthetic.root_path, workspace.root_path);
    }

    #[test]
    fn probe_cwd_uses_container_workspace_root_for_disk_isolated_workspace_root() {
        let workspace = sample_workspace("/host/workspace");
        let worktree = sample_worktree(workspace.id, "/host/workspace");
        let data_plane = WorktreeDataPlane {
            binding: None,
            workspace: workspace.clone(),
            execution_mode: ExecutionMode::Sandbox,
            live_workspace_root: PathBuf::from("/ctx/ws"),
            live_worktree_root: PathBuf::from("/ctx/ws"),
        };

        let cwd = probe_cwd_for_workspace_runtime(
            &data_plane,
            &worktree,
            ExecutionMode::Sandbox,
            ContainerMountMode::DiskIsolated,
        );

        assert_eq!(cwd, PathBuf::from("/ctx/ws"));
    }

    #[test]
    fn probe_cwd_uses_container_managed_worktree_root_for_disk_isolated_worktree() {
        let workspace = sample_workspace("/host/workspace");
        let worktree_id = WorktreeId(Uuid::new_v4());
        let worktree = Worktree {
            id: worktree_id,
            workspace_id: workspace.id,
            root_path: managed_worktree_path(Path::new("/ctx"), workspace.id, worktree_id)
                .to_string_lossy()
                .to_string(),
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
        };
        let data_plane = WorktreeDataPlane {
            binding: None,
            workspace: workspace.clone(),
            execution_mode: ExecutionMode::Sandbox,
            live_workspace_root: PathBuf::from("/ctx/ws"),
            live_worktree_root: container_worktree_root(worktree_id),
        };

        let cwd = probe_cwd_for_workspace_runtime(
            &data_plane,
            &worktree,
            ExecutionMode::Sandbox,
            ContainerMountMode::DiskIsolated,
        );

        assert_eq!(cwd, container_worktree_root(worktree_id));
    }

    #[tokio::test]
    async fn finalize_workspace_probe_env_sets_home_and_xdg_dirs_for_container_probe() {
        let runtime_root = tempfile::tempdir().expect("tempdir");
        let source = ResolvedHarnessSource {
            source_kind: HarnessSourceKind::Endpoint,
            endpoint: None,
            env: HashMap::new(),
        };
        let mut env = HashMap::from([(
            "CTX_DATA_ROOT".to_string(),
            runtime_root.path().to_string_lossy().to_string(),
        )]);

        finalize_workspace_probe_env(&source, "opencode", &mut env)
            .await
            .expect("finalize probe env");

        let home = runtime_root
            .path()
            .join("providers")
            .join("opencode")
            .join("home");
        assert_eq!(
            env.get("HOME").map(String::as_str),
            Some(home.to_string_lossy().as_ref())
        );
        assert_eq!(
            env.get("XDG_CONFIG_HOME").map(String::as_str),
            Some(home.join(".config").to_string_lossy().as_ref())
        );
        assert_eq!(
            env.get("XDG_CACHE_HOME").map(String::as_str),
            Some(home.join(".cache").to_string_lossy().as_ref())
        );
        assert_eq!(
            env.get("XDG_DATA_HOME").map(String::as_str),
            Some(home.join(".local/share").to_string_lossy().as_ref())
        );
        assert_eq!(
            env.get("XDG_STATE_HOME").map(String::as_str),
            Some(home.join(".local/state").to_string_lossy().as_ref())
        );
    }

    #[tokio::test]
    async fn provider_probe_env_projects_codex_subscription_env_into_runtime_root() {
        let _env_lock = lock_env().await;
        let _guard = EnvGuard::without("CTX_CODEX_HOME");
        let data_root = tempfile::tempdir().expect("tempdir");
        let runtime_root = tempfile::tempdir().expect("tempdir");
        let state = test_state(data_root.path()).await;

        let registry = provider_accounts::CodexAccountRegistry {
            active_account_id: Some("acct-codex".to_string()),
            accounts: vec![provider_accounts::CodexAccountEntry {
                id: "acct-codex".to_string(),
                label: "Codex".to_string(),
                kind: provider_accounts::CODEX_CREDENTIAL_KIND_API_KEY.to_string(),
                email: None,
                plan_type: None,
                created_at: Utc::now(),
                last_used_at: None,
                secret_ref: None,
                endpoint_profile: provider_accounts::CodexEndpointProfile::default(),
            }],
        };
        provider_accounts::save_codex_registry(data_root.path(), &registry)
            .await
            .expect("save codex registry");
        let account_dir =
            provider_accounts::ensure_codex_account_dir(data_root.path(), "acct-codex")
                .await
                .expect("ensure codex account dir");
        tokio::fs::write(
            account_dir.join("auth.json"),
            br#"{"OPENAI_API_KEY":"codex-test-key"}"#,
        )
        .await
        .expect("write codex auth");

        let (_, env) =
            provider_env_with_runtime_root(&state, "codex", Some(runtime_root.path()), true)
                .await
                .expect("resolve probe env");

        let codex_home = env
            .get("CODEX_HOME")
            .map(String::as_str)
            .expect("missing CODEX_HOME");
        assert!(
            Path::new(codex_home).starts_with(runtime_root.path()),
            "expected runtime-root projected CODEX_HOME, got {codex_home}"
        );
        assert!(Path::new(codex_home).join("auth.json").exists());
    }

    #[tokio::test]
    async fn provider_probe_env_projects_kimi_subscription_env_into_runtime_root() {
        let data_root = tempfile::tempdir().expect("tempdir");
        let runtime_root = tempfile::tempdir().expect("tempdir");
        let state = test_state(data_root.path()).await;

        provider_accounts::add_kimi_account(
            data_root.path(),
            Some("Kimi".to_string()),
            None,
            r#"{"api_key":"kimi-key"}"#.to_string(),
            None,
            Some("kimi@example.com".to_string()),
        )
        .await
        .expect("add kimi account");

        let (_, env) =
            provider_env_with_runtime_root(&state, "kimi", Some(runtime_root.path()), true)
                .await
                .expect("resolve probe env");

        let share_dir = env
            .get(KIMI_SHARE_DIR_ENV)
            .map(String::as_str)
            .expect("missing KIMI_SHARE_DIR");
        assert!(
            Path::new(share_dir).starts_with(runtime_root.path()),
            "expected runtime-root projected KIMI_SHARE_DIR, got {share_dir}"
        );
        assert!(Path::new(share_dir)
            .join("credentials")
            .join("kimi-code.json")
            .exists());
        let config = tokio::fs::read_to_string(Path::new(share_dir).join("config.toml"))
            .await
            .expect("read kimi config");
        assert!(config.contains("default_model = \"kimi-code/kimi-for-coding\""));
    }

    #[tokio::test]
    async fn provider_probe_env_projects_mistral_subscription_env_into_runtime_root() {
        let data_root = tempfile::tempdir().expect("tempdir");
        let runtime_root = tempfile::tempdir().expect("tempdir");
        let state = test_state(data_root.path()).await;

        provider_accounts::upsert_mistral_account(
            data_root.path(),
            Some("Mistral".to_string()),
            Some("mistral@example.com".to_string()),
        )
        .await
        .expect("upsert mistral account");

        let (_, env) =
            provider_env_with_runtime_root(&state, "mistral", Some(runtime_root.path()), true)
                .await
                .expect("resolve probe env");

        let home = env.get("HOME").map(String::as_str).expect("missing HOME");
        assert!(
            Path::new(home).starts_with(runtime_root.path()),
            "expected runtime-root projected HOME, got {home}"
        );
    }

    #[tokio::test]
    async fn provider_probe_env_requires_kimi_account_env() {
        let data_root = tempfile::tempdir().expect("tempdir");
        let state = test_state(data_root.path()).await;

        let err = provider_env_with_runtime_root(&state, "kimi", None, true)
            .await
            .expect_err("missing kimi account env should fail probe");
        assert!(err.contains("subscription account env is missing"));
        assert!(err.contains("active account"));
    }

    #[tokio::test]
    async fn provider_probe_env_requires_mistral_account_env() {
        let data_root = tempfile::tempdir().expect("tempdir");
        let state = test_state(data_root.path()).await;

        let err = provider_env_with_runtime_root(&state, "mistral", None, true)
            .await
            .expect_err("missing mistral account env should fail probe");
        assert!(err.contains("subscription account env is missing"));
        assert!(err.contains("active account"));
    }
}
