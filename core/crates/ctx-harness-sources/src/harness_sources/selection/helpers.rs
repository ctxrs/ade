use super::*;

pub(super) fn repair_provider_selection(
    provider: &mut HarnessProviderConfigInternal,
    endpoint_supported: bool,
) -> bool {
    if !endpoint_supported {
        if provider.selected_source_kind != HarnessSourceKind::Subscription
            || provider.selected_endpoint_id.is_some()
        {
            provider.selected_source_kind = HarnessSourceKind::Subscription;
            provider.selected_endpoint_id = None;
            return true;
        }
        return false;
    }

    if provider.selected_source_kind != HarnessSourceKind::Endpoint {
        if provider.selected_endpoint_id.is_some() {
            provider.selected_endpoint_id = None;
            return true;
        }
        return false;
    }

    if provider.selected_source_kind == HarnessSourceKind::Endpoint {
        let exists = provider
            .selected_endpoint_id
            .as_ref()
            .and_then(|id| provider.endpoints.iter().find(|ep| ep.id == *id))
            .is_some();
        if !exists {
            provider.selected_source_kind = HarnessSourceKind::Subscription;
            provider.selected_endpoint_id = None;
            return true;
        }
    }

    false
}

pub(super) async fn provider_source_config_from_internal(
    data_root: &Path,
    canonical: &str,
    endpoint_supported: bool,
    provider: &HarnessProviderConfigInternal,
) -> HarnessProviderSourceConfig {
    let mut endpoints = Vec::new();
    if endpoint_supported {
        endpoints.reserve(provider.endpoints.len());
        for endpoint in &provider.endpoints {
            endpoints.push(public_endpoint_from_internal(data_root, endpoint).await);
        }
    }
    HarnessProviderSourceConfig {
        provider_id: canonical.to_string(),
        selected_source_kind: if endpoint_supported && provider.selected_endpoint_id.is_some() {
            provider.selected_source_kind
        } else {
            HarnessSourceKind::Subscription
        },
        selected_endpoint_id: if endpoint_supported {
            provider.selected_endpoint_id.clone()
        } else {
            None
        },
        endpoints,
    }
}

pub(super) async fn get_provider_source_config_locked(
    data_root: &Path,
    registry: &mut HarnessSourceRegistryInternal,
    canonical: &str,
    endpoint_supported: bool,
) -> Result<HarnessProviderSourceConfig> {
    let config = {
        let provider = registry
            .providers
            .entry(canonical.to_string())
            .or_insert_with(HarnessProviderConfigInternal::default);
        let repaired = repair_provider_selection(provider, endpoint_supported);
        (repaired, provider.clone())
    };
    if config.0 {
        registry::save_registry(data_root, registry).await?;
    }
    Ok(
        provider_source_config_from_internal(data_root, canonical, endpoint_supported, &config.1)
            .await,
    )
}
