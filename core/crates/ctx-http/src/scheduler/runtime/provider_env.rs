use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use serde_json::json;

use ctx_core::ids::{RunId, TurnId};
use ctx_core::models::Session;
use ctx_core::provider_ids::CODEX_PROVIDER_ID;
use ctx_harness_sources::{
    HarnessRouteBackend, HarnessRuntimeSourceMode, HarnessSourceKind, ResolvedHarnessSource,
};
use ctx_provider_accounts as provider_accounts;
use ctx_provider_install::install_state::InstallTarget;

use crate::daemon::AppState;
use crate::installer;
use crate::ops_events::OpsEvent;
use crate::settings::ProviderControlMode;

pub(super) struct BaseProviderEnvRequest<'a> {
    pub(super) daemon_url: &'a str,
    pub(super) data_root: &'a Path,
    pub(super) session: &'a Session,
    pub(super) full_model_id: &'a str,
    pub(super) provider_control_mode: &'a ProviderControlMode,
}

pub(super) fn build_base_provider_env(
    request: BaseProviderEnvRequest<'_>,
) -> HashMap<String, String> {
    let BaseProviderEnvRequest {
        daemon_url,
        data_root,
        session,
        full_model_id,
        provider_control_mode,
    } = request;
    let mut provider_env = HashMap::new();
    provider_env.insert("CTX_DAEMON_URL".to_string(), daemon_url.to_string());
    provider_env.insert(
        "CTX_DATA_ROOT".to_string(),
        data_root.to_string_lossy().to_string(),
    );
    provider_env.insert("CTX_PROVIDER_ID".to_string(), session.provider_id.clone());
    provider_env.insert(
        "CLAUDE_CODE_ENABLE_ASK_USER_QUESTION_TOOL".to_string(),
        "1".to_string(),
    );
    if let Some(provider_ref) = session.provider_session_ref.clone() {
        provider_env.insert("CTX_PROVIDER_SESSION_REF".to_string(), provider_ref);
    }
    provider_env.insert("CTX_SESSION_ID".to_string(), session.id.0.to_string());
    provider_env.insert(
        "CTX_WORKTREE_ID".to_string(),
        session.worktree_id.0.to_string(),
    );
    provider_env.insert("CTX_MODEL_ID".to_string(), full_model_id.to_string());
    if let Some(mode_id) = provider_mode_id_for(&session.provider_id, provider_control_mode) {
        provider_env.insert("CTX_PROVIDER_MODE".to_string(), mode_id.to_string());
    }
    if let Ok(v) = std::env::var("CTX_MCP_COMMAND") {
        provider_env.insert("CTX_MCP_COMMAND".to_string(), v);
    }
    if let Ok(v) = std::env::var("CTX_MCP_DISABLED") {
        provider_env.insert("CTX_MCP_DISABLED".to_string(), v);
    }
    provider_env
}

pub(super) fn provider_mode_id_for(
    provider_id: &str,
    control_mode: &ProviderControlMode,
) -> Option<&'static str> {
    match control_mode {
        ProviderControlMode::Full => match provider_id {
            CODEX_PROVIDER_ID => Some("full-access"),
            "claude-crp" => Some("bypassPermissions"),
            "droid" => Some("auto_high"),
            _ => None,
        },
        ProviderControlMode::HarnessNative | ProviderControlMode::CtxEnforced => None,
    }
}

pub(super) struct ProviderSourceEnvResolution {
    pub(super) resolved_source: ResolvedHarnessSource,
    pub(super) runtime_source_mode: HarnessRuntimeSourceMode,
    pub(super) using_endpoint_source: bool,
}

pub(super) async fn apply_runtime_source_env(
    data_root: &Path,
    provider_id: &str,
    runtime_plan: &ctx_harness_runtime::HarnessExecutionPlan,
    provider_env: &mut HashMap<String, String>,
) -> Result<ProviderSourceEnvResolution> {
    for (key, value) in runtime_plan.env_overrides.iter() {
        provider_env.insert(key.clone(), value.clone());
    }
    ctx_mcp_command::configure_runtime_mcp_command(provider_id, provider_env, data_root)?;

    let runtime_data_root = runtime_plan.runtime_data_root();
    let resolved_source = ctx_harness_sources::resolve_provider_source_for_run_with_runtime_root(
        data_root,
        provider_id,
        runtime_data_root,
    )
    .await
    .map_err(|err| anyhow!("provider source resolution failed for {provider_id}: {err}"))?;
    let runtime_source_mode = resolved_source.runtime_source_mode();
    let using_endpoint_source = runtime_source_mode.source_kind() == HarnessSourceKind::Endpoint;
    provider_env.insert(
        "CTX_PROVIDER_SOURCE_KIND".to_string(),
        match resolved_source.source_kind {
            HarnessSourceKind::Subscription => "subscription".to_string(),
            HarnessSourceKind::Endpoint => "endpoint".to_string(),
        },
    );
    if let Some(endpoint) = resolved_source.endpoint.as_ref() {
        provider_env.insert("CTX_PROVIDER_ENDPOINT_ID".to_string(), endpoint.id.clone());
        provider_env.insert(
            "CTX_PROVIDER_ENDPOINT_SHAPE".to_string(),
            endpoint.api_shape.as_str().to_string(),
        );
    }
    for (key, value) in resolved_source.env.iter() {
        provider_env.insert(key.clone(), value.clone());
    }

    Ok(ProviderSourceEnvResolution {
        resolved_source,
        runtime_source_mode,
        using_endpoint_source,
    })
}

pub(super) struct ProviderRuntimeEnvironmentRequest<'a> {
    pub(super) state: &'a Arc<AppState>,
    pub(super) provider_env: &'a mut HashMap<String, String>,
    pub(super) runtime_provider_id: &'a str,
    pub(super) runtime_plan: &'a ctx_harness_runtime::HarnessExecutionPlan,
    pub(super) is_linux_sandbox: bool,
    pub(super) runtime_source_mode: HarnessRuntimeSourceMode,
    pub(super) adapter_cfg: &'a installer::AgentServerConfigFile,
    pub(super) install_target: InstallTarget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderRuntimeCredentialMode {
    Subscription,
    UserManagedEndpoint,
    CtxManagedRelay,
}

fn provider_runtime_credential_mode(
    runtime_source_mode: HarnessRuntimeSourceMode,
) -> ProviderRuntimeCredentialMode {
    match runtime_source_mode {
        HarnessRuntimeSourceMode::Subscription => ProviderRuntimeCredentialMode::Subscription,
        HarnessRuntimeSourceMode::Endpoint(HarnessRouteBackend::UserManaged) => {
            ProviderRuntimeCredentialMode::UserManagedEndpoint
        }
        HarnessRuntimeSourceMode::Endpoint(HarnessRouteBackend::CtxManagedRelay) => {
            ProviderRuntimeCredentialMode::CtxManagedRelay
        }
    }
}

pub(super) async fn prepare_provider_runtime_environment(
    request: ProviderRuntimeEnvironmentRequest<'_>,
) -> Result<()> {
    let ProviderRuntimeEnvironmentRequest {
        state,
        provider_env,
        runtime_provider_id,
        runtime_plan,
        is_linux_sandbox,
        runtime_source_mode,
        adapter_cfg,
        install_target,
    } = request;
    let credential_mode = provider_runtime_credential_mode(runtime_source_mode);
    let using_user_managed_endpoint_source =
        credential_mode == ProviderRuntimeCredentialMode::UserManagedEndpoint;
    let using_ctx_managed_relay = credential_mode == ProviderRuntimeCredentialMode::CtxManagedRelay;
    let using_subscription_source = credential_mode == ProviderRuntimeCredentialMode::Subscription;

    if runtime_provider_id == CODEX_PROVIDER_ID
        && is_linux_sandbox
        && using_user_managed_endpoint_source
    {
        if let Some(root) = runtime_plan.env_overrides.get("CTX_DATA_ROOT") {
            provider_accounts::ensure_codex_endpoint_runtime_home_from_env(
                Path::new(root),
                provider_env,
            )
            .await?;
        }
    }

    if runtime_provider_id == CODEX_PROVIDER_ID
        && !provider_env.contains_key("CODEX_HOME")
        && using_subscription_source
    {
        if is_linux_sandbox {
            if let Some(root) = runtime_plan.env_overrides.get("CTX_DATA_ROOT") {
                let codex_home = provider_accounts::codex_runtime_home(Path::new(root));
                tokio::fs::create_dir_all(&codex_home).await.ok();
                provider_accounts::seed_codex_auth_from_host(&codex_home).await?;
                provider_env.insert(
                    "CODEX_HOME".to_string(),
                    codex_home.to_string_lossy().to_string(),
                );
            }
        } else {
            let env =
                provider_accounts::codex_env_for_active_account(&state.core.data_root).await?;
            for (key, value) in env {
                provider_env.insert(key, value);
            }
        }
    }
    if runtime_provider_id != CODEX_PROVIDER_ID && using_subscription_source {
        let env = if is_linux_sandbox {
            if let Some(root) = runtime_plan.env_overrides.get("CTX_DATA_ROOT") {
                provider_accounts::subscription_env_for_active_account_with_runtime_root(
                    &state.core.data_root,
                    Path::new(root),
                    runtime_provider_id,
                )
                .await?
            } else {
                provider_accounts::subscription_env_for_active_account(
                    &state.core.data_root,
                    runtime_provider_id,
                )
                .await?
            }
        } else {
            provider_accounts::subscription_env_for_active_account(
                &state.core.data_root,
                runtime_provider_id,
            )
            .await?
        };
        for (key, value) in env {
            provider_env.insert(key, value);
        }
    }
    if runtime_provider_id == CODEX_PROVIDER_ID && !using_ctx_managed_relay {
        let codex_home = provider_env
            .get("CODEX_HOME")
            .cloned()
            .ok_or_else(|| anyhow!("missing CODEX_HOME for {runtime_provider_id}"))?;
        provider_accounts::ensure_codex_auth_ready(Path::new(&codex_home))
        .await
        .map_err(|err| {
            if using_user_managed_endpoint_source {
                anyhow!(
                    "Codex endpoint credentials are not configured correctly. Open Settings -> Agent Harnesses and verify the selected endpoint. Details: {err}"
                )
            } else {
                anyhow!(
                    "Codex authentication is not configured. Open Settings -> Codex and add a subscription login or API key. Details: {err}"
                )
            }
        })?;
        if is_linux_sandbox && using_user_managed_endpoint_source {
            let openai_api_key_present = provider_env
                .get("OPENAI_API_KEY")
                .is_some_and(|value| !value.trim().is_empty());
            if !openai_api_key_present {
                anyhow::bail!(
                "codex endpoint container runtime missing OPENAI_API_KEY after endpoint resolution"
            );
            }
            if let Some(root) = runtime_plan.env_overrides.get("CTX_DATA_ROOT") {
                let expected_home = provider_accounts::codex_runtime_home(Path::new(root));
                if Path::new(&codex_home) != expected_home {
                    anyhow::bail!(
                        "codex endpoint container runtime must use CODEX_HOME={} but resolved {}",
                        expected_home.display(),
                        codex_home
                    );
                }
            }
        }
    }

    if is_linux_sandbox {
        if let Some(root) = runtime_plan.env_overrides.get("CTX_DATA_ROOT") {
            provider_accounts::ensure_provider_runtime_home_env(
                Path::new(root),
                runtime_provider_id,
                provider_env,
            )
            .await?;
        }
    }

    installer::prepend_runtime_bin_dirs_to_provider_path_for_target(
        provider_env,
        adapter_cfg,
        runtime_provider_id,
        &state.core.data_root,
        Some(install_target),
    );
    installer::ensure_codex_cli_command_env_for_target(
        provider_env,
        adapter_cfg,
        runtime_provider_id,
        Some(install_target),
    )?;

    Ok(())
}

pub(super) struct ProviderRunEnvReadyEvent<'a> {
    pub(super) state: &'a Arc<AppState>,
    pub(super) session: &'a Session,
    pub(super) run_id: RunId,
    pub(super) turn_id: TurnId,
    pub(super) workdir_str: &'a str,
    pub(super) full_model_id: &'a str,
    pub(super) execution_environment: &'a str,
    pub(super) session_root_kind: &'a str,
    pub(super) runtime_provider_id: &'a str,
    pub(super) using_endpoint_source: bool,
    pub(super) is_linux_sandbox: bool,
    pub(super) runtime_plan: &'a ctx_harness_runtime::HarnessExecutionPlan,
    pub(super) provider_env: &'a HashMap<String, String>,
}

pub(super) fn emit_provider_run_env_ready_event(event: ProviderRunEnvReadyEvent<'_>) {
    let ProviderRunEnvReadyEvent {
        state,
        session,
        run_id,
        turn_id,
        workdir_str,
        full_model_id,
        execution_environment,
        session_root_kind,
        runtime_provider_id,
        using_endpoint_source,
        is_linux_sandbox,
        runtime_plan,
        provider_env,
    } = event;
    let mut run_env_event = OpsEvent::new("info", "provider_run_env_ready");
    run_env_event.session_id = Some(session.id.0.to_string());
    run_env_event.worktree_id = Some(session.worktree_id.0.to_string());
    run_env_event.run_id = Some(run_id.0.to_string());
    run_env_event.turn_id = Some(turn_id.0.to_string());
    run_env_event.provider_id = Some(session.provider_id.clone());
    run_env_event.cwd = Some(workdir_str.to_string());
    run_env_event.worktree_root = Some(workdir_str.to_string());
    run_env_event.meta = Some(json!({
        "model_id": full_model_id,
        "reasoning_effort": session.reasoning_effort.clone(),
        "execution_environment": execution_environment,
        "session_root_kind": session_root_kind,
        "runtime_provider_id": runtime_provider_id,
        "source_kind": if using_endpoint_source { "endpoint" } else { "subscription" },
        "is_container": is_linux_sandbox,
        "runtime_kind": runtime_plan
            .env_overrides
            .get(ctx_harness_runtime::CTX_HARNESS_RUNTIME_KIND_ENV)
            .cloned()
            .unwrap_or_else(|| "host".to_string()),
        "has_openai_api_key": provider_env
            .get("OPENAI_API_KEY")
            .is_some_and(|value| !value.trim().is_empty()),
        "has_codex_home": provider_env
            .get("CODEX_HOME")
            .is_some_and(|value| !value.trim().is_empty()),
        "openai_base_url_host": provider_env
            .get("OPENAI_BASE_URL")
            .and_then(|value| url::Url::parse(value).ok())
            .and_then(|parsed| parsed.host_str().map(|host| host.to_string())),
    }));
    state.telemetry.ops_events.emit(run_env_event);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctx_managed_relay_does_not_use_subscription_or_endpoint_credentials() {
        assert_eq!(
            provider_runtime_credential_mode(HarnessRuntimeSourceMode::Endpoint(
                HarnessRouteBackend::CtxManagedRelay
            )),
            ProviderRuntimeCredentialMode::CtxManagedRelay
        );
        assert_eq!(
            provider_runtime_credential_mode(HarnessRuntimeSourceMode::Endpoint(
                HarnessRouteBackend::UserManaged
            )),
            ProviderRuntimeCredentialMode::UserManagedEndpoint
        );
        assert_eq!(
            provider_runtime_credential_mode(HarnessRuntimeSourceMode::Subscription),
            ProviderRuntimeCredentialMode::Subscription
        );
    }
}
