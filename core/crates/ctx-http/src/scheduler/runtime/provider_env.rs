use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use serde_json::json;

use ctx_core::ids::{RunId, TurnId};
use ctx_core::models::Session;
use ctx_core::provider_ids::CODEX_PROVIDER_ID;
use ctx_provider_accounts as provider_accounts;
use ctx_provider_install::install_state::InstallTarget;

use crate::daemon::AppState;
use crate::installer;
use crate::ops_events::OpsEvent;

pub(super) struct ProviderRuntimeEnvironmentRequest<'a> {
    pub(super) state: &'a Arc<AppState>,
    pub(super) provider_env: &'a mut HashMap<String, String>,
    pub(super) runtime_provider_id: &'a str,
    pub(super) runtime_plan: &'a ctx_harness_runtime::HarnessExecutionPlan,
    pub(super) is_linux_sandbox: bool,
    pub(super) using_endpoint_source: bool,
    pub(super) adapter_cfg: &'a installer::AgentServerConfigFile,
    pub(super) install_target: InstallTarget,
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
        using_endpoint_source,
        adapter_cfg,
        install_target,
    } = request;
    if runtime_provider_id == CODEX_PROVIDER_ID && is_linux_sandbox && using_endpoint_source {
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
        && !using_endpoint_source
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
    if runtime_provider_id != CODEX_PROVIDER_ID && !using_endpoint_source {
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
    if runtime_provider_id == CODEX_PROVIDER_ID {
        let codex_home = provider_env
            .get("CODEX_HOME")
            .cloned()
            .ok_or_else(|| anyhow!("missing CODEX_HOME for {runtime_provider_id}"))?;
        provider_accounts::ensure_codex_auth_ready(Path::new(&codex_home))
        .await
        .map_err(|err| {
            if using_endpoint_source {
                anyhow!(
                    "Codex endpoint credentials are not configured correctly. Open Settings -> Agent Harnesses and verify the selected endpoint. Details: {err}"
                )
            } else {
                anyhow!(
                    "Codex authentication is not configured. Open Settings -> Codex and add a subscription login or API key. Details: {err}"
                )
            }
        })?;
        if is_linux_sandbox && using_endpoint_source {
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
