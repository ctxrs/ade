use super::*;

mod accounts_response;
mod load;

use accounts_response::load_bootstrap_accounts;
use load::{load_bootstrap_workspace, load_preferred_model_by_provider};

fn parse_workspace_id(ws_id: &str) -> Result<WorkspaceId, (StatusCode, Json<serde_json::Value>)> {
    Ok(WorkspaceId(uuid::Uuid::parse_str(ws_id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "invalid workspace id",
            })),
        )
    })?))
}

pub(crate) async fn get_workspace_providers_bootstrap(
    State(providers): State<ProvidersHandle>,
    Path(ws_id): Path<String>,
) -> Result<Json<ProvidersBootstrapResponse>, (StatusCode, Json<serde_json::Value>)> {
    let ws_id = parse_workspace_id(&ws_id)?;
    load_bootstrap_workspace(&providers, ws_id).await?;
    let install_target = providers
        .install_target_for_workspace(ws_id)
        .await
        .map_err(|error| status::workspace_execution_settings_error_json(&error))?;
    let preferred_model_by_provider =
        std::sync::Arc::new(load_preferred_model_by_provider(&providers, ws_id).await?);

    let provider_statuses = providers
        .providers_statuses_response(install_target, true)
        .await;
    let visible_providers = provider_statuses
        .iter()
        .filter(|provider| !provider.detail_flag("ui_hidden").unwrap_or(false))
        .cloned()
        .collect::<Vec<_>>();

    let per_provider =
        futures::stream::iter(visible_providers.into_iter().map(|provider_status| {
            let providers = providers.clone();
            let preferred_model_by_provider = std::sync::Arc::clone(&preferred_model_by_provider);
            async move {
                let preferred_model_id = preferred_model_by_provider
                    .get(&provider_status.provider_id)
                    .cloned();
                providers
                    .build_bootstrap_options(ws_id, provider_status, preferred_model_id)
                    .await
            }
        }))
        .buffer_unordered(ctx_daemon::daemon::providers::visible_provider_count_hint(
            provider_statuses.len(),
        ))
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
    let accounts = load_bootstrap_accounts(&providers).await?;

    Ok(Json(ProvidersBootstrapResponse {
        providers: provider_statuses,
        provider_options,
        provider_harness_config,
        codex_accounts: accounts.codex_accounts,
        claude_accounts: accounts.claude_accounts,
        gemini_accounts: accounts.gemini_accounts,
        qwen_accounts: accounts.qwen_accounts,
        kimi_accounts: accounts.kimi_accounts,
        mistral_accounts: accounts.mistral_accounts,
        copilot_accounts: accounts.copilot_accounts,
        cursor_accounts: accounts.cursor_accounts,
        amp_accounts: accounts.amp_accounts,
    }))
}
