use super::catalog::{is_known_effort_id, normalize_effort_id, split_model_id};
use super::*;

pub(crate) fn compose_model_id(model_id: &str, reasoning_effort: Option<&str>) -> String {
    let base = model_id.trim();
    if base.is_empty() {
        return String::new();
    }
    match reasoning_effort
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(effort) => format!("{base}/{effort}"),
        None => base.to_string(),
    }
}

fn pick_default_effort(efforts: &[String]) -> Option<String> {
    let medium = efforts
        .iter()
        .find(|e| normalize_effort_id(e) == DEFAULT_REASONING_EFFORT)
        .cloned();
    medium.or_else(|| efforts.first().cloned())
}

fn build_resolved_model(model_id: String, reasoning_effort: Option<String>) -> ResolvedModel {
    let model_id = model_id.trim().to_string();
    let reasoning_effort = reasoning_effort
        .map(|value| normalize_effort_id(&value))
        .filter(|value| !value.is_empty());
    let full_model_id = compose_model_id(&model_id, reasoning_effort.as_deref());
    ResolvedModel {
        model_id,
        reasoning_effort,
        full_model_id,
    }
}

fn resolved_from_full_model_id(
    catalog: Option<&ModelCatalog>,
    full_model_id: &str,
) -> ResolvedModel {
    let trimmed = full_model_id.trim();
    if let Some(catalog) = catalog {
        if let Some(info) = catalog.info_by_full_id.get(trimmed) {
            return build_resolved_model(info.base.clone(), info.effort.clone());
        }
    }
    let (base, suffix) = split_model_id(trimmed);
    let reasoning_effort = suffix.filter(|value| is_known_effort_id(value));
    if reasoning_effort.is_some() {
        return build_resolved_model(base, reasoning_effort);
    }
    build_resolved_model(trimmed.to_string(), None)
}

pub(crate) fn resolve_model_id(
    requested_model: Option<&str>,
    requested_effort: Option<&str>,
    fallback_model: Option<&str>,
    catalog: Option<&ModelCatalog>,
) -> Result<ResolvedModel, String> {
    let model = requested_model
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
            if let Some(req_effort) = effort_input {
                let (_, suffix) = split_model_id(&model);
                if suffix.is_none() {
                    return Ok(build_resolved_model(model, Some(req_effort)));
                }
            }
            return Ok(resolved_from_full_model_id(None, &model));
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
                return Ok(resolved_from_full_model_id(Some(catalog), &model));
            }
            if let Some(map) = effort_map {
                if let Some(full_id) = map.get(&req_norm) {
                    return Ok(resolved_from_full_model_id(Some(catalog), full_id));
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
            return Ok(resolved_from_full_model_id(Some(catalog), &model));
        }

        if supports_default_effort {
            if let Some(default_effort) = pick_default_effort(&available_efforts) {
                let default_norm = normalize_effort_id(&default_effort);
                if let Some(map) = effort_map {
                    if let Some(full_id) = map.get(&default_norm) {
                        return Ok(resolved_from_full_model_id(Some(catalog), full_id));
                    }
                }
            }
        } else if available_efforts.len() == 1 {
            let default_effort = available_efforts[0].clone();
            let default_norm = normalize_effort_id(&default_effort);
            if let Some(map) = effort_map {
                if let Some(full_id) = map.get(&default_norm) {
                    return Ok(resolved_from_full_model_id(Some(catalog), full_id));
                }
            }
        }

        return Ok(resolved_from_full_model_id(Some(catalog), &model));
    }

    if let Some(req_effort) = effort_input {
        let (_, suffix) = split_model_id(&model);
        if suffix.is_none() {
            return Ok(build_resolved_model(model, Some(req_effort)));
        }
    }

    Ok(resolved_from_full_model_id(None, &model))
}
