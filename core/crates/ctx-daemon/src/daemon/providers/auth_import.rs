use std::collections::HashSet;
use std::sync::Arc;

use ctx_observability::logs;
use ctx_provider_auth_import as provider_auth_import;
use ctx_provider_auth_import::{
    ProviderAuthImportCandidatesRouteResponse, ProviderAuthImportProfilesRouteResponse,
    ProviderAuthImportRouteError, ProviderAuthImportRouteRequest, ProviderAuthImportRouteResponse,
};

use crate::daemon::{DaemonState, ProvidersHandle};

fn redacted_route_error(error: anyhow::Error) -> ProviderAuthImportRouteError {
    ProviderAuthImportRouteError::new(logs::redact_sensitive(&error.to_string()))
}

fn raw_route_error(error: anyhow::Error) -> ProviderAuthImportRouteError {
    ProviderAuthImportRouteError::new(error.to_string())
}

pub async fn list_provider_auth_import_candidates(
) -> anyhow::Result<Vec<provider_auth_import::ProviderAuthImportCandidate>> {
    provider_auth_import::list_provider_auth_import_candidates().await
}

pub async fn list_provider_auth_import_profiles(
    state: &Arc<DaemonState>,
) -> anyhow::Result<Vec<provider_auth_import::ProviderImportedAuthProfile>> {
    provider_auth_import::list_provider_auth_profiles(&state.core.data_root).await
}

pub async fn import_provider_auth_candidates(
    state: &Arc<DaemonState>,
    candidate_ids: Vec<String>,
) -> anyhow::Result<Vec<provider_auth_import::ProviderAuthImportResult>> {
    let ids: Vec<String> = candidate_ids
        .into_iter()
        .filter_map(|id| {
            let trimmed = id.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        })
        .collect();
    let results =
        provider_auth_import::import_provider_auth_candidates(&state.core.data_root, &ids).await?;

    let mutated_providers: HashSet<String> = results
        .iter()
        .filter(|result| provider_auth_import_result_requires_restart(result))
        .map(|result| result.provider_id.clone())
        .collect();
    let mut restart_errors = Vec::new();
    for provider_id in mutated_providers {
        if let Err(error) = super::restart_provider_for_auth_change(
            state,
            &provider_id,
            &format!("{provider_id} auth updated"),
        )
        .await
        {
            restart_errors.push(error.to_string());
        }
    }
    if !restart_errors.is_empty() {
        anyhow::bail!(
            "provider auth updated but one or more runtime restarts failed: {}",
            restart_errors.join("; ")
        );
    }

    Ok(results)
}

pub fn provider_auth_import_result_requires_restart(
    result: &provider_auth_import::ProviderAuthImportResult,
) -> bool {
    // `already_imported` can still mutate active account selection (dedupe/upsert paths),
    // so treat it as auth-affecting to avoid stale runtime credentials.
    matches!(
        result.status.as_str(),
        "imported" | "updated" | "already_imported"
    )
}

impl ProvidersHandle {
    pub async fn list_provider_auth_import_candidates_for_route(
        &self,
    ) -> Result<ProviderAuthImportCandidatesRouteResponse, ProviderAuthImportRouteError> {
        let candidates = list_provider_auth_import_candidates()
            .await
            .map_err(redacted_route_error)?;
        Ok(ProviderAuthImportCandidatesRouteResponse::new(candidates))
    }

    pub async fn list_provider_auth_import_profiles_for_route(
        &self,
    ) -> Result<ProviderAuthImportProfilesRouteResponse, ProviderAuthImportRouteError> {
        let profiles = list_provider_auth_import_profiles(&self.state)
            .await
            .map_err(raw_route_error)?;
        Ok(ProviderAuthImportProfilesRouteResponse::new(profiles))
    }

    pub async fn import_provider_auth_candidates_for_route(
        &self,
        request: ProviderAuthImportRouteRequest,
    ) -> Result<ProviderAuthImportRouteResponse, ProviderAuthImportRouteError> {
        let results = import_provider_auth_candidates(&self.state, request.into_candidate_ids())
            .await
            .map_err(raw_route_error)?;
        Ok(ProviderAuthImportRouteResponse::new(results))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_route_errors_are_redacted_before_crossing_route_boundary() {
        let error = redacted_route_error(anyhow::anyhow!(
            r#"candidate scan failed with {{"ctx_mcp_token":"secret-token"}}"#
        ));

        assert!(error.message().contains("[REDACTED]"));
        assert!(!error.message().contains("secret-token"));
    }

    #[test]
    fn profile_and_import_route_errors_keep_actionable_context() {
        let error = raw_route_error(anyhow::anyhow!(
            "parsing imported auth registry at /tmp/profiles.json"
        ));

        assert!(error.message().contains("parsing imported auth registry"));
        assert!(error.message().contains("profiles.json"));
    }
}
