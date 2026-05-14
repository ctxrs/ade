use std::{path::PathBuf, sync::Arc};

use anyhow::Context;
use chrono::Utc;
use ctx_provider_accounts as provider_accounts;

use super::ProviderAccountMutationError;
use crate::daemon::DaemonState;

pub(crate) struct CodexAccountsSnapshot {
    pub(crate) active_account_id: Option<String>,
    pub(crate) accounts: Vec<provider_accounts::CodexAccountEntry>,
    pub(crate) logins: Vec<provider_accounts::CodexLoginStatus>,
}

pub(crate) struct PreparedCodexLoginStart {
    pub(crate) account_id: String,
    pub(crate) label: String,
    pub(crate) account_dir: PathBuf,
    pub(crate) codex_bin: String,
}

pub(crate) async fn load_codex_account_registry(
    state: &Arc<DaemonState>,
) -> anyhow::Result<provider_accounts::CodexAccountRegistry> {
    provider_accounts::load_codex_registry(&state.core.data_root).await
}

pub(crate) async fn load_codex_accounts_snapshot(
    state: &Arc<DaemonState>,
) -> anyhow::Result<CodexAccountsSnapshot> {
    let registry = load_codex_account_registry(state).await?;
    let logins = crate::daemon::providers::codex_login_statuses(state).await;
    Ok(CodexAccountsSnapshot {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
        logins,
    })
}

pub(crate) async fn probe_host_codex_auth_candidate() -> provider_accounts::CodexHostImportProbe {
    provider_accounts::probe_host_codex_auth_candidate().await
}

pub(crate) async fn prepare_codex_login_start(
    state: &Arc<DaemonState>,
    label: Option<String>,
) -> anyhow::Result<PreparedCodexLoginStart> {
    let account_id = uuid::Uuid::new_v4().to_string();
    let label = provider_accounts::normalize_label(label, &account_id);
    let account_dir =
        provider_accounts::ensure_codex_account_dir(&state.core.data_root, &account_id)
            .await
            .with_context(|| format!("creating codex account directory for {account_id}"))?;

    let prep_result = async {
        let (cfg, managed_config_error) = ctx_provider_runtime::provider_launch::config::load_managed_agent_server_config_with_error(
            &state.core.data_root,
        )
        .await;
        if let Some(error) = managed_config_error {
            anyhow::bail!(error);
        }
        let codex_bin = ctx_managed_installs::require_codex_cli_command_path_for_target(
            &cfg,
            Some(ctx_provider_install::install_state::InstallTarget::Host),
        )
        .context("resolving managed Codex CLI command")?;
        Ok(codex_bin)
    }
    .await;

    match prep_result {
        Ok(codex_bin) => Ok(PreparedCodexLoginStart {
            account_id,
            label,
            account_dir,
            codex_bin,
        }),
        Err(err) => {
            let _ = tokio::fs::remove_dir_all(&account_dir).await;
            Err(err)
        }
    }
}

pub(crate) async fn import_host_codex_auth(
    state: &Arc<DaemonState>,
    label: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::import_host_codex_auth_to_secret_store(&state.core.data_root, label)
        .await
        .map_err(ProviderAccountMutationError::BadRequest)?;
    crate::daemon::providers::restart_codex_providers_for_auth_change(state, "codex auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub(crate) async fn set_active_codex_account(
    state: &Arc<DaemonState>,
    account_id: Option<String>,
) -> Result<CodexAccountsSnapshot, ProviderAccountMutationError> {
    provider_accounts::set_active_codex_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::BadRequest)?;
    crate::daemon::providers::restart_codex_providers_for_auth_change(state, "codex auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)?;
    load_codex_accounts_snapshot(state)
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub(crate) async fn remove_codex_account(
    state: &Arc<DaemonState>,
    account_id: &str,
) -> Result<CodexAccountsSnapshot, ProviderAccountMutationError> {
    let previous_active =
        provider_accounts::begin_codex_account_deletion(&state.core.data_root, account_id)
            .await
            .map_err(ProviderAccountMutationError::Delete)?;
    let stop_result = crate::daemon::providers::stop_codex_providers_for_auth_removal(
        state,
        "codex auth account removed",
    )
    .await;
    if let Err(err) = stop_result {
        if let Err(rollback_err) = provider_accounts::abort_codex_account_deletion(
            &state.core.data_root,
            account_id,
            previous_active,
        )
        .await
        {
            tracing::warn!(
                account_id,
                error = %rollback_err,
                "failed to roll back Codex account deletion marker after provider stop failure"
            );
        }
        return Err(ProviderAccountMutationError::Internal(err));
    }
    provider_accounts::cleanup_codex_account_broker_home(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::Delete)?;
    let registry = provider_accounts::remove_codex_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::Delete)?;
    if let Err(err) =
        provider_accounts::finish_codex_account_deletion(&state.core.data_root, account_id).await
    {
        tracing::warn!(
            account_id,
            error = %err,
            "failed to remove Codex account deletion marker after account deletion"
        );
    }
    let logins = crate::daemon::providers::remove_codex_login_session(state, account_id).await;
    Ok(CodexAccountsSnapshot {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
        logins,
    })
}

pub(crate) async fn persist_successful_codex_login(
    state: &Arc<DaemonState>,
    account_id: &str,
    label: String,
    email: Option<String>,
    plan_type: Option<String>,
) -> anyhow::Result<()> {
    let entry = provider_accounts::CodexAccountEntry {
        id: account_id.to_string(),
        label,
        kind: provider_accounts::CODEX_CREDENTIAL_KIND_OAUTH.to_string(),
        email,
        provider_account_id: None,
        plan_type,
        created_at: Utc::now(),
        last_used_at: Some(Utc::now()),
        secret_ref: None,
        endpoint_profile: provider_accounts::CodexEndpointProfile::default(),
    };
    provider_accounts::upsert_codex_account(&state.core.data_root, entry)
        .await
        .with_context(|| format!("persisting codex account {account_id}"))?;

    let persist_result = async {
        let ingested = provider_accounts::ingest_codex_account_auth_to_secret_store(
            &state.core.data_root,
            account_id,
        )
        .await
        .with_context(|| format!("ingesting codex auth for account {account_id}"))?;
        if !ingested {
            anyhow::bail!("missing persisted codex auth file for account {account_id}");
        }
        provider_accounts::remove_codex_account_home_auth_if_present(
            &state.core.data_root,
            account_id,
        )
        .await
        .with_context(|| format!("removing account-home codex auth for account {account_id}"))?;
        provider_accounts::set_active_codex_account(
            &state.core.data_root,
            Some(account_id.to_string()),
        )
        .await
        .with_context(|| format!("setting active codex account {account_id}"))?;
        crate::daemon::providers::restart_codex_providers_for_auth_change(
            state,
            "codex auth updated",
        )
        .await
        .context("restarting codex providers after auth change")?;
        Ok(())
    }
    .await;

    if let Err(err) = persist_result {
        let _ = provider_accounts::remove_codex_account(&state.core.data_root, account_id).await;
        let _ =
            provider_accounts::cleanup_codex_account_broker_home(&state.core.data_root, account_id)
                .await;
        return Err(err);
    }

    Ok(())
}
