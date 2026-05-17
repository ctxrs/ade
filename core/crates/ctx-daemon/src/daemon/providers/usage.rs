use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Utc};
use ctx_core::provider_ids::CODEX_PROVIDER_ID;
use ctx_provider_runtime::provider_usage;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::daemon::{DaemonState, ProvidersHandle};

#[derive(Debug, Default, Deserialize)]
pub struct ProviderUsageRouteQuery {
    refresh: Option<bool>,
}

impl ProviderUsageRouteQuery {
    fn refresh(&self) -> bool {
        self.refresh.unwrap_or(false)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderUsageRouteSnapshot {
    provider_id: String,
    source: String,
    fetched_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    payload: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl From<provider_usage::ProviderUsageSnapshot> for ProviderUsageRouteSnapshot {
    fn from(snapshot: provider_usage::ProviderUsageSnapshot) -> Self {
        Self {
            provider_id: snapshot.provider_id,
            source: snapshot.source,
            fetched_at: snapshot.fetched_at,
            payload: snapshot.payload,
            error: snapshot.error,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CodexAccountUsageRouteEntry {
    account_id: Option<String>,
    label: String,
    email: Option<String>,
    plan_type: Option<String>,
    last_used_at: Option<DateTime<Utc>>,
    usage: ProviderUsageRouteSnapshot,
}

impl From<CodexAccountUsageRecord> for CodexAccountUsageRouteEntry {
    fn from(record: CodexAccountUsageRecord) -> Self {
        Self {
            account_id: record.account_id,
            label: record.label,
            email: record.email,
            plan_type: record.plan_type,
            last_used_at: record.last_used_at,
            usage: record.usage.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CodexAccountsUsageRouteResponse {
    entries: Vec<CodexAccountUsageRouteEntry>,
}

#[derive(Debug)]
pub struct ProviderUsageRouteError {
    message: String,
}

impl ProviderUsageRouteError {
    pub fn message(&self) -> &str {
        &self.message
    }

    fn from_error(error: anyhow::Error) -> Self {
        Self {
            message: error.to_string(),
        }
    }
}

impl std::fmt::Display for ProviderUsageRouteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl ProvidersHandle {
    pub async fn provider_usage_for_route(
        &self,
        provider_id: &str,
        query: ProviderUsageRouteQuery,
    ) -> Result<ProviderUsageRouteSnapshot, ProviderUsageRouteError> {
        load_provider_usage(&self.state, provider_id, query.refresh())
            .await
            .map(Into::into)
            .map_err(ProviderUsageRouteError::from_error)
    }

    pub async fn codex_accounts_usage_for_route(
        &self,
        query: ProviderUsageRouteQuery,
    ) -> Result<CodexAccountsUsageRouteResponse, ProviderUsageRouteError> {
        let entries = load_codex_accounts_usage(&self.state, query.refresh())
            .await
            .map_err(ProviderUsageRouteError::from_error)?
            .into_iter()
            .map(Into::into)
            .collect();
        Ok(CodexAccountsUsageRouteResponse { entries })
    }
}

async fn provider_usage_env(
    state: &Arc<DaemonState>,
    provider_id: &str,
) -> Result<HashMap<String, String>> {
    if provider_id != CODEX_PROVIDER_ID {
        return Ok(HashMap::new());
    }

    let mut env =
        ctx_provider_accounts::codex_usage_env_for_active_account(&state.core.data_root).await?;
    let (cfg, config_error) =
        ctx_provider_runtime::provider_launch::config::load_managed_agent_server_config_with_error(
            &state.core.data_root,
        )
        .await;
    if let Some(config_error) = config_error {
        anyhow::bail!(config_error);
    }
    ctx_managed_installs::ensure_codex_cli_command_env_for_target(
        &mut env,
        &cfg,
        CODEX_PROVIDER_ID,
        Some(ctx_provider_install::InstallTarget::Host),
    )?;
    Ok(env)
}

async fn load_provider_usage(
    state: &Arc<DaemonState>,
    provider_id: &str,
    refresh: bool,
) -> Result<provider_usage::ProviderUsageSnapshot> {
    let env = provider_usage_env(state, provider_id).await?;
    if !refresh {
        if let Some(snapshot) = state
            .providers
            .provider_usage_cache_entry(provider_id)
            .await
        {
            return Ok(snapshot);
        }
    }
    provider_usage::refresh_provider_usage_for(state.as_ref(), provider_id, env).await
}

struct CodexAccountUsageRecord {
    account_id: Option<String>,
    label: String,
    email: Option<String>,
    plan_type: Option<String>,
    last_used_at: Option<DateTime<Utc>>,
    usage: provider_usage::ProviderUsageSnapshot,
}

fn codex_account_usage_error(error: String) -> provider_usage::ProviderUsageSnapshot {
    provider_usage::ProviderUsageSnapshot {
        provider_id: CODEX_PROVIDER_ID.to_string(),
        source: "error".to_string(),
        fetched_at: Utc::now(),
        payload: None,
        error: Some(error),
    }
}

async fn load_codex_accounts_usage(
    state: &Arc<DaemonState>,
    refresh: bool,
) -> Result<Vec<CodexAccountUsageRecord>> {
    let registry = ctx_provider_accounts::load_codex_registry(&state.core.data_root).await?;
    let active_id = registry.active_account_id.clone();
    let cached_active = if !refresh {
        state
            .providers
            .provider_usage_cache_entry(CODEX_PROVIDER_ID)
            .await
    } else {
        None
    };
    let (cfg, config_error) =
        ctx_provider_runtime::provider_launch::config::load_managed_agent_server_config_with_error(
            &state.core.data_root,
        )
        .await;
    if let Some(config_error) = config_error {
        anyhow::bail!(config_error);
    }

    let mut entries = Vec::new();
    for account in registry.accounts {
        let usage = match ctx_provider_accounts::codex_usage_env_for_account(
            &state.core.data_root,
            &account.id,
        )
        .await
        {
            Ok(mut env) => {
                ctx_managed_installs::ensure_codex_cli_command_env_for_target(
                    &mut env,
                    &cfg,
                    CODEX_PROVIDER_ID,
                    Some(ctx_provider_install::InstallTarget::Host),
                )?;
                if active_id.as_deref() == Some(&account.id) {
                    if let Some(snapshot) = cached_active.clone() {
                        snapshot
                    } else {
                        provider_usage::fetch_codex_usage_snapshot(env).await?
                    }
                } else {
                    provider_usage::fetch_codex_usage_snapshot(env).await?
                }
            }
            Err(err) => codex_account_usage_error(err.to_string()),
        };
        entries.push(CodexAccountUsageRecord {
            account_id: Some(account.id),
            label: account.label,
            email: account.email,
            plan_type: account.plan_type,
            last_used_at: account.last_used_at,
            usage,
        });
    }

    Ok(entries)
}

#[cfg(test)]
mod route_tests {
    use super::*;

    #[test]
    fn provider_usage_route_query_defaults_to_no_refresh() {
        assert!(!ProviderUsageRouteQuery::default().refresh());
    }

    #[test]
    fn provider_usage_route_error_preserves_current_message() {
        let error = ProviderUsageRouteError::from_error(anyhow::anyhow!(
            "parsing agent server config failed"
        ));

        assert_eq!(error.message(), "parsing agent server config failed");
    }

    #[test]
    fn provider_usage_route_snapshot_preserves_json_shape() {
        let snapshot = ProviderUsageRouteSnapshot::from(provider_usage::ProviderUsageSnapshot {
            provider_id: "codex".to_string(),
            source: "oauth".to_string(),
            fetched_at: DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
            payload: Some(serde_json::json!({ "cached": true })),
            error: None,
        });
        let payload = serde_json::to_value(snapshot).unwrap();

        assert_eq!(payload["provider_id"].as_str(), Some("codex"));
        assert_eq!(payload["source"].as_str(), Some("oauth"));
        assert_eq!(payload["payload"]["cached"].as_bool(), Some(true));
        assert!(payload.get("error").is_none());
    }

    #[test]
    fn codex_account_usage_route_entry_preserves_metadata_and_usage() {
        let record = CodexAccountUsageRecord {
            account_id: Some("acct-1".to_string()),
            label: "Work".to_string(),
            email: Some("dev@example.com".to_string()),
            plan_type: Some("plus".to_string()),
            last_used_at: Some(DateTime::from_timestamp(1_700_000_000, 0).unwrap()),
            usage: provider_usage::ProviderUsageSnapshot {
                provider_id: "codex".to_string(),
                source: "error".to_string(),
                fetched_at: DateTime::from_timestamp(1_700_000_001, 0).unwrap(),
                payload: None,
                error: Some("being deleted".to_string()),
            },
        };

        let payload = serde_json::to_value(CodexAccountUsageRouteEntry::from(record)).unwrap();

        assert_eq!(payload["account_id"].as_str(), Some("acct-1"));
        assert_eq!(payload["label"].as_str(), Some("Work"));
        assert_eq!(payload["email"].as_str(), Some("dev@example.com"));
        assert_eq!(payload["plan_type"].as_str(), Some("plus"));
        assert_eq!(payload["usage"]["provider_id"].as_str(), Some("codex"));
        assert_eq!(payload["usage"]["source"].as_str(), Some("error"));
        assert_eq!(payload["usage"]["error"].as_str(), Some("being deleted"));
        assert!(payload["usage"].get("payload").is_none());
    }
}
