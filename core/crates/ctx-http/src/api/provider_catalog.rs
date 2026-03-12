pub(crate) fn provider_models_payload_is_final(models: &serde_json::Value) -> bool {
    let meta = models
        .get("meta")
        .and_then(serde_json::Value::as_object)
        .cloned()
        .unwrap_or_default();
    match meta
        .get("source_kind")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
    {
        "subscription" => {
            meta.get("refresh_pending")
                .and_then(serde_json::Value::as_bool)
                == Some(false)
        }
        "endpoint" => matches!(
            meta.get("catalog_status")
                .and_then(serde_json::Value::as_str),
            Some("ready") | Some("manual_only")
        ),
        _ => false,
    }
}

pub(crate) fn provider_supports_runtime_model_catalog(provider_id: &str) -> bool {
    matches!(provider_id, "codex" | "claude-crp" | "copilot" | "cursor")
}

fn provider_options_probe_failed(value: &serde_json::Value) -> bool {
    value.get("probe_ok").and_then(serde_json::Value::as_bool) == Some(false)
        || value
            .get("probe_error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| !message.trim().is_empty())
}

pub(crate) fn provider_options_cache_entry_is_authoritative(
    provider_id: &str,
    value: &serde_json::Value,
) -> bool {
    if provider_id != "codex" && provider_id != "claude-crp" && provider_id != "copilot" {
        return true;
    }
    if value
        .get("models")
        .is_some_and(provider_models_payload_is_final)
    {
        return true;
    }
    provider_options_probe_failed(value)
}

pub(crate) fn runtime_probe_models_payload(
    probe: &ctx_providers::crp::CrpModelsProbe,
) -> Option<serde_json::Value> {
    if probe.catalog_source.as_deref() != Some("live_remote") {
        return None;
    }
    Some(serde_json::json!({
        "models": probe.models,
        "current_model_id": probe.current_model_id,
        "meta": {
            "source_kind": "subscription",
            "catalog_source": "runtime_probe_live",
            "refresh_pending": false,
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::{
        provider_options_cache_entry_is_authoritative, provider_supports_runtime_model_catalog,
        runtime_probe_models_payload,
    };

    #[test]
    fn runtime_model_catalog_provider_set_matches_supported_live_discovery_paths() {
        assert!(provider_supports_runtime_model_catalog("codex"));
        assert!(provider_supports_runtime_model_catalog("claude-crp"));
        assert!(provider_supports_runtime_model_catalog("copilot"));
        assert!(provider_supports_runtime_model_catalog("cursor"));
        assert!(!provider_supports_runtime_model_catalog("gemini"));
    }

    #[test]
    fn pinned_subscription_catalog_is_not_authoritative_for_discovery_cache() {
        let cached = serde_json::json!({
            "provider_id": "codex",
            "probe_ok": true,
            "models": {
                "models": [{ "id": "gpt-5.4/medium" }],
                "current_model_id": "gpt-5.4/medium",
                "meta": {
                    "source_kind": "subscription",
                    "catalog_source": "codex_bundle_pinned",
                    "refresh_pending": true,
                },
            },
        });

        assert!(!provider_options_cache_entry_is_authoritative(
            "codex", &cached,
        ));
    }

    #[test]
    fn live_runtime_probe_catalog_is_authoritative_for_discovery_cache() {
        let probe = ctx_providers::crp::CrpModelsProbe {
            models: vec![ctx_providers::crp::CrpModelInfo {
                id: "gpt-5.4".to_string(),
                name: Some("gpt-5.4".to_string()),
            }],
            current_model_id: Some("gpt-5.4".to_string()),
            catalog_source: Some("live_remote".to_string()),
        };
        let cached = serde_json::json!({
            "provider_id": "codex",
            "probe_ok": true,
            "models": runtime_probe_models_payload(&probe),
        });

        assert!(provider_options_cache_entry_is_authoritative(
            "codex", &cached,
        ));
    }

    #[test]
    fn explicit_probe_failure_remains_authoritative_until_retry() {
        let cached = serde_json::json!({
            "provider_id": "codex",
            "probe_ok": false,
            "probe_error": "runtime probe failed",
        });

        assert!(provider_options_cache_entry_is_authoritative(
            "codex", &cached,
        ));
    }

    #[test]
    fn local_runtime_catalog_is_not_promoted_to_live_models() {
        let probe = ctx_providers::crp::CrpModelsProbe {
            models: vec![ctx_providers::crp::CrpModelInfo {
                id: "gpt-5.3-codex".to_string(),
                name: Some("gpt-5.3-codex".to_string()),
            }],
            current_model_id: Some("gpt-5.3-codex".to_string()),
            catalog_source: Some("local_bundle".to_string()),
        };

        assert!(runtime_probe_models_payload(&probe).is_none());
    }
}
