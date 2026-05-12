use crate::daemon::AppState;

pub(super) async fn list_provider_processes(
    state: &AppState,
) -> Vec<ctx_providers::adapters::ProviderProcessInfo> {
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
