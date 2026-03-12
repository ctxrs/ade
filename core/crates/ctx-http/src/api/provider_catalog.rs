const RUNTIME_MODEL_CATALOG_PROVIDER_IDS: &[&str] =
    &["codex", "claude-crp", "copilot", "cursor", "gemini", "qwen"];

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
    RUNTIME_MODEL_CATALOG_PROVIDER_IDS.contains(&provider_id)
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
    if !provider_supports_runtime_model_catalog(provider_id) {
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

fn runtime_probe_catalog_is_live(
    provider_id: &str,
    probe: &ctx_providers::crp::CrpModelsProbe,
) -> bool {
    match probe
        .catalog_source
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some("live_remote") => true,
        // Gemini's ACP bridge does not currently preserve a catalog_source marker
        // on models.list responses, but the returned catalog is still authoritative.
        None if provider_id == "gemini" => true,
        _ => false,
    }
}

pub(crate) fn runtime_probe_models_payload(
    provider_id: &str,
    probe: &ctx_providers::crp::CrpModelsProbe,
    fallback_current_model_id: Option<&str>,
) -> Option<serde_json::Value> {
    if !runtime_probe_catalog_is_live(provider_id, probe) {
        return None;
    }
    let current_model_id = probe
        .current_model_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| fallback_current_model_id.map(str::to_string));
    Some(serde_json::json!({
        "models": probe.models,
        "current_model_id": current_model_id,
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
        assert!(provider_supports_runtime_model_catalog("gemini"));
        assert!(provider_supports_runtime_model_catalog("qwen"));
        assert!(!provider_supports_runtime_model_catalog("kimi"));
        assert!(!provider_supports_runtime_model_catalog("amp"));
        assert!(!provider_supports_runtime_model_catalog("mistral"));
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
            "models": runtime_probe_models_payload("codex", &probe, None),
        });

        assert!(provider_options_cache_entry_is_authoritative(
            "codex", &cached,
        ));
    }

    #[test]
    fn explicit_probe_failure_remains_authoritative_until_retry() {
        let cached = serde_json::json!({
            "provider_id": "gemini",
            "probe_ok": false,
            "probe_error": "runtime probe failed",
        });

        assert!(provider_options_cache_entry_is_authoritative(
            "gemini", &cached,
        ));
    }

    #[test]
    fn discovery_provider_without_final_models_is_not_authoritative() {
        let cached = serde_json::json!({
            "provider_id": "qwen",
            "probe_ok": true,
            "models": null,
        });

        assert!(!provider_options_cache_entry_is_authoritative(
            "qwen", &cached,
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

        assert!(runtime_probe_models_payload("codex", &probe, None).is_none());
    }

    #[test]
    fn gemini_probe_without_catalog_source_is_treated_as_live() {
        let probe = ctx_providers::crp::CrpModelsProbe {
            models: vec![ctx_providers::crp::CrpModelInfo {
                id: "auto-gemini-3".to_string(),
                name: Some("Auto (Gemini 3)".to_string()),
            }],
            current_model_id: Some("auto-gemini-3".to_string()),
            catalog_source: None,
        };

        let payload = runtime_probe_models_payload("gemini", &probe, Some("auto-gemini-3"))
            .expect("gemini payload");
        assert_eq!(
            payload
                .pointer("/meta/catalog_source")
                .and_then(serde_json::Value::as_str),
            Some("runtime_probe_live")
        );
        assert_eq!(
            payload
                .get("current_model_id")
                .and_then(serde_json::Value::as_str),
            Some("auto-gemini-3")
        );
    }

    #[test]
    fn non_gemini_probe_without_catalog_source_is_not_promoted() {
        let probe = ctx_providers::crp::CrpModelsProbe {
            models: vec![ctx_providers::crp::CrpModelInfo {
                id: "gpt-5.4".to_string(),
                name: Some("gpt-5.4".to_string()),
            }],
            current_model_id: Some("gpt-5.4".to_string()),
            catalog_source: None,
        };

        assert!(runtime_probe_models_payload("codex", &probe, None).is_none());
    }

    #[test]
    fn missing_current_model_id_uses_supplied_fallback() {
        let probe = ctx_providers::crp::CrpModelsProbe {
            models: vec![ctx_providers::crp::CrpModelInfo {
                id: "auto-gemini-3".to_string(),
                name: Some("Auto (Gemini 3)".to_string()),
            }],
            current_model_id: None,
            catalog_source: Some("live_remote".to_string()),
        };

        let payload = runtime_probe_models_payload("gemini", &probe, Some("auto-gemini-3"))
            .expect("gemini payload");
        assert_eq!(
            payload
                .get("current_model_id")
                .and_then(serde_json::Value::as_str),
            Some("auto-gemini-3")
        );
    }
}
