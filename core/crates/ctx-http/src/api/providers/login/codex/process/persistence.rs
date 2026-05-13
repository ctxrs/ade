use super::*;

pub(in crate::api::providers::login::codex) async fn persist_successful_codex_login(
    state: &Arc<AppState>,
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
        return Err(err);
    }

    Ok(())
}
