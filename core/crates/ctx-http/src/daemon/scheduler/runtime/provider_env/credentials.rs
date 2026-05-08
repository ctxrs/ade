use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use ctx_core::provider_ids::CODEX_PROVIDER_ID;
use ctx_harness_sources::{HarnessRouteBackend, HarnessRuntimeSourceMode};
use ctx_provider_accounts as provider_accounts;
use ctx_provider_install::install_state::InstallTarget;

use crate::daemon::installer;
use crate::daemon::AppState;

pub(in crate::daemon::scheduler::runtime) struct ProviderRuntimeEnvironmentRequest<'a> {
    pub(in crate::daemon::scheduler::runtime) state: &'a Arc<AppState>,
    pub(in crate::daemon::scheduler::runtime) provider_env: &'a mut HashMap<String, String>,
    pub(in crate::daemon::scheduler::runtime) runtime_provider_id: &'a str,
    pub(in crate::daemon::scheduler::runtime) runtime_plan:
        &'a ctx_harness_runtime::HarnessExecutionPlan,
    pub(in crate::daemon::scheduler::runtime) is_linux_sandbox: bool,
    pub(in crate::daemon::scheduler::runtime) runtime_source_mode: HarnessRuntimeSourceMode,
    pub(in crate::daemon::scheduler::runtime) adapter_cfg: &'a installer::AgentServerConfigFile,
    pub(in crate::daemon::scheduler::runtime) install_target: InstallTarget,
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

pub(in crate::daemon::scheduler::runtime) async fn prepare_provider_runtime_environment(
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
