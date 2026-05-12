use crate::daemon::AppState;

pub(super) async fn list_provider_processes(
    state: &AppState,
) -> Vec<ctx_providers::adapters::ProviderProcessInfo> {
    let providers = state.providers.provider_adapter_entries().await;
    let mut processes = Vec::new();
    for (_, adapter) in providers {
        processes.extend(adapter.list_processes().await);
    }
    processes
}
