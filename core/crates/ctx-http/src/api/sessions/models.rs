use super::*;

const DEFAULT_REASONING_EFFORT: &str = "medium";
const KNOWN_EFFORT_IDS: [&str; 6] = ["none", "minimal", "low", "medium", "high", "xhigh"];

#[derive(Debug, Clone)]
struct ModelInfo {
    base: String,
    effort: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct ModelCatalog {
    pub(super) full_ids: Vec<String>,
    base_ids: Vec<String>,
    efforts_by_base: HashMap<String, Vec<String>>,
    full_id_by_base_effort: HashMap<String, HashMap<String, String>>,
    info_by_full_id: HashMap<String, ModelInfo>,
}

#[derive(Debug, Clone)]
pub(super) struct ResolvedModel {
    pub(super) model_id: String,
}

pub(super) fn normalize_effort_id(value: &str) -> String {
    let raw = value.trim().to_lowercase();
    match raw.as_str() {
        "extra_high" | "extra-high" | "extra high" | "extrahigh" => "xhigh".to_string(),
        _ => raw,
    }
}

fn is_known_effort_id(value: &str) -> bool {
    let norm = normalize_effort_id(value);
    KNOWN_EFFORT_IDS.iter().any(|id| *id == norm)
}

pub(super) fn split_model_id(full: &str) -> (String, Option<String>) {
    let trimmed = full.trim();
    if trimmed.is_empty() {
        return (String::new(), None);
    }
    if let Some(idx) = trimmed.rfind('/') {
        if idx > 0 && idx + 1 < trimmed.len() {
            let base = trimmed[..idx].to_string();
            let suffix = trimmed[idx + 1..].trim().to_string();
            if !suffix.is_empty() {
                return (base, Some(suffix));
            }
        }
    }
    (trimmed.to_string(), None)
}

fn has_trailing_paren_suffix(name: &str, suffix: &str) -> bool {
    let trimmed = name.trim_end();
    if !trimmed.ends_with(')') {
        return false;
    }
    let Some(start) = trimmed.rfind('(') else {
        return false;
    };
    let inner = trimmed[start + 1..trimmed.len() - 1].trim();
    normalize_effort_id(inner) == normalize_effort_id(suffix)
}

fn order_effort_ids(list: &mut [String]) {
    let order_index = |value: &str| {
        let norm = normalize_effort_id(value);
        KNOWN_EFFORT_IDS
            .iter()
            .position(|id| *id == norm)
            .unwrap_or(usize::MAX)
    };
    list.sort_by(|a, b| {
        let ia = order_index(a);
        let ib = order_index(b);
        if ia != ib {
            return ia.cmp(&ib);
        }
        a.cmp(b)
    });
}

fn build_model_catalog(models: &serde_json::Value) -> Option<ModelCatalog> {
    let entries = extract_model_entries(models);
    if entries.is_empty() {
        return None;
    }
    let mut full_ids = HashSet::new();
    let mut base_ids = HashSet::new();
    let mut info_by_full_id = HashMap::new();
    let mut raw_efforts_by_base: HashMap<String, HashSet<String>> = HashMap::new();
    let mut full_id_by_base_effort: HashMap<String, HashMap<String, String>> = HashMap::new();

    for (id, name) in entries {
        let trimmed = id.trim();
        if trimmed.is_empty() {
            continue;
        }
        let (base_candidate, suffix) = split_model_id(trimmed);
        let effort = suffix.and_then(|s| {
            if is_known_effort_id(&s)
                || name
                    .as_deref()
                    .map(|n| has_trailing_paren_suffix(n, &s))
                    .unwrap_or(false)
            {
                Some(s)
            } else {
                None
            }
        });
        let base = if effort.is_some() {
            base_candidate
        } else {
            trimmed.to_string()
        };
        base_ids.insert(base.clone());
        full_ids.insert(trimmed.to_string());
        info_by_full_id.insert(
            trimmed.to_string(),
            ModelInfo {
                base: base.clone(),
                effort: effort.clone(),
            },
        );
        if let Some(effort) = effort {
            raw_efforts_by_base
                .entry(base.clone())
                .or_default()
                .insert(effort.clone());
            full_id_by_base_effort
                .entry(base)
                .or_default()
                .insert(normalize_effort_id(&effort), trimmed.to_string());
        }
    }

    let mut efforts_by_base = HashMap::new();
    for (base, efforts) in raw_efforts_by_base {
        let mut list = efforts.into_iter().collect::<Vec<_>>();
        order_effort_ids(&mut list);
        efforts_by_base.insert(base, list);
    }

    let mut full_ids = full_ids.into_iter().collect::<Vec<_>>();
    full_ids.sort();
    let mut base_ids = base_ids.into_iter().collect::<Vec<_>>();
    base_ids.sort();

    Some(ModelCatalog {
        full_ids,
        base_ids,
        efforts_by_base,
        full_id_by_base_effort,
        info_by_full_id,
    })
}

fn pick_default_effort(efforts: &[String]) -> Option<String> {
    let medium = efforts
        .iter()
        .find(|e| normalize_effort_id(e) == DEFAULT_REASONING_EFFORT)
        .cloned();
    medium.or_else(|| efforts.first().cloned())
}

pub(super) fn resolve_model_id(
    requested_model: Option<&str>,
    requested_effort: Option<&str>,
    fallback_model: Option<&str>,
    catalog: Option<&ModelCatalog>,
) -> Result<ResolvedModel, String> {
    let mut model = requested_model
        .or(fallback_model)
        .unwrap_or("")
        .trim()
        .to_string();
    if model.is_empty() {
        return Err("model is required".to_string());
    }
    let effort_input = requested_effort
        .map(|e| e.trim())
        .filter(|e| !e.is_empty())
        .map(|e| e.to_string());

    if let Some(catalog) = catalog {
        let model_known = catalog.full_ids.contains(&model) || catalog.base_ids.contains(&model);
        if !model_known && requested_model.is_some() {
            return Err(format!(
                "unknown model '{model}'; available models: {}",
                catalog.full_ids.join(", ")
            ));
        }

        let info = catalog.info_by_full_id.get(&model);
        let base = info
            .map(|i| i.base.clone())
            .unwrap_or_else(|| model.clone());
        let existing_effort = info.and_then(|i| i.effort.clone());
        let available_efforts = catalog
            .efforts_by_base
            .get(&base)
            .cloned()
            .unwrap_or_default();
        let supports_default_effort = available_efforts.len() >= 2;
        let effort_map = catalog.full_id_by_base_effort.get(&base);

        if let Some(req_effort) = effort_input {
            let req_norm = normalize_effort_id(&req_effort);
            if let Some(existing) = existing_effort.as_ref() {
                if normalize_effort_id(existing) != req_norm {
                    return Err(format!(
                        "model '{model}' already includes effort '{existing}'; requested '{req_effort}'"
                    ));
                }
                return Ok(ResolvedModel { model_id: model });
            }
            if let Some(map) = effort_map {
                if let Some(full_id) = map.get(&req_norm) {
                    return Ok(ResolvedModel {
                        model_id: full_id.clone(),
                    });
                }
            }
            let efforts = if available_efforts.is_empty() {
                effort_map
                    .map(|map| map.keys().cloned().collect::<Vec<_>>())
                    .unwrap_or_default()
            } else {
                available_efforts.clone()
            };
            if efforts.is_empty() {
                return Err(format!("model '{base}' does not support reasoning_effort"));
            }
            return Err(format!(
                "invalid reasoning_effort '{req_effort}' for model '{base}'; available: {}",
                efforts.join(", ")
            ));
        }

        if existing_effort.is_some() {
            return Ok(ResolvedModel { model_id: model });
        }

        if supports_default_effort {
            if let Some(default_effort) = pick_default_effort(&available_efforts) {
                let default_norm = normalize_effort_id(&default_effort);
                if let Some(map) = effort_map {
                    if let Some(full_id) = map.get(&default_norm) {
                        return Ok(ResolvedModel {
                            model_id: full_id.clone(),
                        });
                    }
                }
            }
        } else if available_efforts.len() == 1 {
            let default_effort = available_efforts[0].clone();
            let default_norm = normalize_effort_id(&default_effort);
            if let Some(map) = effort_map {
                if let Some(full_id) = map.get(&default_norm) {
                    return Ok(ResolvedModel {
                        model_id: full_id.clone(),
                    });
                }
            }
        }

        return Ok(ResolvedModel { model_id: model });
    }

    if let Some(req_effort) = effort_input {
        let (_, suffix) = split_model_id(&model);
        if suffix.is_none() {
            model = format!("{}/{}", model, req_effort);
            return Ok(ResolvedModel { model_id: model });
        }
    }

    Ok(ResolvedModel { model_id: model })
}

pub(super) async fn load_provider_model_catalog(
    state: &Arc<AppState>,
    workspace: &Workspace,
    provider_id: &str,
) -> Result<Option<ModelCatalog>, String> {
    let install_target =
        crate::execution_effective::effective_install_target(state.as_ref(), workspace.id)
            .await
            .map_err(|err| {
                format!("workspace execution settings unavailable for provider options: {err:#}")
            })?;
    let cache_key = format!(
        "{}/{}/{}",
        workspace.id.0,
        install_target.as_str(),
        provider_id
    );
    if let Some(entry) = state
        .providers
        .options_cache
        .lock()
        .await
        .get(&cache_key)
        .filter(|entry| {
            crate::api::provider_catalog::provider_options_cache_entry_is_authoritative(
                provider_id,
                &entry.value,
            )
        })
    {
        if let Some(models) = entry.value.get("models") {
            if let Some(catalog) = build_model_catalog(models) {
                return Ok(Some(catalog));
            }
        }
    }

    if provider_id == "copilot" {
        let managed = crate::installer::load_agent_server_config(&state.core.data_root)
            .await
            .unwrap_or_default();
        let matrix = crate::provider_matrix::load_matrix_cached(
            &state.core.data_root,
            &state.providers.matrix_cache,
        )
        .await;
        let provider_status = crate::api::providers::provider_status_for_target(
            state,
            &managed,
            &matrix,
            provider_id,
            install_target,
        )
        .await;
        if let Some(version) = provider_status.version.as_deref() {
            if let Some(models_value) =
                crate::provider_accounts::copilot_models_value_for_version(version)
            {
                if let Some(catalog) = build_model_catalog(&models_value) {
                    return Ok(Some(catalog));
                }
            }
        }
    }

    let source_config =
        crate::harness_sources::get_provider_source_config(&state.core.data_root, provider_id)
            .await
            .ok();
    if let Some(config) = source_config.as_ref() {
        if config.selected_source_kind == crate::harness_sources::HarnessSourceKind::Endpoint {
            let selected_endpoint_id = config.selected_endpoint_id.as_deref().ok_or_else(|| {
                format!(
                    "selected source is endpoint for '{provider_id}' but no endpoint is selected"
                )
            })?;
            let endpoint = config
                .endpoints
                .iter()
                .find(|candidate| candidate.id == selected_endpoint_id)
                .ok_or_else(|| {
                    format!(
                        "selected endpoint '{selected_endpoint_id}' for '{provider_id}' was not found"
                    )
                })?;

            let now = chrono::Utc::now();
            if crate::harness_sources::endpoint_model_catalog_is_stale(endpoint, now) {
                let data_root = state.core.data_root.clone();
                let provider_id_for_refresh = provider_id.to_string();
                let endpoint_id_for_refresh = endpoint.id.clone();
                tokio::spawn(async move {
                    let _ = crate::harness_sources::refresh_provider_endpoint_model_catalog(
                        &data_root,
                        &provider_id_for_refresh,
                        &endpoint_id_for_refresh,
                    )
                    .await;
                });
            }

            let models_value = serde_json::json!({
                "models": endpoint.model_catalog_models,
                "current_model_id": endpoint.model_override,
            });
            if let Some(models) = build_model_catalog(&models_value) {
                let mut value = serde_json::json!({
                    "provider_id": provider_id,
                    "workspace_id": workspace.id.0,
                    "installed": true,
                    "probe_ok": true,
                    "supports_load": false,
                    "auth_required": false,
                    "models": models_value,
                    "probed_at": now.to_rfc3339(),
                });
                value["source"] = serde_json::to_value(config).unwrap_or(serde_json::Value::Null);
                value = redact_json_value(value);
                state.providers.options_cache.lock().await.insert(
                    cache_key,
                    crate::daemon::CachedProviderOptions {
                        cached_at: std::time::Instant::now(),
                        value,
                    },
                );
                return Ok(Some(models));
            }

            return Ok(None);
        }
    }

    if provider_id != "codex" && provider_id != "claude-crp" {
        return Ok(None);
    }

    let cfg = installer::load_agent_server_config(&state.core.data_root)
        .await
        .unwrap_or_default();
    let runtime_command = installer::resolve_runtime_provider_command_for_target(
        &cfg,
        provider_id,
        Some(install_target),
    )
    .map_err(|e| format!("runtime_command_invalid: provider={provider_id} error={e}"))?
    .ok_or_else(|| {
        format!(
            "runtime_command_missing: provider={provider_id} (configure an absolute runtime command)"
        )
    })?;
    let command = runtime_command.command_abs_path;
    let args = runtime_command.args;

    let mut env = match crate::provider_probe::provider_probe_env_for_workspace_runtime(
        state,
        workspace,
        provider_id,
    )
    .await
    {
        Ok((_source, env)) => env,
        Err(err) => {
            tracing::warn!(
                provider_id = provider_id,
                "provider probe runtime env failed: {}",
                logs::redact_sensitive(&err)
            );
            return Ok(None);
        }
    };
    installer::prepend_runtime_bin_dirs_to_provider_path_for_target(
        &mut env,
        &cfg,
        provider_id,
        &state.core.data_root,
        Some(install_target),
    );

    let probe = match probe_crp_models(
        provider_id,
        command,
        args,
        PathBuf::from(&workspace.root_path),
        env,
    )
    .await
    {
        Ok(probe) => probe,
        Err(e) => {
            tracing::warn!(
                provider_id = provider_id,
                "provider options probe failed: {}",
                logs::redact_sensitive(&e.to_string())
            );
            return Ok(None);
        }
    };

    let Some(models_value) = crate::api::provider_catalog::runtime_probe_models_payload(&probe)
    else {
        return Ok(None);
    };
    if let Some(models) = build_model_catalog(&models_value) {
        let mut value = serde_json::json!({
            "provider_id": provider_id,
            "workspace_id": workspace.id.0,
            "installed": true,
            "probe_ok": true,
            "supports_load": false,
            "auth_required": false,
            "models": models_value,
            "probed_at": chrono::Utc::now().to_rfc3339(),
        });
        value = redact_json_value(value);
        state.providers.options_cache.lock().await.insert(
            cache_key,
            crate::daemon::CachedProviderOptions {
                cached_at: std::time::Instant::now(),
                value,
            },
        );
        return Ok(Some(models));
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::load_provider_model_catalog;

    use std::collections::HashMap;
    use std::sync::Arc;

    use ctx_core::models::VcsKind;
    use ctx_store::StoreManager;

    use crate::daemon::AppState;
    use crate::settings::{ExecutionMode, ExecutionSettings, Settings};

    #[tokio::test]
    async fn load_provider_model_catalog_reads_target_scoped_options_cache() {
        let temp = tempfile::tempdir().expect("tempdir");
        let stores = StoreManager::open(temp.path()).await.expect("open stores");
        let state = Arc::new(AppState::new(
            temp.path().to_path_buf(),
            stores,
            HashMap::new(),
            "http://127.0.0.1:4310".to_string(),
            None,
        ));
        let workspace = state
            .global_store()
            .create_workspace(
                "ws".to_string(),
                temp.path().join("repo").to_string_lossy().to_string(),
                VcsKind::Git,
            )
            .await
            .expect("create workspace");
        crate::settings::save_settings(
            state.global_store(),
            &Settings {
                execution: Some(ExecutionSettings {
                    mode: ExecutionMode::Container,
                    ..ExecutionSettings::default()
                }),
                ..Settings::default()
            },
        )
        .await
        .expect("save settings");

        state.providers.options_cache.lock().await.insert(
            format!("{}/container/codex", workspace.id.0),
            crate::daemon::CachedProviderOptions {
                cached_at: std::time::Instant::now(),
                value: serde_json::json!({
                    "models": {
                        "models": [
                            { "id": "gpt-5" },
                            { "id": "gpt-5/high" }
                        ],
                        "current_model_id": "gpt-5",
                        "meta": {
                            "source_kind": "subscription",
                            "catalog_source": "runtime_probe_live",
                            "refresh_pending": false
                        }
                    }
                }),
            },
        );

        let catalog = load_provider_model_catalog(&state, &workspace, "codex")
            .await
            .expect("load catalog")
            .expect("catalog");

        assert!(catalog.full_ids.iter().any(|id| id == "gpt-5"));
        assert!(catalog.full_ids.iter().any(|id| id == "gpt-5/high"));
    }
}
