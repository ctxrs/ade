use std::sync::Arc;

use ctx_core::models::Workspace;

use crate::daemon::providers::auth_check::outcome::ProviderVerifyOutcome;
use crate::daemon::providers::auth_check::ProviderAuthCheckError;
use crate::daemon::providers::probe_provider_auth_verification_runtime;
use crate::daemon::AppState;

pub(super) async fn apply_auth_verification_probe(
    state: &Arc<AppState>,
    workspace: &Workspace,
    provider_id: &str,
    outcome: &mut ProviderVerifyOutcome,
) -> Result<(), ProviderAuthCheckError> {
    let probe = probe_provider_auth_verification_runtime(
        state,
        workspace,
        provider_id,
        outcome.selected_endpoint_id().map(str::to_string),
    )
    .await
    .map_err(ProviderAuthCheckError::ExecutionSettings)?;
    outcome.set_selected_endpoint_id(probe.selected_endpoint_id);
    if let Some(probe_error) = probe.probe_error {
        if outcome.has_endpoint_catalog_result() {
            outcome.apply_endpoint_catalog_runtime_probe_failure(probe_error);
        } else {
            outcome.apply_classified_probe_error(probe_error);
        }
    }
    Ok(())
}
