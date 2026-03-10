use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use ctx_core::ids::WorktreeId;
use ctx_core::models::{Workspace, Worktree};
use ctx_fs::worktrees::managed_worktree_path;

use crate::daemon::AppState;
use crate::disk_isolated;
use crate::execution_effective;
use crate::harness_runtime::CTX_CONTAINER_WORKSPACE_ROOT;
use crate::harness_sources::{self, HarnessSourceKind, ResolvedHarnessSource};
use crate::logs;
use crate::provider_accounts;
use crate::settings::{ContainerMountMode, ExecutionMode};

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
    let source = harness_sources::resolve_provider_source_for_probe_with_runtime_root(
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

fn probe_worktree_priority(
    data_root: &Path,
    workspace: &Workspace,
    worktree: &Worktree,
) -> Option<u8> {
    if worktree.root_path == workspace.root_path {
        return Some(0);
    }

    if Path::new(&worktree.root_path) == managed_worktree_path(data_root, workspace.id, worktree.id)
    {
        return Some(1);
    }

    None
}

fn select_probe_worktree(
    data_root: &Path,
    workspace: &Workspace,
    worktrees: &[Worktree],
) -> Result<Option<Worktree>, String> {
    if worktrees.is_empty() {
        return Ok(None);
    }

    let mut selected: Option<(u8, Worktree)> = None;
    let mut stale_paths = Vec::new();
    for worktree in worktrees {
        match probe_worktree_priority(data_root, workspace, worktree) {
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
        "container provider probe requires an explicit ctx-managed worktree; workspace '{}' has no worktree at the workspace root or ctx-managed path (found: {})",
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
    data_root: &Path,
    workspace: &Workspace,
    worktree: &Worktree,
    mode: ExecutionMode,
    mount_mode: ContainerMountMode,
) -> PathBuf {
    if !matches!(mode, ExecutionMode::Container)
        || !matches!(mount_mode, ContainerMountMode::DiskIsolated)
    {
        return PathBuf::from(&worktree.root_path);
    }

    if worktree.root_path == workspace.root_path {
        return PathBuf::from(CTX_CONTAINER_WORKSPACE_ROOT);
    }

    let managed_root = managed_worktree_path(data_root, workspace.id, worktree.id);
    if Path::new(&worktree.root_path) == managed_root {
        return disk_isolated::container_worktree_root(worktree.id);
    }

    PathBuf::from(CTX_CONTAINER_WORKSPACE_ROOT)
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

    let worktrees = state
        .global_store()
        .list_worktrees(workspace.id)
        .await
        .map_err(|err| {
            logs::redact_sensitive(&format!("loading workspace worktrees failed: {err}"))
        })?;
    let worktree = select_probe_worktree(&state.core.data_root, workspace, &worktrees)?
        .unwrap_or_else(|| synthetic_probe_worktree(workspace));
    let mode = effective.mode.clone();
    let mount_mode = effective.container.mount_mode.clone();
    let cwd = probe_cwd_for_workspace_runtime(
        &state.core.data_root,
        workspace,
        &worktree,
        mode,
        mount_mode,
    );
    let runtime_plan = state
        .execution
        .harness
        .prepare(workspace, &worktree, &effective, &state.core.daemon_url)
        .await
        .map_err(|err| {
            logs::redact_sensitive(&format!("probe runtime preparation failed: {err:#}"))
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
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use chrono::Utc;
    use ctx_core::ids::{WorkspaceId, WorktreeId};
    use ctx_core::models::{Workspace, Worktree};
    use ctx_fs::worktrees::managed_worktree_path;
    use ctx_store::StoreManager;
    use uuid::Uuid;

    use crate::daemon::AppState;
    use crate::disk_isolated;
    use crate::harness_sources::{HarnessSourceKind, ResolvedHarnessSource};
    use crate::provider_accounts;
    use crate::provider_accounts::KIMI_SHARE_DIR_ENV;
    use crate::settings::{ContainerMountMode, ExecutionMode};

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
        let selected =
            select_probe_worktree(Path::new("/ctx"), &workspace, &[first, preferred.clone()])
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

        let selected =
            select_probe_worktree(data_root.path(), &workspace, std::slice::from_ref(&managed))
                .expect("selection should succeed");
        assert_eq!(selected.expect("selected").id, managed.id);
    }

    #[test]
    fn select_probe_worktree_rejects_only_stale_worktrees() {
        let workspace = sample_workspace("/repo");
        let first = sample_worktree(workspace.id, "/repo-a");
        let second = sample_worktree(workspace.id, "/repo-b");
        let err = select_probe_worktree(Path::new("/ctx"), &workspace, &[first, second])
            .expect_err("stale worktrees should be rejected explicitly");
        assert!(err.contains("explicit ctx-managed worktree"));
    }

    #[test]
    fn select_probe_worktree_returns_none_for_empty_list() {
        let workspace = sample_workspace("/repo");
        assert!(select_probe_worktree(Path::new("/ctx"), &workspace, &[])
            .expect("empty selection should not error")
            .is_none());
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

        let selected =
            select_probe_worktree(data_root.path(), &workspace, &[first.clone(), second])
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
        let data_root = tempfile::tempdir().expect("tempdir");
        let workspace = sample_workspace("/host/workspace");
        let worktree = sample_worktree(workspace.id, "/host/workspace");

        let cwd = probe_cwd_for_workspace_runtime(
            data_root.path(),
            &workspace,
            &worktree,
            ExecutionMode::Container,
            ContainerMountMode::DiskIsolated,
        );

        assert_eq!(cwd, PathBuf::from("/ctx/ws"));
    }

    #[test]
    fn probe_cwd_uses_container_managed_worktree_root_for_disk_isolated_worktree() {
        let data_root = tempfile::tempdir().expect("tempdir");
        let workspace = sample_workspace("/host/workspace");
        let worktree_id = WorktreeId(Uuid::new_v4());
        let worktree = Worktree {
            id: worktree_id,
            workspace_id: workspace.id,
            root_path: managed_worktree_path(data_root.path(), workspace.id, worktree_id)
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

        let cwd = probe_cwd_for_workspace_runtime(
            data_root.path(),
            &workspace,
            &worktree,
            ExecutionMode::Container,
            ContainerMountMode::DiskIsolated,
        );

        assert_eq!(cwd, disk_isolated::container_worktree_root(worktree_id));
    }

    #[test]
    fn probe_cwd_keeps_host_path_for_host_mounted_container_mode() {
        let data_root = tempfile::tempdir().expect("tempdir");
        let workspace = sample_workspace("/host/workspace");
        let worktree = sample_worktree(workspace.id, "/host/workspace");

        let cwd = probe_cwd_for_workspace_runtime(
            data_root.path(),
            &workspace,
            &worktree,
            ExecutionMode::Container,
            ContainerMountMode::HostMounted,
        );

        assert_eq!(cwd, PathBuf::from("/host/workspace"));
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
