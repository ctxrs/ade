use super::*;

fn repair_provider_selection(
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

fn provider_source_config_from_internal(
    canonical: &str,
    endpoint_supported: bool,
    provider: &HarnessProviderConfigInternal,
) -> HarnessProviderSourceConfig {
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
        endpoints: if endpoint_supported {
            provider
                .endpoints
                .iter()
                .map(public_endpoint_from_internal)
                .collect()
        } else {
            Vec::new()
        },
    }
}

async fn get_provider_source_config_locked(
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
        let config = provider_source_config_from_internal(canonical, endpoint_supported, provider);
        (repaired, config)
    };
    if config.0 {
        registry::save_registry(data_root, registry).await?;
    }
    Ok(config.1)
}

pub(crate) async fn load_repaired_provider_internal(
    data_root: &Path,
    canonical: &str,
    endpoint_supported: bool,
) -> Result<HarnessProviderConfigInternal> {
    let _registry_write_guard = REGISTRY_WRITE_LOCK.lock().await;
    let mut registry = registry::load_registry(data_root).await?;
    let provider = {
        let provider = registry
            .providers
            .entry(canonical.to_string())
            .or_insert_with(HarnessProviderConfigInternal::default);
        let repaired = repair_provider_selection(provider, endpoint_supported);
        (repaired, provider.clone())
    };
    if provider.0 {
        registry::save_registry(data_root, &registry).await?;
    }
    Ok(provider.1)
}

pub async fn get_provider_source_config(
    data_root: &Path,
    provider_id: &str,
) -> Result<HarnessProviderSourceConfig> {
    let canonical = validation::normalize_provider_id(provider_id).ok_or_else(|| {
        anyhow::anyhow!("provider does not support harness endpoints: {provider_id}")
    })?;
    let endpoint_supported = validation::provider_supports_harness_endpoint(canonical);
    let _registry_write_guard = REGISTRY_WRITE_LOCK.lock().await;
    let mut registry = registry::load_registry(data_root).await?;
    get_provider_source_config_locked(data_root, &mut registry, canonical, endpoint_supported).await
}

pub async fn find_provider_endpoint_import_match(
    data_root: &Path,
    provider_id: &str,
    base_url: Option<String>,
    api_shape: HarnessApiShape,
    auth_type: Option<String>,
    model_override: Option<String>,
    api_key: &str,
) -> Result<Option<HarnessEndpointImportMatch>> {
    let canonical = validation::normalize_provider_id(provider_id).ok_or_else(|| {
        anyhow::anyhow!("provider does not support harness endpoints: {provider_id}")
    })?;
    if !validation::provider_supports_harness_endpoint(canonical) {
        anyhow::bail!("provider does not support harness endpoints: {provider_id}");
    }
    validation::ensure_shape_compatible(canonical, api_shape)?;
    let normalized_base_url =
        validation::normalize_base_url_for_provider(canonical, base_url.as_deref())?;
    let normalized_auth_type =
        validation::normalize_auth_type_for_provider(canonical, auth_type.as_deref())?;
    let normalized_model_override = model_override
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    let registry = registry::load_registry(data_root).await?;
    let Some(provider) = registry.providers.get(canonical) else {
        return Ok(None);
    };

    let mut config_match_endpoint_id: Option<String> = None;
    for endpoint in &provider.endpoints {
        if endpoint.base_url != normalized_base_url
            || endpoint.api_shape != api_shape
            || endpoint.auth_type != normalized_auth_type
        {
            continue;
        }
        let endpoint_model_override = endpoint
            .model_override
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        if endpoint_model_override != normalized_model_override {
            continue;
        }
        if config_match_endpoint_id.is_none() {
            config_match_endpoint_id = Some(endpoint.id.clone());
        }
        if let Ok(existing_secret) =
            secrets::read_endpoint_secret(data_root, &endpoint.secret_ref).await
        {
            if existing_secret.api_key.as_deref() == Some(api_key) {
                return Ok(Some(HarnessEndpointImportMatch {
                    endpoint_id: endpoint.id.clone(),
                    kind: HarnessEndpointImportMatchKind::ExactCredentials,
                }));
            }
        }
    }

    Ok(
        config_match_endpoint_id.map(|endpoint_id| HarnessEndpointImportMatch {
            endpoint_id,
            kind: HarnessEndpointImportMatchKind::SameConfig,
        }),
    )
}

pub async fn upsert_provider_endpoint(
    data_root: &Path,
    provider_id: &str,
    input: HarnessEndpointUpsert,
) -> Result<HarnessEndpointRecord> {
    let canonical = validation::normalize_provider_id(provider_id).ok_or_else(|| {
        anyhow::anyhow!("provider does not support harness endpoints: {provider_id}")
    })?;
    if !validation::provider_supports_harness_endpoint(canonical) {
        anyhow::bail!("provider does not support harness endpoints: {provider_id}");
    }
    let api_shape = input
        .api_shape
        .or_else(|| validation::default_shape_for_provider(canonical))
        .ok_or_else(|| anyhow::anyhow!("api_shape is required"))?;
    validation::ensure_shape_compatible(canonical, api_shape)?;

    let name = validation::normalize_name(&input.name)?;
    let base_url =
        validation::normalize_base_url_for_provider(canonical, input.base_url.as_deref())?;
    let model_override = input
        .model_override
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let _registry_write_guard = REGISTRY_WRITE_LOCK.lock().await;
    let mut registry = registry::load_registry(data_root).await?;
    let provider = registry
        .providers
        .entry(canonical.to_string())
        .or_insert_with(HarnessProviderConfigInternal::default);

    let now = Utc::now();
    let endpoint_id = match input
        .endpoint_id
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        Some(value) => validation::normalize_endpoint_id(&value)?,
        None => uuid::Uuid::new_v4().to_string(),
    };
    validation::ensure_safe_endpoint_id(&endpoint_id)?;

    let auth_type =
        validation::normalize_auth_type_for_provider(canonical, input.auth_type.as_deref())?;

    let existing_index = provider
        .endpoints
        .iter()
        .position(|ep| ep.id == endpoint_id);
    let secret_ref = existing_index
        .and_then(|idx| provider.endpoints.get(idx).map(|ep| ep.secret_ref.clone()))
        .unwrap_or_else(|| format!("{canonical}-{endpoint_id}.json"));
    let existing_secret = match existing_index {
        Some(index) => {
            secrets::read_endpoint_secret(data_root, &provider.endpoints[index].secret_ref)
                .await
                .ok()
        }
        None => None,
    };
    let next_secret = secrets::resolve_endpoint_secret_material(
        canonical,
        &auth_type,
        existing_secret.as_ref(),
        &input,
    )?;
    secrets::write_endpoint_secret(data_root, &secret_ref, &next_secret).await?;

    let mut next = HarnessEndpointRecordInternal {
        id: endpoint_id.clone(),
        provider_id: canonical.to_string(),
        name,
        base_url,
        api_shape,
        auth_type,
        model_override,
        created_at: now,
        updated_at: now,
        last_verification_status: HarnessEndpointVerificationStatus::Unknown,
        last_verification_at: None,
        last_error: None,
        model_catalog_status: EndpointModelCatalogStatus::Unknown,
        model_catalog_fetched_at: None,
        model_catalog_error: None,
        model_catalog_models: Vec::new(),
        manual_model_ids: Vec::new(),
        model_catalog_source: None,
        secret_ref,
    };

    if let Some(idx) = existing_index {
        if let Some(previous) = provider.endpoints.get(idx) {
            next.created_at = previous.created_at;
            next.model_catalog_status = previous.model_catalog_status;
            next.model_catalog_fetched_at = previous.model_catalog_fetched_at;
            next.model_catalog_error = previous.model_catalog_error.clone();
            next.model_catalog_models = previous.model_catalog_models.clone();
            next.manual_model_ids = previous.manual_model_ids.clone();
            next.model_catalog_source = previous.model_catalog_source.clone();
        }
        provider.endpoints[idx] = next.clone();
    } else {
        provider.endpoints.push(next.clone());
    }

    registry::save_registry(data_root, &registry).await?;
    Ok(public_endpoint_from_internal(&next))
}

pub async fn delete_provider_endpoint(
    data_root: &Path,
    provider_id: &str,
    endpoint_id: &str,
) -> Result<HarnessProviderSourceConfig> {
    let canonical = validation::normalize_provider_id(provider_id).ok_or_else(|| {
        anyhow::anyhow!("provider does not support harness endpoints: {provider_id}")
    })?;
    if !validation::provider_supports_harness_endpoint(canonical) {
        anyhow::bail!("provider does not support harness endpoints: {provider_id}");
    }
    let _registry_write_guard = REGISTRY_WRITE_LOCK.lock().await;
    let mut registry = registry::load_registry(data_root).await?;
    let provider = registry
        .providers
        .entry(canonical.to_string())
        .or_insert_with(HarnessProviderConfigInternal::default);

    let before = provider.endpoints.len();
    let removed: Vec<(String, String)> = provider
        .endpoints
        .iter()
        .filter(|ep| ep.id == endpoint_id)
        .map(|ep| (ep.id.clone(), ep.secret_ref.clone()))
        .collect();
    provider.endpoints.retain(|ep| ep.id != endpoint_id);
    if removed.is_empty() {
        anyhow::bail!("unknown endpoint");
    }

    if provider.selected_endpoint_id.as_deref() == Some(endpoint_id) {
        provider.selected_source_kind = HarnessSourceKind::Subscription;
        provider.selected_endpoint_id = None;
    }

    if before != provider.endpoints.len() {
        let runtime = runtime_resolution::ProviderRuntimeContext::new(canonical, data_root, None);
        for (removed_endpoint_id, secret_ref) in removed {
            let secret_path = secrets::endpoint_secret_path(data_root, &secret_ref);
            match tokio::fs::remove_file(&secret_path).await {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => {
                    return Err(err).with_context(|| {
                        format!("removing endpoint secret {}", secret_path.display())
                    });
                }
            }
            runtime
                .cleanup_endpoint_runtime(&removed_endpoint_id)
                .await?;
        }
        registry::save_registry(data_root, &registry).await?;
    }

    let endpoint_supported = validation::provider_supports_harness_endpoint(canonical);
    get_provider_source_config_locked(data_root, &mut registry, canonical, endpoint_supported).await
}

pub async fn set_provider_source_selection(
    data_root: &Path,
    provider_id: &str,
    source_kind: HarnessSourceKind,
    endpoint_id: Option<String>,
) -> Result<HarnessProviderSourceConfig> {
    let canonical = validation::normalize_provider_id(provider_id).ok_or_else(|| {
        anyhow::anyhow!("provider does not support harness endpoints: {provider_id}")
    })?;

    let _registry_write_guard = REGISTRY_WRITE_LOCK.lock().await;
    let mut registry = registry::load_registry(data_root).await?;
    let provider = registry
        .providers
        .entry(canonical.to_string())
        .or_insert_with(HarnessProviderConfigInternal::default);

    match source_kind {
        HarnessSourceKind::Subscription => {
            provider.selected_source_kind = HarnessSourceKind::Subscription;
            provider.selected_endpoint_id = None;
        }
        HarnessSourceKind::Endpoint => {
            if !validation::provider_supports_harness_endpoint(canonical) {
                anyhow::bail!("provider does not support harness endpoints: {provider_id}");
            }
            let endpoint_id = endpoint_id
                .as_ref()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow::anyhow!("endpoint_id is required for endpoint source"))?;
            let exists = provider.endpoints.iter().any(|ep| ep.id == endpoint_id);
            if !exists {
                anyhow::bail!("unknown endpoint_id: {endpoint_id}");
            }
            provider.selected_source_kind = HarnessSourceKind::Endpoint;
            provider.selected_endpoint_id = Some(endpoint_id);
        }
    }

    registry::save_registry(data_root, &registry).await?;
    let endpoint_supported = validation::provider_supports_harness_endpoint(canonical);
    get_provider_source_config_locked(data_root, &mut registry, canonical, endpoint_supported).await
}

pub async fn mark_endpoint_verification(
    data_root: &Path,
    provider_id: &str,
    endpoint_id: &str,
    status: HarnessEndpointVerificationStatus,
    error: Option<String>,
) -> Result<()> {
    let canonical = validation::normalize_provider_id(provider_id).ok_or_else(|| {
        anyhow::anyhow!("provider does not support harness endpoints: {provider_id}")
    })?;
    let _registry_write_guard = REGISTRY_WRITE_LOCK.lock().await;
    let mut registry = registry::load_registry(data_root).await?;
    let Some(provider) = registry.providers.get_mut(canonical) else {
        return Ok(());
    };
    let Some(endpoint) = provider
        .endpoints
        .iter_mut()
        .find(|ep| ep.id == endpoint_id)
    else {
        return Ok(());
    };
    endpoint.last_verification_status = status;
    endpoint.last_verification_at = Some(Utc::now());
    endpoint.last_error = error;
    endpoint.updated_at = Utc::now();
    registry::save_registry(data_root, &registry).await
}

pub async fn set_provider_endpoint_manual_models(
    data_root: &Path,
    provider_id: &str,
    endpoint_id: &str,
    manual_model_ids: Vec<String>,
) -> Result<HarnessEndpointRecord> {
    let canonical = validation::normalize_provider_id(provider_id).ok_or_else(|| {
        anyhow::anyhow!("provider does not support harness endpoints: {provider_id}")
    })?;
    let normalized_manual = validation::normalize_manual_model_ids(&manual_model_ids);
    let _registry_write_guard = REGISTRY_WRITE_LOCK.lock().await;
    let mut registry = registry::load_registry(data_root).await?;
    let provider = registry
        .providers
        .get_mut(canonical)
        .ok_or_else(|| anyhow::anyhow!("unknown provider endpoint config for {canonical}"))?;
    let endpoint = provider
        .endpoints
        .iter_mut()
        .find(|ep| ep.id == endpoint_id)
        .ok_or_else(|| anyhow::anyhow!("unknown endpoint_id: {endpoint_id}"))?;

    endpoint.manual_model_ids = normalized_manual.clone();
    endpoint.model_catalog_source = if endpoint.manual_model_ids.is_empty() {
        if endpoint.model_catalog_models.is_empty() {
            None
        } else {
            Some("discovered".to_string())
        }
    } else if endpoint.model_catalog_models.is_empty() {
        Some("manual".to_string())
    } else {
        Some("mixed".to_string())
    };
    endpoint.model_catalog_status = if endpoint.manual_model_ids.is_empty() {
        if endpoint.model_catalog_models.is_empty() {
            EndpointModelCatalogStatus::Unknown
        } else {
            EndpointModelCatalogStatus::Ready
        }
    } else if endpoint.model_catalog_models.is_empty() {
        EndpointModelCatalogStatus::ManualOnly
    } else {
        EndpointModelCatalogStatus::Ready
    };
    endpoint.updated_at = Utc::now();

    let public = public_endpoint_from_internal(endpoint);
    registry::save_registry(data_root, &registry).await?;
    Ok(public)
}

pub async fn refresh_provider_endpoint_model_catalog(
    data_root: &Path,
    provider_id: &str,
    endpoint_id: &str,
) -> Result<HarnessEndpointRecord> {
    let canonical = validation::normalize_provider_id(provider_id).ok_or_else(|| {
        anyhow::anyhow!("provider does not support harness endpoints: {provider_id}")
    })?;

    let endpoint_snapshot = {
        let _registry_write_guard = REGISTRY_WRITE_LOCK.lock().await;
        let registry = registry::load_registry(data_root).await?;
        let provider = registry
            .providers
            .get(canonical)
            .ok_or_else(|| anyhow::anyhow!("unknown provider endpoint config for {canonical}"))?;
        provider
            .endpoints
            .iter()
            .find(|ep| ep.id == endpoint_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("unknown endpoint_id: {endpoint_id}"))?
    };
    let secret = secrets::read_endpoint_secret(data_root, &endpoint_snapshot.secret_ref).await?;
    let discovery_result = match (
        model_catalog::supports_model_discovery(&endpoint_snapshot),
        endpoint_snapshot.provider_id.as_str(),
        endpoint_snapshot.auth_type.as_str(),
    ) {
        (true, PROVIDER_GEMINI, GEMINI_AUTH_TYPE_VERTEX_AI) => Err(anyhow::anyhow!(
            "model discovery is unsupported for Gemini Vertex AI service-account auth"
        )),
        (true, _, _) => {
            let api_key = secrets::endpoint_secret_api_key(&secret)?;
            model_catalog::discover_openai_models(
                &endpoint_snapshot.base_url,
                &endpoint_snapshot.auth_type,
                &api_key,
            )
            .await
        }
        (false, _, _) => Err(anyhow::anyhow!(
            "model discovery is unsupported for provider '{}' with api_shape '{}' and base_url '{}'",
            canonical,
            endpoint_snapshot.api_shape.as_str(),
            endpoint_snapshot.base_url
        )),
    };

    let _registry_write_guard = REGISTRY_WRITE_LOCK.lock().await;
    let mut registry = registry::load_registry(data_root).await?;
    let provider = registry
        .providers
        .get_mut(canonical)
        .ok_or_else(|| anyhow::anyhow!("unknown provider endpoint config for {canonical}"))?;
    let endpoint = provider
        .endpoints
        .iter_mut()
        .find(|ep| ep.id == endpoint_id)
        .ok_or_else(|| anyhow::anyhow!("unknown endpoint_id: {endpoint_id}"))?;

    endpoint.updated_at = Utc::now();
    match discovery_result {
        Ok(discovered_models) => {
            endpoint.model_catalog_models = discovered_models;
            endpoint.model_catalog_fetched_at = Some(Utc::now());
            endpoint.model_catalog_error = None;
            endpoint.model_catalog_source = if endpoint.manual_model_ids.is_empty() {
                Some("discovered".to_string())
            } else {
                Some("mixed".to_string())
            };
            endpoint.model_catalog_status = if endpoint.model_catalog_models.is_empty() {
                if endpoint.manual_model_ids.is_empty() {
                    EndpointModelCatalogStatus::Error
                } else {
                    EndpointModelCatalogStatus::ManualOnly
                }
            } else {
                EndpointModelCatalogStatus::Ready
            };
        }
        Err(err) => {
            endpoint.model_catalog_error =
                Some(model_catalog::truncate_discovery_error(&err.to_string()));
            endpoint.model_catalog_source = if endpoint.manual_model_ids.is_empty() {
                if endpoint.model_catalog_models.is_empty() {
                    None
                } else {
                    Some("discovered".to_string())
                }
            } else if endpoint.model_catalog_models.is_empty() {
                Some("manual".to_string())
            } else {
                Some("mixed".to_string())
            };
            endpoint.model_catalog_status = if endpoint.manual_model_ids.is_empty() {
                if endpoint.model_catalog_models.is_empty() {
                    EndpointModelCatalogStatus::Error
                } else {
                    EndpointModelCatalogStatus::Ready
                }
            } else if endpoint.model_catalog_models.is_empty() {
                EndpointModelCatalogStatus::ManualOnly
            } else {
                EndpointModelCatalogStatus::Ready
            };
        }
    }

    let public = public_endpoint_from_internal(endpoint);
    registry::save_registry(data_root, &registry).await?;
    Ok(public)
}

pub(super) fn public_endpoint_from_internal(
    endpoint: &HarnessEndpointRecordInternal,
) -> HarnessEndpointRecord {
    let manual_model_ids = validation::normalize_manual_model_ids(&endpoint.manual_model_ids);
    let model_catalog_models = model_catalog::merge_endpoint_model_records(
        &endpoint.model_catalog_models,
        &manual_model_ids,
    );
    HarnessEndpointRecord {
        id: endpoint.id.clone(),
        provider_id: endpoint.provider_id.clone(),
        name: endpoint.name.clone(),
        base_url: if endpoint.base_url.trim().is_empty() {
            None
        } else {
            Some(endpoint.base_url.clone())
        },
        api_shape: endpoint.api_shape,
        auth_type: endpoint.auth_type.clone(),
        model_override: endpoint.model_override.clone(),
        created_at: endpoint.created_at,
        updated_at: endpoint.updated_at,
        last_verification_status: endpoint.last_verification_status,
        last_verification_at: endpoint.last_verification_at,
        last_error: endpoint.last_error.clone(),
        has_api_key: true,
        model_catalog_status: endpoint.model_catalog_status,
        model_catalog_fetched_at: endpoint.model_catalog_fetched_at,
        model_catalog_error: endpoint.model_catalog_error.clone(),
        model_catalog_models,
        manual_model_ids,
        model_catalog_source: endpoint.model_catalog_source.clone(),
    }
}
