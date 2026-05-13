use ctx_observability::logs;

use crate::daemon::AppState;

pub(crate) struct ProviderMatrixRefreshSummary {
    pub(crate) provider_count: usize,
    pub(crate) generated_at: Option<String>,
    pub(crate) source: String,
    pub(crate) degraded: bool,
    pub(crate) last_error: Option<String>,
}

pub(crate) async fn refresh_provider_inventory(
    state: &AppState,
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
