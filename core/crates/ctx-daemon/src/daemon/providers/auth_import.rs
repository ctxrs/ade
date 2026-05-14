use std::collections::HashSet;
use std::sync::Arc;

use ctx_provider_auth_import as provider_auth_import;

use crate::daemon::DaemonState;

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
