use ctx_observability::logs;

use crate::daemon::DaemonState;

pub struct ProviderMatrixRefreshSummary {
    pub provider_count: usize,
    pub generated_at: Option<String>,
    pub source: String,
    pub degraded: bool,
    pub last_error: Option<String>,
}

pub async fn refresh_provider_inventory(
    state: &DaemonState,
) -> anyhow::Result<ProviderMatrixRefreshSummary> {
    let outcome = state
        .providers
        .refresh_provider_matrix_from_local_sources(&state.core.data_root)
        .await;
    ctx_managed_installs::refresh_provider_statuses(state).await?;

    Ok(ProviderMatrixRefreshSummary {
        provider_count: outcome.matrix.providers.len(),
        generated_at: outcome.matrix.generated_at,
        source: outcome.source.as_str().to_string(),
        degraded: outcome.degraded,
        last_error: outcome
            .last_error
            .map(|value| logs::redact_sensitive(&value)),
    })
}
