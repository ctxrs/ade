use std::sync::Arc;

use anyhow::Context;
use ctx_provider_install::install_state::InstallTarget;
pub use ctx_provider_runtime::provider_status_service::ProviderStatusResponseError;
use ctx_provider_runtime::{
    provider_status_service as status_service, ProviderStatusListRouteError,
    ProviderStatusRouteError, ProviderStatusRouteQuery,
};
use ctx_providers::adapters::ProviderStatus;

use crate::daemon::providers::parse_provider_install_target;
use crate::daemon::{execution_effective, DaemonState, ProvidersHandle};

pub async fn install_target_for_workspace(
    state: &Arc<DaemonState>,
    workspace_id: ctx_core::ids::WorkspaceId,
) -> anyhow::Result<InstallTarget> {
    execution_effective::effective_install_target(state.as_ref(), workspace_id)
        .await
        .with_context(|| {
            format!(
                "loading execution settings for workspace {}",
                workspace_id.0
            )
        })
}

pub async fn refresh_provider_statuses(state: &DaemonState) -> anyhow::Result<()> {
    status_service::refresh_provider_statuses(state).await
}

pub async fn providers_statuses_response(
    state: &Arc<DaemonState>,
    target: InstallTarget,
    include_matrix_providers: bool,
) -> Vec<ProviderStatus> {
    status_service::providers_statuses_response(state.as_ref(), target, include_matrix_providers)
        .await
}

pub async fn provider_status_response(
    state: &Arc<DaemonState>,
    provider_id: &str,
    target: InstallTarget,
) -> Result<ProviderStatus, ProviderStatusResponseError> {
    status_service::provider_status_response(state.as_ref(), provider_id, target).await
}

impl ProvidersHandle {
    pub async fn providers_statuses_for_route(
        &self,
        query: ProviderStatusRouteQuery,
    ) -> Result<Vec<ProviderStatus>, ProviderStatusListRouteError> {
        let target = parse_provider_install_target(query.target())
            .map_err(|_| ProviderStatusListRouteError)?;
        Ok(providers_statuses_response(&self.state, target, false).await)
    }

    pub async fn provider_status_for_route(
        &self,
        provider_id: &str,
        query: ProviderStatusRouteQuery,
    ) -> Result<ProviderStatus, ProviderStatusRouteError> {
        let target = parse_provider_install_target(query.target())
            .map_err(ProviderStatusRouteError::bad_request)?;
        provider_status_response(&self.state, provider_id, target)
            .await
            .map_err(provider_status_route_error)
    }
}

fn provider_status_route_error(error: ProviderStatusResponseError) -> ProviderStatusRouteError {
    match error {
        ProviderStatusResponseError::NotFound { provider_id } => {
            ProviderStatusRouteError::not_found(format!("provider not found: {provider_id}"))
        }
    }
}

#[cfg(test)]
mod route_tests {
    use super::*;
    use ctx_provider_runtime::ProviderStatusRouteErrorKind;

    #[test]
    fn provider_status_route_error_preserves_not_found_body() {
        let error = provider_status_route_error(ProviderStatusResponseError::NotFound {
            provider_id: "missing-provider".to_string(),
        });

        assert_eq!(error.kind(), ProviderStatusRouteErrorKind::NotFound);
        assert_eq!(
            error.body()["error"].as_str(),
            Some("provider not found: missing-provider")
        );
    }
}
