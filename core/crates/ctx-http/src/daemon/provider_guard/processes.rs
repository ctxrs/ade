use crate::daemon::AppState;

pub(super) async fn list_provider_processes(
    state: &AppState,
) -> Vec<ctx_providers::adapters::ProviderProcessInfo> {
    let providers = {
        let providers = state.providers.adapters.lock().await;
        providers.values().cloned().collect::<Vec<_>>()
    };
    let mut processes = Vec::new();
    for adapter in providers {
        processes.extend(adapter.list_processes().await);
    }
    processes
}
