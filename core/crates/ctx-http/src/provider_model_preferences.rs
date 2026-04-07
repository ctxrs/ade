use serde_json::Value;

pub(crate) fn preferred_model_id_from_available_models(
    preferred_model_id: Option<String>,
    models: Option<&Value>,
) -> Option<String> {
    let preferred_model_id = preferred_model_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)?;
    let models = models?;
    if model_payload_contains_id(models, &preferred_model_id) {
        Some(preferred_model_id)
    } else {
        None
    }
}

fn model_payload_contains_id(models: &Value, target_id: &str) -> bool {
    let target_id = target_id.trim();
    if target_id.is_empty() {
        return false;
    }
    if models
        .get("current_model_id")
        .or_else(|| models.get("currentModelId"))
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|value| value == target_id)
    {
        return true;
    }
    extract_model_entries(models)
        .into_iter()
        .any(|model_id| model_id == target_id)
}

fn extract_model_entries(models: &Value) -> Vec<String> {
    let Some(entries) = models
        .get("models")
        .or_else(|| models.get("availableModels"))
        .or_else(|| models.get("available_models"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };

    entries
        .iter()
        .filter_map(|entry| {
            entry
                .get("id")
                .or_else(|| entry.get("modelId"))
                .or_else(|| entry.get("model_id"))
                .or_else(|| entry.get("name"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::preferred_model_id_from_available_models;

    #[test]
    fn omits_preference_when_catalog_does_not_contain_it() {
        let models = serde_json::json!({
            "current_model_id": "gpt-5.4/medium",
            "models": [
                {"id": "gpt-5.4/medium"},
                {"id": "gpt-5.4/high"}
            ]
        });

        assert_eq!(
            preferred_model_id_from_available_models(
                Some("gpt-5.4/xhigh".to_string()),
                Some(&models),
            ),
            None
        );
    }

    #[test]
    fn keeps_preference_when_catalog_contains_it() {
        let models = serde_json::json!({
            "current_model_id": "gpt-5.4/medium",
            "models": [
                {"id": "gpt-5.4/medium"},
                {"id": "gpt-5.4/xhigh"}
            ]
        });

        assert_eq!(
            preferred_model_id_from_available_models(
                Some(" gpt-5.4/xhigh ".to_string()),
                Some(&models),
            ),
            Some("gpt-5.4/xhigh".to_string())
        );
    }

    #[test]
    fn keeps_preference_when_catalog_only_has_camel_case_current_model_id() {
        let models = serde_json::json!({
            "currentModelId": "gpt-5.4/xhigh"
        });

        assert_eq!(
            preferred_model_id_from_available_models(
                Some("gpt-5.4/xhigh".to_string()),
                Some(&models),
            ),
            Some("gpt-5.4/xhigh".to_string())
        );
    }
}
