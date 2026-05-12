use std::collections::HashMap;
use std::sync::Arc;

use ctx_providers::adapters::ProviderProcessInfo;

use crate::daemon::AppState;

pub(super) async fn list_provider_processes(state: &Arc<AppState>) -> Vec<ProviderProcessInfo> {
    let providers = {
        state
            .providers
            .with_provider_adapters(|providers| providers.values().cloned().collect::<Vec<_>>())
            .await
    };
    let mut processes = Vec::new();
    for adapter in providers {
        processes.extend(adapter.list_processes().await);
    }
    processes
}

pub(super) async fn provider_session_counts(state: &Arc<AppState>) -> HashMap<String, u64> {
    let session_ids = state.sessions.list_running_sessions().await;
    let mut counts: HashMap<String, u64> = HashMap::new();
    for session_id in session_ids {
        let session = match state.store_for_session(session_id).await {
            Ok(store) => store.get_session(session_id).await.ok().flatten(),
            Err(_) => None,
        };
        let Some(session) = session else {
            continue;
        };
        *counts.entry(session.provider_id).or_insert(0) += 1;
    }
    counts
}
