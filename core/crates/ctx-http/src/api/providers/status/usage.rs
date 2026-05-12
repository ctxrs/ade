use std::collections::HashMap;

use super::*;
use ctx_core::provider_ids::CODEX_PROVIDER_ID;
use ctx_provider_runtime::provider_usage;

fn provider_usage_internal_error(
    error: impl std::fmt::Display,
) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({
            "error": error.to_string()
        })),
    )
}

async fn provider_usage_env_for_request(
    state: &Arc<AppState>,
    provider_id: &str,
) -> Result<HashMap<String, String>, (StatusCode, Json<serde_json::Value>)> {
    if provider_id != CODEX_PROVIDER_ID {
        return Ok(HashMap::new());
    }

    let mut env = provider_accounts::codex_env_for_active_account(&state.core.data_root)
        .await
        .map_err(provider_usage_internal_error)?;
    let (cfg, config_error) =
        ctx_provider_runtime::provider_launch::config::load_managed_agent_server_config_with_error(
            &state.core.data_root,
        )
        .await;
    if let Some(config_error) = config_error {
        return Err(provider_usage_internal_error(config_error));
    }
    ctx_managed_installs::ensure_codex_cli_command_env_for_target(
        &mut env,
        &cfg,
        CODEX_PROVIDER_ID,
        Some(InstallTarget::Host),
    )
    .map_err(provider_usage_internal_error)?;
    Ok(env)
}

pub(crate) async fn get_provider_usage(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<ProviderUsageQuery>,
) -> Result<Json<provider_usage::ProviderUsageSnapshot>, (StatusCode, Json<serde_json::Value>)> {
    let refresh = query.refresh.unwrap_or(false);
    let env = provider_usage_env_for_request(&state, &id).await?;
    let snapshot = if !refresh {
        state.providers.provider_usage_cache_entry(&id).await
    } else {
        None
    };
    let snapshot = match snapshot {
        Some(snapshot) => snapshot,
        None => provider_usage::refresh_provider_usage_for(state.as_ref(), &id, env)
            .await
            .map_err(provider_usage_internal_error)?,
    };
    Ok(Json(snapshot))
}
