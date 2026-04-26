use std::collections::HashMap;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use ctx_core::ids::WorkspaceId;
use ctx_core::models::{Workspace, Worktree};
use ctx_core::provider_ids::{canonical_provider_id, CODEX_CRP_PROVIDER_ID};
use ctx_harness_sources::{HarnessSourceKind, ResolvedHarnessSource};
use ctx_provider_accounts as provider_accounts;

use crate::provider_auth::provider_has_active_auth_config_with_runtime_root;

#[derive(Debug, Clone)]
pub struct PreparedWorkspaceProbeRuntime {
    pub cwd: PathBuf,
    pub runtime_data_root: Option<PathBuf>,
    pub env_overrides: HashMap<String, String>,
}

pub struct WorkspaceRuntimeProbeContext {
    pub source: ResolvedHarnessSource,
    pub env: HashMap<String, String>,
    pub cwd: PathBuf,
}

#[async_trait]
pub trait ProviderProbeHost: Send + Sync + 'static {
    fn data_root(&self) -> &Path;
    fn daemon_url(&self) -> &str;
    fn auth_token(&self) -> Option<&String>;
    fn redact_sensitive(&self, input: &str) -> String;

    async fn load_workspace(&self, workspace_id: WorkspaceId) -> Result<Option<Workspace>, String>;

    async fn prepare_workspace_probe_runtime(
        &self,
        workspace: &Workspace,
    ) -> Result<PreparedWorkspaceProbeRuntime, String>;

    async fn prepare_worktree_probe_runtime(
        &self,
        workspace: &Workspace,
        worktree: &Worktree,
    ) -> Result<PreparedWorkspaceProbeRuntime, String>;
}

pub async fn provider_probe_env<H>(
    state: &H,
    provider_id: &str,
) -> Result<(ResolvedHarnessSource, HashMap<String, String>), String>
where
    H: ProviderProbeHost,
{
    provider_env_with_runtime_root(state, provider_id, None, true, false, true).await
}

async fn provider_env_with_runtime_root<H>(
    state: &H,
    provider_id: &str,
    runtime_data_root: Option<&Path>,
    require_subscription_account_env: bool,
    include_daemon_auth: bool,
    disable_mcp: bool,
) -> Result<(ResolvedHarnessSource, HashMap<String, String>), String>
where
    H: ProviderProbeHost,
{
    let provider_id = canonical_provider_id(provider_id);
    let source = ctx_harness_sources::resolve_provider_source_for_probe_with_runtime_root(
        state.data_root(),
        provider_id,
        runtime_data_root,
    )
    .await
    .map_err(|e| state.redact_sensitive(&e.to_string()))?;
    let mut env = HashMap::new();
    env.insert("CTX_DAEMON_URL".to_string(), state.daemon_url().to_string());
    if include_daemon_auth {
        if let Some(token) = state.auth_token() {
            env.insert("CTX_AUTH_TOKEN".to_string(), token.clone());
        }
    }
    if disable_mcp {
        env.insert("CTX_MCP_DISABLED".to_string(), "1".to_string());
    }
    if source.source_kind == HarnessSourceKind::Subscription {
        if require_subscription_account_env && provider_id == CODEX_CRP_PROVIDER_ID {
            let has_auth = match runtime_data_root {
                Some(runtime_root) => {
                    provider_accounts::codex_has_active_auth_with_runtime_root(
                        state.data_root(),
                        runtime_root,
                    )
                    .await
                }
                None => provider_accounts::codex_has_active_auth(state.data_root()).await,
            }
            .map_err(|err| {
                state.redact_sensitive(&format!(
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
                    state.data_root(),
                    runtime_root,
                    provider_id,
                )
                .await
            }
            None => {
                provider_accounts::subscription_env_for_active_account(
                    state.data_root(),
                    provider_id,
                )
                .await
            }
        }
        .map_err(|err| {
            state.redact_sensitive(&format!(
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

async fn finalize_workspace_probe_env<H>(
    state: &H,
    source: &ResolvedHarnessSource,
    provider_id: &str,
    env: &mut HashMap<String, String>,
) -> Result<(), String>
where
    H: ProviderProbeHost,
{
    let provider_id = canonical_provider_id(provider_id);
    if provider_id == CODEX_CRP_PROVIDER_ID && source.source_kind == HarnessSourceKind::Endpoint {
        if let Some(root) = env.get("CTX_DATA_ROOT").cloned() {
            provider_accounts::ensure_codex_endpoint_runtime_home_from_env(Path::new(&root), env)
                .await
                .map_err(|err| {
                    state.redact_sensitive(&format!(
                        "probe codex endpoint runtime-home preparation failed: {err:#}"
                    ))
                })?;
        }
    }

    if let Some(root) = env.get("CTX_DATA_ROOT").cloned() {
        provider_accounts::ensure_provider_runtime_home_env(Path::new(&root), provider_id, env)
            .await
            .map_err(|err| {
                state.redact_sensitive(&format!(
                    "probe provider runtime-home preparation failed: {err:#}"
                ))
            })?;
    }
    Ok(())
}

async fn provider_context_for_workspace_runtime<H>(
    state: &H,
    workspace: &Workspace,
    provider_id: &str,
    require_subscription_account_env: bool,
    disable_mcp: bool,
) -> Result<WorkspaceRuntimeProbeContext, String>
where
    H: ProviderProbeHost,
{
    let runtime = state.prepare_workspace_probe_runtime(workspace).await?;
    let (source, mut env) = provider_env_with_runtime_root(
        state,
        provider_id,
        runtime.runtime_data_root.as_deref(),
        require_subscription_account_env,
        false,
        disable_mcp,
    )
    .await?;
    for (key, value) in runtime.env_overrides {
        env.insert(key, value);
    }
    finalize_workspace_probe_env(state, &source, provider_id, &mut env).await?;
    Ok(WorkspaceRuntimeProbeContext {
        source,
        env,
        cwd: runtime.cwd,
    })
}

pub async fn provider_auth_context_for_worktree_runtime<H>(
    state: &H,
    worktree: &Worktree,
    provider_id: &str,
) -> Result<WorkspaceRuntimeProbeContext, String>
where
    H: ProviderProbeHost,
{
    let workspace = state
        .load_workspace(worktree.workspace_id)
        .await?
        .ok_or_else(|| "workspace not found".to_string())?;
    let runtime = state
        .prepare_worktree_probe_runtime(&workspace, worktree)
        .await?;
    let (source, mut env) = provider_env_with_runtime_root(
        state,
        provider_id,
        runtime.runtime_data_root.as_deref(),
        false,
        true,
        false,
    )
    .await?;
    for (key, value) in runtime.env_overrides {
        env.insert(key, value);
    }
    finalize_workspace_probe_env(state, &source, provider_id, &mut env).await?;
    Ok(WorkspaceRuntimeProbeContext {
        source,
        env,
        cwd: runtime.cwd,
    })
}

pub async fn provider_probe_context_for_workspace_runtime<H>(
    state: &H,
    workspace: &Workspace,
    provider_id: &str,
) -> Result<WorkspaceRuntimeProbeContext, String>
where
    H: ProviderProbeHost,
{
    provider_context_for_workspace_runtime(state, workspace, provider_id, true, false).await
}

pub async fn provider_auth_context_for_workspace_runtime<H>(
    state: &H,
    workspace: &Workspace,
    provider_id: &str,
) -> Result<WorkspaceRuntimeProbeContext, String>
where
    H: ProviderProbeHost,
{
    provider_context_for_workspace_runtime(state, workspace, provider_id, false, true).await
}

pub async fn provider_has_active_auth_for_workspace_runtime<H>(
    state: &H,
    workspace: &Workspace,
    provider_id: &str,
    source_config: Option<&ctx_harness_sources::HarnessProviderSourceConfig>,
) -> Result<bool, String>
where
    H: ProviderProbeHost,
{
    let runtime_root =
        provider_context_for_workspace_runtime(state, workspace, provider_id, false, false)
            .await?
            .env
            .get("CTX_DATA_ROOT")
            .map(PathBuf::from);
    provider_has_active_auth_config_with_runtime_root(
        state.data_root(),
        runtime_root.as_deref(),
        provider_id,
        source_config,
    )
    .await
}

pub async fn provider_probe_env_for_workspace_runtime<H>(
    state: &H,
    workspace: &Workspace,
    provider_id: &str,
) -> Result<(ResolvedHarnessSource, HashMap<String, String>), String>
where
    H: ProviderProbeHost,
{
    let context =
        provider_probe_context_for_workspace_runtime(state, workspace, provider_id).await?;
    Ok((context.source, context.env))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use ctx_core::ids::WorkspaceId;
    use ctx_harness_sources::{
        mark_endpoint_verification, set_provider_source_selection, upsert_provider_endpoint,
        HarnessApiShape, HarnessEndpointUpsert, HarnessEndpointVerificationStatus,
        HarnessSourceKind,
    };
    use std::sync::Arc;

    #[derive(Clone)]
    struct TestProbeHost {
        data_root: PathBuf,
        daemon_url: String,
        workspace: Arc<Workspace>,
        runtime: PreparedWorkspaceProbeRuntime,
    }

    #[async_trait]
    impl ProviderProbeHost for TestProbeHost {
        fn data_root(&self) -> &Path {
            &self.data_root
        }

        fn daemon_url(&self) -> &str {
            &self.daemon_url
        }

        fn auth_token(&self) -> Option<&String> {
            None
        }

        fn redact_sensitive(&self, input: &str) -> String {
            input.to_string()
        }

        async fn load_workspace(
            &self,
            workspace_id: WorkspaceId,
        ) -> Result<Option<Workspace>, String> {
            if self.workspace.id == workspace_id {
                Ok(Some((*self.workspace).clone()))
            } else {
                Ok(None)
            }
        }

        async fn prepare_workspace_probe_runtime(
            &self,
            _workspace: &Workspace,
        ) -> Result<PreparedWorkspaceProbeRuntime, String> {
            Ok(self.runtime.clone())
        }

        async fn prepare_worktree_probe_runtime(
            &self,
            _workspace: &Workspace,
            _worktree: &Worktree,
        ) -> Result<PreparedWorkspaceProbeRuntime, String> {
            Ok(self.runtime.clone())
        }
    }

    #[tokio::test]
    async fn codex_endpoint_probe_runtime_preserves_openai_compatible_base_url_without_model_provider(
    ) {
        let root = tempfile::tempdir().expect("tempdir");
        let data_root = root.path().join("data-root");
        let runtime_root = root.path().join("runtime-root");
        let workspace_root = root.path().join("workspace");
        tokio::fs::create_dir_all(&data_root)
            .await
            .expect("create data root");
        tokio::fs::create_dir_all(&runtime_root)
            .await
            .expect("create runtime root");
        tokio::fs::create_dir_all(&workspace_root)
            .await
            .expect("create workspace root");

        let endpoint = upsert_provider_endpoint(
            &data_root,
            "codex",
            HarnessEndpointUpsert {
                endpoint_id: None,
                name: "OpenRouter".to_string(),
                base_url: Some("https://openrouter.ai/api/v1".to_string()),
                api_shape: Some(HarnessApiShape::OpenaiResponses),
                auth_type: None,
                model_override: Some("openai/gpt-4.1-mini".to_string()),
                api_key: Some("sk-or-test".to_string()),
                service_account_json: None,
                project_id: None,
                location: None,
            },
        )
        .await
        .expect("upsert endpoint");
        set_provider_source_selection(
            &data_root,
            "codex",
            HarnessSourceKind::Endpoint,
            Some(endpoint.id.clone()),
        )
        .await
        .expect("select endpoint");
        mark_endpoint_verification(
            &data_root,
            "codex",
            &endpoint.id,
            HarnessEndpointVerificationStatus::Valid,
            None,
        )
        .await
        .expect("mark verified");

        let workspace = Arc::new(Workspace {
            id: WorkspaceId::new(),
            name: "ws".to_string(),
            root_path: workspace_root.to_string_lossy().to_string(),
            created_at: Utc::now(),
            vcs_kind: None,
        });
        let host = TestProbeHost {
            data_root: data_root.clone(),
            daemon_url: "http://127.0.0.1:0".to_string(),
            workspace: workspace.clone(),
            runtime: PreparedWorkspaceProbeRuntime {
                cwd: workspace_root.clone(),
                runtime_data_root: Some(runtime_root.clone()),
                env_overrides: HashMap::from([(
                    "CTX_DATA_ROOT".to_string(),
                    runtime_root.to_string_lossy().to_string(),
                )]),
            },
        };

        let context = provider_auth_context_for_workspace_runtime(&host, &workspace, "codex")
            .await
            .expect("probe context");

        assert_eq!(context.source.source_kind, HarnessSourceKind::Endpoint);
        assert_eq!(
            context.env.get("CTX_MODEL_PROVIDER").map(String::as_str),
            None
        );
        assert_eq!(
            context.env.get("OPENAI_BASE_URL").map(String::as_str),
            Some("https://openrouter.ai/api/v1")
        );

        let codex_home = PathBuf::from(
            context
                .env
                .get("CODEX_HOME")
                .cloned()
                .expect("runtime CODEX_HOME"),
        );
        assert!(
            codex_home.starts_with(&runtime_root),
            "expected runtime CODEX_HOME under runtime root, got {}",
            codex_home.display()
        );
        let auth = tokio::fs::read_to_string(codex_home.join("auth.json"))
            .await
            .expect("runtime auth json");
        assert!(auth.contains("\"OPENAI_BASE_URL\": \"https://openrouter.ai/api/v1\""));
    }
}
