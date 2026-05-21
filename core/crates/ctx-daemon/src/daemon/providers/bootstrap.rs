use std::collections::HashMap;
use std::sync::Arc;

use ctx_core::ids::WorkspaceId;
use ctx_harness_sources::HarnessProviderSourceConfig;
use ctx_observability::logs;
use ctx_provider_accounts::{
    AmpAccountsResponse, ClaudeAccountsResponse, CodexAccountsResponse, CopilotAccountsResponse,
    CursorAccountsResponse, GeminiAccountsResponse, KimiAccountsResponse, MistralAccountsResponse,
    QwenAccountsResponse,
};
use ctx_provider_runtime::model_preferences::preferred_model_id_from_available_models;
use ctx_provider_runtime::provider_auth::{
    provider_auth_mode, provider_has_active_auth_config_with_runtime_root,
    selected_endpoint_record_from_harness_config,
};
use ctx_provider_runtime::provider_launch::models::{
    endpoint_models_payload, subscription_models_payload_from_status,
};
use ctx_provider_runtime::provider_usability::{
    provider_status_is_usable, provider_status_unusable_reason,
};
use ctx_provider_runtime::{
    ProvidersBootstrapResponse, ProvidersBootstrapRouteError, ProvidersBootstrapRouteRequest,
};
use ctx_providers::adapters::ProviderStatus;
use futures::StreamExt;

use crate::daemon::{DaemonState, ProvidersHandle};

use super::{accounts, status};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum ProvidersBootstrapErrorKind {
    NotFound,
    Internal,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct ProvidersBootstrapError {
    kind: ProvidersBootstrapErrorKind,
    message: String,
}

impl ProvidersBootstrapError {
    fn not_found(message: impl Into<String>) -> Self {
        Self {
            kind: ProvidersBootstrapErrorKind::NotFound,
            message: message.into(),
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self {
            kind: ProvidersBootstrapErrorKind::Internal,
            message: message.into(),
        }
    }

    fn kind(&self) -> ProvidersBootstrapErrorKind {
        self.kind
    }

    fn message(&self) -> &str {
        &self.message
    }
}

impl ProvidersHandle {
    pub async fn workspace_providers_bootstrap_for_route(
        &self,
        request: ProvidersBootstrapRouteRequest,
    ) -> Result<ProvidersBootstrapResponse, ProvidersBootstrapRouteError> {
        let workspace_id = parse_bootstrap_workspace_id(request.workspace_id())?;
        workspace_providers_bootstrap(&self.state, workspace_id)
            .await
            .map_err(bootstrap_route_error)
    }
}

fn parse_bootstrap_workspace_id(
    workspace_id: &str,
) -> Result<WorkspaceId, ProvidersBootstrapRouteError> {
    uuid::Uuid::parse_str(workspace_id)
        .map(WorkspaceId)
        .map_err(|_| ProvidersBootstrapRouteError::bad_request("invalid workspace id"))
}

fn bootstrap_route_error(error: ProvidersBootstrapError) -> ProvidersBootstrapRouteError {
    match error.kind() {
        ProvidersBootstrapErrorKind::NotFound => {
            ProvidersBootstrapRouteError::not_found(error.message())
        }
        ProvidersBootstrapErrorKind::Internal => {
            ProvidersBootstrapRouteError::internal(error.message())
        }
    }
}

async fn workspace_providers_bootstrap(
    state: &Arc<DaemonState>,
    ws_id: WorkspaceId,
) -> Result<ProvidersBootstrapResponse, ProvidersBootstrapError> {
    load_bootstrap_workspace(state, ws_id).await?;
    let install_target = status::install_target_for_workspace(state, ws_id)
        .await
        .map_err(|error| {
            ProvidersBootstrapError::internal(format!(
                "failed to load workspace execution settings: {error:#}"
            ))
        })?;
    let preferred_model_by_provider =
        Arc::new(load_preferred_model_by_provider(state, ws_id).await?);

    let provider_statuses = status::providers_statuses_response(state, install_target, true).await;
    let visible_providers = provider_statuses
        .iter()
        .filter(|provider| !provider.detail_flag("ui_hidden").unwrap_or(false))
        .cloned()
        .collect::<Vec<_>>();

    let per_provider =
        futures::stream::iter(visible_providers.into_iter().map(|provider_status| {
            let state = Arc::clone(state);
            let preferred_model_by_provider = Arc::clone(&preferred_model_by_provider);
            async move {
                let preferred_model_id = preferred_model_by_provider
                    .get(&provider_status.provider_id)
                    .cloned();
                build_bootstrap_options(&state, ws_id, provider_status, preferred_model_id).await
            }
        }))
        .buffer_unordered(visible_provider_count_hint(provider_statuses.len()))
        .collect::<Vec<_>>()
        .await;

    let mut provider_options = HashMap::new();
    let mut provider_harness_config = HashMap::new();
    for (provider_id, options, source_config) in per_provider {
        provider_options.insert(provider_id.clone(), options);
        if let Some(config) = source_config {
            provider_harness_config.insert(provider_id, config);
        }
    }

    let accounts = load_bootstrap_accounts(state).await?;

    Ok(ProvidersBootstrapResponse::new(
        provider_statuses,
        provider_options,
        provider_harness_config,
        accounts.codex_accounts,
        accounts.claude_accounts,
        accounts.gemini_accounts,
        accounts.qwen_accounts,
        accounts.kimi_accounts,
        accounts.mistral_accounts,
        accounts.copilot_accounts,
        accounts.cursor_accounts,
        accounts.amp_accounts,
    ))
}

#[cfg(test)]
mod route_tests {
    use ctx_provider_runtime::ProvidersBootstrapRouteErrorKind;

    use super::*;

    #[test]
    fn bootstrap_route_parse_failure_preserves_invalid_workspace_body() {
        let error = parse_bootstrap_workspace_id("not-a-uuid").unwrap_err();

        assert_eq!(error.kind(), ProvidersBootstrapRouteErrorKind::BadRequest);
        assert_eq!(error.body()["error"].as_str(), Some("invalid workspace id"));
    }

    #[test]
    fn bootstrap_route_error_preserves_not_found_body() {
        let error =
            bootstrap_route_error(ProvidersBootstrapError::not_found("workspace not found"));

        assert_eq!(error.kind(), ProvidersBootstrapRouteErrorKind::NotFound);
        assert_eq!(error.body()["error"].as_str(), Some("workspace not found"));
    }

    #[test]
    fn bootstrap_route_error_preserves_internal_body() {
        let error = bootstrap_route_error(ProvidersBootstrapError::internal(
            "failed to load workspace execution settings: boom",
        ));

        assert_eq!(error.kind(), ProvidersBootstrapRouteErrorKind::Internal);
        assert_eq!(
            error.body()["error"].as_str(),
            Some("failed to load workspace execution settings: boom")
        );
    }
}

async fn load_bootstrap_workspace(
    state: &Arc<DaemonState>,
    ws_id: WorkspaceId,
) -> Result<(), ProvidersBootstrapError> {
    let exists = state
        .global_store()
        .get_workspace(ws_id)
        .await
        .map_err(|_| ProvidersBootstrapError::internal("failed to load workspace"))?
        .is_some();
    if exists {
        return Ok(());
    }

    Err(ProvidersBootstrapError::not_found("workspace not found"))
}

async fn load_preferred_model_by_provider(
    state: &Arc<DaemonState>,
    ws_id: WorkspaceId,
) -> Result<HashMap<String, String>, ProvidersBootstrapError> {
    let store = state.store_for_workspace(ws_id).await.map_err(|error| {
        ProvidersBootstrapError::internal(format!(
            "failed to load workspace provider model preferences: {}",
            logs::redact_sensitive(&error.to_string())
        ))
    })?;
    ctx_workspace_config::load_preferred_new_session_models(&store)
        .await
        .map_err(|error| {
            ProvidersBootstrapError::internal(format!(
                "failed to load workspace provider model preferences: {}",
                logs::redact_sensitive(&error.to_string())
            ))
        })
}

struct BootstrapAccounts {
    codex_accounts: CodexAccountsResponse,
    claude_accounts: ClaudeAccountsResponse,
    gemini_accounts: GeminiAccountsResponse,
    qwen_accounts: QwenAccountsResponse,
    kimi_accounts: KimiAccountsResponse,
    mistral_accounts: MistralAccountsResponse,
    copilot_accounts: CopilotAccountsResponse,
    cursor_accounts: CursorAccountsResponse,
    amp_accounts: AmpAccountsResponse,
}

fn bootstrap_accounts_error(provider_id: &str, err: anyhow::Error) -> ProvidersBootstrapError {
    ProvidersBootstrapError::internal(format!(
        "failed to load {provider_id} accounts: {}",
        logs::redact_sensitive(&err.to_string())
    ))
}

async fn load_bootstrap_accounts(
    state: &Arc<DaemonState>,
) -> Result<BootstrapAccounts, ProvidersBootstrapError> {
    let codex_snapshot = accounts::load_codex_accounts_snapshot(state)
        .await
        .map_err(|err| bootstrap_accounts_error("codex", err))?;
    let claude_registry = accounts::load_claude_account_registry(state)
        .await
        .map_err(|err| bootstrap_accounts_error("claude-crp", err))?;
    let gemini_registry = accounts::load_gemini_account_registry(state)
        .await
        .map_err(|err| bootstrap_accounts_error("gemini", err))?;
    let qwen_registry = accounts::load_qwen_account_registry(state)
        .await
        .map_err(|err| bootstrap_accounts_error("qwen", err))?;
    let kimi_registry = accounts::load_kimi_account_registry(state)
        .await
        .map_err(|err| bootstrap_accounts_error("kimi", err))?;
    let mistral_registry = accounts::load_mistral_account_registry(state)
        .await
        .map_err(|err| bootstrap_accounts_error("mistral", err))?;
    let copilot_registry = accounts::load_copilot_account_registry(state)
        .await
        .map_err(|err| bootstrap_accounts_error("copilot", err))?;
    let cursor_registry = accounts::load_cursor_account_registry(state)
        .await
        .map_err(|err| bootstrap_accounts_error("cursor", err))?;
    let amp_registry = accounts::ensure_amp_account_registry_from_runtime_auth(state)
        .await
        .map_err(|err| bootstrap_accounts_error("amp", err))?;

    Ok(BootstrapAccounts {
        codex_accounts: CodexAccountsResponse::new(
            codex_snapshot.active_account_id,
            codex_snapshot.accounts,
            codex_snapshot.logins,
        ),
        claude_accounts: ClaudeAccountsResponse::from(claude_registry),
        gemini_accounts: GeminiAccountsResponse::from(gemini_registry),
        qwen_accounts: QwenAccountsResponse::from(qwen_registry),
        kimi_accounts: KimiAccountsResponse::from(kimi_registry),
        mistral_accounts: MistralAccountsResponse::from(mistral_registry),
        copilot_accounts: CopilotAccountsResponse::from(copilot_registry),
        cursor_accounts: CursorAccountsResponse::from(cursor_registry),
        amp_accounts: AmpAccountsResponse::from(amp_registry),
    })
}

async fn build_bootstrap_options(
    state: &Arc<DaemonState>,
    ws_id: WorkspaceId,
    provider_status: ProviderStatus,
    preferred_model_id: Option<String>,
) -> (
    String,
    serde_json::Value,
    Option<HarnessProviderSourceConfig>,
) {
    let provider_id = provider_status.provider_id.clone();
    let (source_config, source_config_error) =
        ctx_provider_runtime::provider_launch::config::load_provider_source_config_with_error(
            &state.core.data_root,
            &provider_id,
        )
        .await;
    let (has_active_auth, auth_mode, auth_config_error) = provider_auth_summary(
        state,
        &provider_id,
        source_config.as_ref(),
        &source_config_error,
    )
    .await;
    let (mut probe_ok, mut auth_required, mut probe_error) =
        bootstrap_provider_probe_summary(&provider_status, has_active_auth);
    if let Some(config_error) = auth_config_error.as_ref() {
        probe_ok = false;
        auth_required = false;
        probe_error = Some(config_error.clone());
    }

    let mut options = serde_json::json!({
        "provider_id": provider_id,
        "workspace_id": ws_id.0.to_string(),
        "supports_load": false,
        "auth_required": auth_required,
        "has_active_auth": has_active_auth,
        "auth_mode": auth_mode,
        "probe_ok": probe_ok,
        "probed_at": chrono::Utc::now().to_rfc3339(),
    });
    if let Some(probe_error) = probe_error {
        options["probe_error"] = serde_json::json!(probe_error);
    }
    if let Some(config_error) = source_config_error.as_ref().or(auth_config_error.as_ref()) {
        options["probe_ok"] = serde_json::json!(false);
        options["probe_error"] = serde_json::json!(config_error);
        options["config_error"] = serde_json::json!(config_error);
    }
    append_model_options(
        &provider_id,
        &provider_status,
        source_config.as_ref(),
        &mut options,
    );
    if let Some(preferred_model_id) =
        preferred_model_id_from_available_models(preferred_model_id, options.get("models"))
    {
        options["preferred_model_id"] = serde_json::json!(preferred_model_id);
    }

    if let Some(source) = source_config.as_ref() {
        options["source"] = serde_json::to_value(source).unwrap_or(serde_json::Value::Null);
    }

    (provider_id, options, source_config)
}

fn visible_provider_count_hint(total_provider_count: usize) -> usize {
    total_provider_count.max(1)
}

async fn provider_auth_summary(
    state: &Arc<DaemonState>,
    provider_id: &str,
    source_config: Option<&HarnessProviderSourceConfig>,
    source_config_error: &Option<String>,
) -> (bool, &'static str, Option<String>) {
    // Bootstrap is auth/config hydration only. It must stay substrate-agnostic and
    // never cross into workspace runtime preparation.
    if source_config_error.is_some() {
        return (false, "none", None);
    }

    match provider_has_active_auth_config_with_runtime_root(
        &state.core.data_root,
        None,
        provider_id,
        source_config,
    )
    .await
    {
        Ok(has_active_auth) => (
            has_active_auth,
            provider_auth_mode(has_active_auth, source_config),
            None,
        ),
        Err(err) => (false, "none", Some(logs::redact_sensitive(&err))),
    }
}

fn bootstrap_provider_probe_summary(
    provider_status: &ProviderStatus,
    has_active_auth: bool,
) -> (bool, bool, Option<String>) {
    if !provider_status_is_usable(provider_status) {
        return (
            false,
            false,
            Some(
                provider_status_unusable_reason(provider_status)
                    .unwrap_or_else(|| "provider not ready for use".to_string()),
            ),
        );
    }

    (true, !has_active_auth, None)
}

fn append_model_options(
    provider_id: &str,
    provider_status: &ProviderStatus,
    source_config: Option<&HarnessProviderSourceConfig>,
    options: &mut serde_json::Value,
) {
    if let Some(endpoint) = selected_endpoint_record_from_harness_config(source_config) {
        options["models"] = endpoint_models_payload(provider_id, &endpoint, chrono::Utc::now());
    } else if let Some(models) = subscription_models_payload_from_status(provider_status) {
        options["models"] = models;
    }
}
