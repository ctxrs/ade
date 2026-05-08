use super::*;

#[test]
fn cache_key_provider_matcher_works() {
    assert!(cache_key_matches_provider(
        "7f72430e-4c43-499f-b54d-6ce2deaed4a0/host/codex",
        "codex"
    ));
    assert!(!cache_key_matches_provider(
        "7f72430e-4c43-499f-b54d-6ce2deaed4a0/host/codex",
        "codex-crp"
    ));
    assert!(!cache_key_matches_provider(
        "7f72430e-4c43-499f-b54d-6ce2deaed4a0/container/claude-crp",
        "codex"
    ));
    assert!(!cache_key_matches_provider("not-a-key", "codex"));
}

#[test]
fn classify_probe_error_detects_auth_required_messages() {
    let (status, auth_required, endpoint_status) =
        classify_probe_error("401 unauthorized: missing api key");
    assert_eq!(status, "auth_required");
    assert_eq!(auth_required, Some(true));
    assert_eq!(endpoint_status, HarnessEndpointVerificationStatus::Invalid);
}

#[test]
fn classify_probe_error_treats_models_list_protocol_failures_as_generic_errors() {
    let message = "CRP models.list probe timed out after 10s; stderr_tail=Cursor CLI authenticated | Invalid message { type: 'models.list' }";
    let (status, auth_required, endpoint_status) = classify_probe_error(message);
    assert_eq!(status, "error");
    assert_eq!(auth_required, Some(false));
    assert_eq!(endpoint_status, HarnessEndpointVerificationStatus::Error);
}

#[test]
fn selected_endpoint_from_harness_config_prefers_endpoint_selection() {
    let endpoint =
        selected_endpoint_from_harness_config(Some(harness_sources::HarnessProviderSourceConfig {
            provider_id: "codex".to_string(),
            selected_source_kind: HarnessSourceKind::Endpoint,
            selected_endpoint_id: Some("ep-123".to_string()),
            endpoints: Vec::new(),
        }));
    assert_eq!(endpoint.as_deref(), Some("ep-123"));

    let subscription =
        selected_endpoint_from_harness_config(Some(harness_sources::HarnessProviderSourceConfig {
            provider_id: "codex".to_string(),
            selected_source_kind: HarnessSourceKind::Subscription,
            selected_endpoint_id: Some("ep-123".to_string()),
            endpoints: Vec::new(),
        }));
    assert!(subscription.is_none());
}

#[test]
fn selected_endpoint_record_from_harness_config_returns_selected_record() {
    let selected = selected_endpoint_record_from_harness_config(Some(
        &harness_sources::HarnessProviderSourceConfig {
            provider_id: "codex".to_string(),
            selected_source_kind: HarnessSourceKind::Endpoint,
            selected_endpoint_id: Some("ep-2".to_string()),
            endpoints: vec![test_endpoint("ep-1"), test_endpoint("ep-2")],
        },
    ))
    .expect("selected endpoint");
    assert_eq!(selected.id, "ep-2");

    let missing = selected_endpoint_record_from_harness_config(Some(
        &harness_sources::HarnessProviderSourceConfig {
            provider_id: "codex".to_string(),
            selected_source_kind: HarnessSourceKind::Endpoint,
            selected_endpoint_id: Some("ep-3".to_string()),
            endpoints: vec![test_endpoint("ep-1"), test_endpoint("ep-2")],
        },
    ));
    assert!(missing.is_none());
}

#[test]
fn endpoint_models_payload_includes_models_and_meta() {
    let now = Utc::now();
    let mut endpoint = test_endpoint("ep-1");
    endpoint.model_override = Some("openai/gpt-5.2".to_string());
    endpoint.model_catalog_status = harness_sources::EndpointModelCatalogStatus::Ready;
    endpoint.model_catalog_fetched_at = Some(now);
    endpoint.model_catalog_source = Some("mixed".to_string());
    endpoint.model_catalog_models = vec![harness_sources::EndpointModelRecord {
        id: "openai/gpt-5.2".to_string(),
        name: Some("GPT-5.2".to_string()),
    }];

    let payload = endpoint_models_payload("codex", &endpoint, now);
    assert_eq!(
        payload
            .pointer("/models/0/id")
            .and_then(serde_json::Value::as_str),
        Some("openai/gpt-5.2")
    );
    assert_eq!(
        payload
            .pointer("/current_model_id")
            .and_then(serde_json::Value::as_str),
        Some("openai/gpt-5.2")
    );
    assert_eq!(
        payload
            .pointer("/meta/catalog_status")
            .and_then(serde_json::Value::as_str),
        Some("ready")
    );
    assert_eq!(
        payload
            .pointer("/meta/catalog_source")
            .and_then(serde_json::Value::as_str),
        Some("mixed")
    );
    assert_eq!(
        payload
            .pointer("/meta/source_kind")
            .and_then(serde_json::Value::as_str),
        Some("endpoint")
    );
    assert_eq!(
        payload
            .pointer("/meta/stale")
            .and_then(serde_json::Value::as_bool),
        Some(false)
    );
}

#[test]
fn endpoint_models_payload_uses_droid_custom_model_selector() {
    let now = Utc::now();
    let mut endpoint = test_endpoint("ep-1");
    endpoint.base_url = Some("https://openrouter.ai/api/v1".to_string());
    endpoint.model_override = Some("openai/gpt-5.2".to_string());

    let payload = endpoint_models_payload("droid", &endpoint, now);
    assert_eq!(
        payload
            .pointer("/current_model_id")
            .and_then(serde_json::Value::as_str),
        Some("custom:openai/gpt-5.2-[openrouter]-0")
    );
}

#[test]
fn endpoint_models_payload_merges_manual_model_ids() {
    let now = Utc::now();
    let mut endpoint = test_endpoint("ep-1");
    endpoint.model_catalog_models = vec![harness_sources::EndpointModelRecord {
        id: "openai/gpt-5.2".to_string(),
        name: Some("GPT-5.2".to_string()),
    }];
    endpoint.manual_model_ids = vec![
        "openai/gpt-5.2".to_string(),
        "custom/manual-model".to_string(),
    ];
    endpoint.model_catalog_source = Some("mixed".to_string());
    endpoint.model_catalog_status = harness_sources::EndpointModelCatalogStatus::Ready;

    let payload = endpoint_models_payload("codex", &endpoint, now);
    let models = payload
        .get("models")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .expect("models array");
    assert_eq!(models.len(), 2);
    assert_eq!(
        models[1].get("id").and_then(serde_json::Value::as_str),
        Some("custom/manual-model")
    );
}

#[test]
fn endpoint_selection_is_active_requires_selected_endpoint_record() {
    let active = harness_sources::HarnessProviderSourceConfig {
        provider_id: "codex".to_string(),
        selected_source_kind: HarnessSourceKind::Endpoint,
        selected_endpoint_id: Some("ep-1".to_string()),
        endpoints: vec![test_endpoint("ep-1")],
    };
    assert!(endpoint_selection_is_active(&active));

    let missing = harness_sources::HarnessProviderSourceConfig {
        provider_id: "codex".to_string(),
        selected_source_kind: HarnessSourceKind::Endpoint,
        selected_endpoint_id: Some("ep-2".to_string()),
        endpoints: vec![test_endpoint("ep-1")],
    };
    assert!(!endpoint_selection_is_active(&missing));
}

#[test]
fn endpoint_supports_model_catalog_verify_requires_openai_shape_and_base_url() {
    let mut endpoint = test_endpoint("ep-1");
    assert!(endpoint_supports_model_catalog_verify(&endpoint));

    endpoint.api_shape = HarnessApiShape::AnthropicMessages;
    assert!(!endpoint_supports_model_catalog_verify(&endpoint));

    endpoint.api_shape = HarnessApiShape::OpenaiResponses;
    endpoint.base_url = None;
    assert!(!endpoint_supports_model_catalog_verify(&endpoint));
}

#[test]
fn endpoint_catalog_verify_outcome_ready_and_manual_only_are_valid() {
    let mut ready = test_endpoint("ep-ready");
    ready.model_catalog_status = harness_sources::EndpointModelCatalogStatus::Ready;
    let (status, auth_required, message, endpoint_status) = endpoint_catalog_verify_outcome(&ready);
    assert_eq!(status, "ok");
    assert_eq!(auth_required, Some(false));
    assert!(message.is_none());
    assert_eq!(endpoint_status, HarnessEndpointVerificationStatus::Valid);

    let mut manual = test_endpoint("ep-manual");
    manual.model_catalog_status = harness_sources::EndpointModelCatalogStatus::ManualOnly;
    let (status, auth_required, message, endpoint_status) =
        endpoint_catalog_verify_outcome(&manual);
    assert_eq!(status, "ok");
    assert_eq!(auth_required, Some(false));
    assert!(message.is_none());
    assert_eq!(endpoint_status, HarnessEndpointVerificationStatus::Valid);
}

#[test]
fn endpoint_catalog_verify_outcome_classifies_auth_errors() {
    let mut endpoint = test_endpoint("ep-auth");
    endpoint.model_catalog_status = harness_sources::EndpointModelCatalogStatus::Error;
    endpoint.model_catalog_error = Some("model discovery failed with status 401".to_string());
    let (status, auth_required, message, endpoint_status) =
        endpoint_catalog_verify_outcome(&endpoint);
    assert_eq!(status, "auth_required");
    assert_eq!(auth_required, Some(true));
    assert!(message
        .as_deref()
        .is_some_and(|value| value.contains("status 401")));
    assert_eq!(endpoint_status, HarnessEndpointVerificationStatus::Invalid);
}

#[test]
fn endpoint_catalog_runtime_probe_failure_preserves_endpoint_status() {
    let (status, auth_required, message, endpoint_status) = endpoint_catalog_runtime_probe_failure(
        "connection refused while launching bundled runtime".to_string(),
        HarnessEndpointVerificationStatus::Valid,
    );
    assert_eq!(status, "network_error");
    assert_eq!(auth_required, Some(false));
    assert!(message
        .as_deref()
        .is_some_and(|value| value.contains("connection refused")));
    assert_eq!(endpoint_status, HarnessEndpointVerificationStatus::Valid);
}

#[test]
fn provider_auth_mode_prefers_endpoint_for_active_endpoint_selection() {
    let endpoint = harness_sources::HarnessProviderSourceConfig {
        provider_id: "codex".to_string(),
        selected_source_kind: HarnessSourceKind::Endpoint,
        selected_endpoint_id: Some("ep-1".to_string()),
        endpoints: vec![test_endpoint("ep-1")],
    };
    assert_eq!(provider_auth_mode(true, Some(&endpoint)), "endpoint");

    let subscription = harness_sources::HarnessProviderSourceConfig {
        provider_id: "codex".to_string(),
        selected_source_kind: HarnessSourceKind::Subscription,
        selected_endpoint_id: None,
        endpoints: vec![],
    };
    assert_eq!(
        provider_auth_mode(true, Some(&subscription)),
        "subscription"
    );
    assert_eq!(provider_auth_mode(false, Some(&subscription)), "none");
}
