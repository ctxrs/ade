use super::*;

#[test]
fn claude_supports_endpoint_mode_but_cursor_does_not() {
    assert!(supports_harness_endpoint(PROVIDER_CLAUDE));
    assert!(!supports_harness_endpoint(PROVIDER_CURSOR));
    assert!(supports_harness_endpoint(PROVIDER_CODEX));
    assert!(!supports_harness_endpoint(PROVIDER_GOOSE));
    assert!(!supports_harness_endpoint(PROVIDER_OPENHANDS));
}

#[tokio::test]
async fn cursor_source_config_defaults_to_subscription_without_endpoints() {
    let root = tempfile::tempdir().expect("tempdir");
    let cfg = get_provider_source_config(root.path(), PROVIDER_CURSOR)
        .await
        .expect("config");
    assert_eq!(cfg.selected_source_kind, HarnessSourceKind::Subscription);
    assert!(cfg.selected_endpoint_id.is_none());
    assert!(cfg.endpoints.is_empty());
}

#[tokio::test]
async fn cursor_rejects_endpoint_source_selection() {
    let root = tempfile::tempdir().expect("tempdir");
    let err = set_provider_source_selection(
        root.path(),
        PROVIDER_CURSOR,
        HarnessSourceKind::Endpoint,
        Some("ep-1".to_string()),
    )
    .await
    .expect_err("cursor endpoint mode should be rejected");
    assert!(err
        .to_string()
        .contains("provider does not support harness endpoints"));
}

#[test]
fn parse_openai_models_payload_extracts_unique_ids() {
    let payload = serde_json::json!({
        "data": [
            { "id": "openai/gpt-5.2", "name": "GPT-5.2" },
            { "id": "openai/gpt-5.2" },
            { "id": "openai/gpt-4.1" },
            { "id": "" },
            {}
        ]
    });
    let models = parse_openai_models_payload(&payload).expect("models should parse");
    assert_eq!(
        models,
        vec![
            EndpointModelRecord {
                id: "openai/gpt-5.2".to_string(),
                name: Some("GPT-5.2".to_string()),
            },
            EndpointModelRecord {
                id: "openai/gpt-4.1".to_string(),
                name: None,
            },
        ]
    );
}

#[test]
fn infer_endpoint_model_provider_namespace_prefers_non_generic_host_label() {
    assert_eq!(
        infer_endpoint_model_provider_namespace("https://openrouter.ai/api/v1"),
        Some("openrouter".to_string())
    );
    assert_eq!(
        infer_endpoint_model_provider_namespace("https://api.myawesomeprovider.example/v1"),
        Some("myawesomeprovider".to_string())
    );
}

#[test]
fn normalize_namespaced_model_override_always_prefixes_namespace() {
    assert_eq!(
        normalize_namespaced_model_override("openai/gpt-5.2-codex", Some("openrouter")),
        "openrouter/openai/gpt-5.2-codex"
    );
    assert_eq!(
        normalize_namespaced_model_override("openrouter/openai/gpt-5.2-codex", Some("openrouter"),),
        "openrouter/openrouter/openai/gpt-5.2-codex"
    );
    assert_eq!(
        normalize_namespaced_model_override(
            "myawesomeprovider/openai/gpt-5.2-codex",
            Some("openrouter"),
        ),
        "openrouter/myawesomeprovider/openai/gpt-5.2-codex"
    );
}

#[test]
fn parse_openai_models_payload_requires_data_array() {
    let payload = serde_json::json!({
        "models": []
    });
    let err = parse_openai_models_payload(&payload).expect_err("missing data should error");
    assert!(err
        .to_string()
        .contains("models payload missing array field 'data'"));
}

#[test]
fn normalize_manual_model_ids_deduplicates_and_trims() {
    let input = vec![
        " openai/gpt-5.2 ".to_string(),
        "".to_string(),
        "openai/gpt-5.2".to_string(),
        "anthropic/claude-sonnet-4.5".to_string(),
    ];
    assert_eq!(
        normalize_manual_model_ids(&input),
        vec![
            "openai/gpt-5.2".to_string(),
            "anthropic/claude-sonnet-4.5".to_string(),
        ]
    );
}

#[test]
fn truncate_discovery_error_preserves_utf8_boundaries() {
    let raw = format!("error: {}", "界".repeat(400));
    let truncated = truncate_discovery_error(&raw);
    assert!(truncated.ends_with("..."));
    assert!(truncated.len() <= 283);
    assert!(std::str::from_utf8(truncated.as_bytes()).is_ok());
}

#[test]
fn endpoint_catalog_stale_logic_handles_status_and_age() {
    let now = Utc::now();
    let mut endpoint = HarnessEndpointRecord {
        id: "ep-1".to_string(),
        provider_id: PROVIDER_CODEX.to_string(),
        name: "OpenRouter".to_string(),
        base_url: Some("https://openrouter.ai/api/v1".to_string()),
        api_shape: HarnessApiShape::OpenaiResponses,
        auth_type: CODEX_AUTH_TYPE_BEARER.to_string(),
        model_override: None,
        created_at: now,
        updated_at: now,
        last_verification_status: HarnessEndpointVerificationStatus::Unknown,
        last_verification_at: None,
        last_error: None,
        has_api_key: true,
        model_catalog_status: EndpointModelCatalogStatus::Unknown,
        model_catalog_fetched_at: None,
        model_catalog_error: None,
        model_catalog_models: Vec::new(),
        manual_model_ids: Vec::new(),
        model_catalog_source: None,
    };
    assert!(endpoint_model_catalog_is_stale(&endpoint, now));

    endpoint.model_catalog_status = EndpointModelCatalogStatus::ManualOnly;
    assert!(!endpoint_model_catalog_is_stale(&endpoint, now));

    endpoint.model_catalog_status = EndpointModelCatalogStatus::Ready;
    endpoint.model_catalog_fetched_at = Some(now - chrono::Duration::hours(1));
    assert!(!endpoint_model_catalog_is_stale(&endpoint, now));

    endpoint.model_catalog_fetched_at = Some(now - chrono::Duration::hours(30));
    assert!(endpoint_model_catalog_is_stale(&endpoint, now));
}

#[tokio::test]
async fn defaults_to_subscription_without_registry() {
    let root = tempfile::tempdir().expect("tempdir");
    let cfg = get_provider_source_config(root.path(), PROVIDER_CODEX)
        .await
        .expect("config");
    assert_eq!(cfg.selected_source_kind, HarnessSourceKind::Subscription);
    assert!(cfg.selected_endpoint_id.is_none());
    assert!(cfg.endpoints.is_empty());
}

#[tokio::test]
async fn amp_subscription_sets_persistent_home_env() {
    let root = tempfile::tempdir().expect("tempdir");
    let resolved = resolve_provider_source_for_run(root.path(), PROVIDER_AMP)
        .await
        .expect("resolved");
    assert_eq!(resolved.source_kind, HarnessSourceKind::Subscription);
    let expected_home = root.path().join("providers").join("amp").join("home");
    assert_eq!(
        resolved.env.get("HOME"),
        Some(&expected_home.to_string_lossy().to_string())
    );
    assert_eq!(
        resolved.env.get("XDG_CONFIG_HOME"),
        Some(&expected_home.join(".config").to_string_lossy().to_string())
    );
    assert_eq!(
        resolved.env.get("XDG_CACHE_HOME"),
        Some(&expected_home.join(".cache").to_string_lossy().to_string())
    );
}

#[tokio::test]
async fn refresh_provider_endpoint_model_catalog_returns_error_state_for_unsupported_discovery() {
    let root = tempfile::tempdir().expect("tempdir");
    let endpoint = upsert_provider_endpoint(
        root.path(),
        PROVIDER_GEMINI,
        HarnessEndpointUpsert {
            endpoint_id: None,
            name: "Gemini Native".to_string(),
            base_url: None,
            api_shape: None,
            auth_type: Some(GEMINI_AUTH_TYPE_GEMINI_API_KEY.to_string()),
            model_override: None,
            api_key: Some("gemini-key".to_string()),
            service_account_json: None,
            project_id: None,
            location: None,
        },
    )
    .await
    .expect("upsert endpoint");

    let refreshed =
        refresh_provider_endpoint_model_catalog(root.path(), PROVIDER_GEMINI, &endpoint.id)
            .await
            .expect("refresh should not fail");

    assert_eq!(
        refreshed.model_catalog_status,
        EndpointModelCatalogStatus::Error
    );
    assert!(refreshed
        .model_catalog_error
        .as_deref()
        .unwrap_or_default()
        .contains("model discovery is unsupported"));
}

#[tokio::test]
async fn invalid_registry_json_returns_error() {
    let root = tempfile::tempdir().expect("tempdir");
    let path = registry_path(root.path());
    tokio::fs::create_dir_all(path.parent().expect("parent"))
        .await
        .expect("mkdir");
    tokio::fs::write(&path, b"{not-json")
        .await
        .expect("write invalid");

    let err = get_provider_source_config(root.path(), PROVIDER_CODEX)
        .await
        .expect_err("invalid registry should fail");
    assert!(err.to_string().contains("parsing harness source registry"));
}

#[tokio::test]
async fn codex_endpoint_requires_verify_for_run_resolution() {
    let root = tempfile::tempdir().expect("tempdir");
    let endpoint = upsert_provider_endpoint(
        root.path(),
        PROVIDER_CODEX,
        HarnessEndpointUpsert {
            endpoint_id: None,
            name: "OpenRouter".to_string(),
            base_url: Some("https://openrouter.ai/api/v1".to_string()),
            api_shape: Some(HarnessApiShape::OpenaiResponses),
            auth_type: None,
            model_override: None,
            api_key: Some("sk-test".to_string()),
            service_account_json: None,
            project_id: None,
            location: None,
        },
    )
    .await
    .expect("upsert");

    set_provider_source_selection(
        root.path(),
        PROVIDER_CODEX,
        HarnessSourceKind::Endpoint,
        Some(endpoint.id.clone()),
    )
    .await
    .expect("select");

    let err = resolve_provider_source_for_run(root.path(), PROVIDER_CODEX)
        .await
        .expect_err("expected verify gate error");
    assert!(err.to_string().contains("not verified"));

    mark_endpoint_verification(
        root.path(),
        PROVIDER_CODEX,
        &endpoint.id,
        HarnessEndpointVerificationStatus::Valid,
        None,
    )
    .await
    .expect("mark verified");

    let resolved = resolve_provider_source_for_run(root.path(), PROVIDER_CODEX)
        .await
        .expect("resolve run");
    assert_eq!(resolved.source_kind, HarnessSourceKind::Endpoint);
    assert!(resolved.env.contains_key("CODEX_HOME"));
    assert_eq!(
        resolved.env.get("OPENAI_BASE_URL"),
        Some(&"https://openrouter.ai/api/v1".to_string())
    );
}

#[tokio::test]
async fn claude_endpoint_projects_anthropic_env_and_requires_verify_for_run() {
    let root = tempfile::tempdir().expect("tempdir");
    let endpoint = upsert_provider_endpoint(
        root.path(),
        PROVIDER_CLAUDE,
        HarnessEndpointUpsert {
            endpoint_id: None,
            name: "Anthropic".to_string(),
            base_url: Some("https://api.anthropic.com/v1".to_string()),
            api_shape: Some(HarnessApiShape::AnthropicMessages),
            auth_type: None,
            model_override: None,
            api_key: Some("sk-ant-api".to_string()),
            service_account_json: None,
            project_id: None,
            location: None,
        },
    )
    .await
    .expect("upsert");

    set_provider_source_selection(
        root.path(),
        PROVIDER_CLAUDE,
        HarnessSourceKind::Endpoint,
        Some(endpoint.id.clone()),
    )
    .await
    .expect("select");

    let probe = resolve_provider_source_for_probe(root.path(), PROVIDER_CLAUDE)
        .await
        .expect("resolve probe");
    assert_eq!(
        probe.env.get("ANTHROPIC_API_KEY"),
        Some(&"sk-ant-api".to_string())
    );
    assert_eq!(
        probe.env.get("ANTHROPIC_BASE_URL"),
        Some(&"https://api.anthropic.com".to_string())
    );

    let run_err = resolve_provider_source_for_run(root.path(), PROVIDER_CLAUDE)
        .await
        .expect_err("expected verify gate error");
    assert!(run_err.to_string().contains("not verified"));

    mark_endpoint_verification(
        root.path(),
        PROVIDER_CLAUDE,
        &endpoint.id,
        HarnessEndpointVerificationStatus::Valid,
        None,
    )
    .await
    .expect("mark verified");

    let run = resolve_provider_source_for_run(root.path(), PROVIDER_CLAUDE)
        .await
        .expect("resolve run");
    assert_eq!(
        run.env.get("ANTHROPIC_BASE_URL"),
        Some(&"https://api.anthropic.com".to_string())
    );
}

#[tokio::test]
async fn claude_endpoint_openrouter_v1_base_url_is_normalized_for_anthropic_shape() {
    let root = tempfile::tempdir().expect("tempdir");
    let endpoint = upsert_provider_endpoint(
        root.path(),
        PROVIDER_CLAUDE,
        HarnessEndpointUpsert {
            endpoint_id: None,
            name: "OpenRouter".to_string(),
            base_url: Some("https://openrouter.ai/api/v1".to_string()),
            api_shape: Some(HarnessApiShape::AnthropicMessages),
            auth_type: None,
            model_override: Some("anthropic/claude-opus-4.6".to_string()),
            api_key: Some("sk-or-v1".to_string()),
            service_account_json: None,
            project_id: None,
            location: None,
        },
    )
    .await
    .expect("upsert");

    assert_eq!(
        endpoint.base_url,
        Some("https://openrouter.ai/api".to_string())
    );

    set_provider_source_selection(
        root.path(),
        PROVIDER_CLAUDE,
        HarnessSourceKind::Endpoint,
        Some(endpoint.id.clone()),
    )
    .await
    .expect("select");

    mark_endpoint_verification(
        root.path(),
        PROVIDER_CLAUDE,
        &endpoint.id,
        HarnessEndpointVerificationStatus::Valid,
        None,
    )
    .await
    .expect("mark verified");

    let resolved = resolve_provider_source_for_run(root.path(), PROVIDER_CLAUDE)
        .await
        .expect("resolve run");
    assert_eq!(
        resolved.env.get("ANTHROPIC_BASE_URL"),
        Some(&"https://openrouter.ai/api".to_string())
    );
}

#[tokio::test]
async fn gemini_endpoint_does_not_require_verify_for_run_resolution() {
    let root = tempfile::tempdir().expect("tempdir");
    let endpoint = upsert_provider_endpoint(
        root.path(),
        PROVIDER_GEMINI,
        HarnessEndpointUpsert {
            endpoint_id: None,
            name: "Gemini Key".to_string(),
            base_url: None,
            api_shape: Some(HarnessApiShape::OpenaiResponses),
            auth_type: Some(GEMINI_AUTH_TYPE_GEMINI_API_KEY.to_string()),
            model_override: None,
            api_key: Some("gemini-key".to_string()),
            service_account_json: None,
            project_id: None,
            location: None,
        },
    )
    .await
    .expect("upsert");

    set_provider_source_selection(
        root.path(),
        PROVIDER_GEMINI,
        HarnessSourceKind::Endpoint,
        Some(endpoint.id.clone()),
    )
    .await
    .expect("select");

    let resolved = resolve_provider_source_for_run(root.path(), PROVIDER_GEMINI)
        .await
        .expect("resolve run");
    assert_eq!(resolved.source_kind, HarnessSourceKind::Endpoint);
    assert_eq!(endpoint.base_url, None);
    assert_eq!(
        resolved.env.get("GEMINI_API_KEY"),
        Some(&"gemini-key".to_string())
    );
    assert_eq!(resolved.env.get("GOOGLE_API_KEY"), Some(&String::new()));
    let home = PathBuf::from(
        resolved
            .env
            .get("HOME")
            .expect("HOME should be set for gemini endpoint"),
    );
    assert!(home.starts_with(root.path()));
    assert_eq!(
        resolved.env.get("GEMINI_CLI_HOME"),
        Some(&home.to_string_lossy().to_string())
    );
    assert_eq!(
        resolved.env.get("GEMINI_FORCE_FILE_STORAGE"),
        Some(&"true".to_string())
    );
    assert!(home.join(".gemini").exists());
    let settings = tokio::fs::read_to_string(
        gemini_endpoint_home(root.path(), &endpoint.id).join(".gemini/settings.json"),
    )
    .await
    .expect("read gemini settings");
    assert!(settings.contains("\"selectedType\": \"gemini-api-key\""));
    assert!(!resolved.env.contains_key("OPENAI_API_KEY"));
    assert!(!resolved.env.contains_key("OPENAI_BASE_URL"));
}

#[tokio::test]
async fn gemini_vertex_endpoint_projects_vertex_env_for_run_resolution() {
    let root = tempfile::tempdir().expect("tempdir");
    let endpoint = upsert_provider_endpoint(
        root.path(),
        PROVIDER_GEMINI,
        HarnessEndpointUpsert {
            endpoint_id: None,
            name: "Gemini Vertex".to_string(),
            base_url: None,
            api_shape: Some(HarnessApiShape::OpenaiResponses),
            auth_type: Some(GEMINI_AUTH_TYPE_VERTEX_AI.to_string()),
            model_override: None,
            api_key: None,
            service_account_json: Some(
                r#"{"type":"service_account","project_id":"vertex-project","private_key_id":"key-id","private_key":"-----BEGIN PRIVATE KEY-----\nabc\n-----END PRIVATE KEY-----\n","client_email":"ctx-vertex@test.iam.gserviceaccount.com","client_id":"1234567890"}"#
                    .to_string(),
            ),
            project_id: None,
            location: None,
        },
    )
    .await
    .expect("upsert");

    set_provider_source_selection(
        root.path(),
        PROVIDER_GEMINI,
        HarnessSourceKind::Endpoint,
        Some(endpoint.id.clone()),
    )
    .await
    .expect("select");

    let resolved = resolve_provider_source_for_run(root.path(), PROVIDER_GEMINI)
        .await
        .expect("resolve run");
    assert_eq!(resolved.source_kind, HarnessSourceKind::Endpoint);
    assert_eq!(
        resolved.env.get("GOOGLE_APPLICATION_CREDENTIALS"),
        Some(
            &gemini_endpoint_home(root.path(), &endpoint.id)
                .join(".gemini/vertex-service-account.json")
                .to_string_lossy()
                .to_string(),
        )
    );
    assert_eq!(
        resolved.env.get("GOOGLE_CLOUD_PROJECT"),
        Some(&"vertex-project".to_string())
    );
    assert_eq!(
        resolved.env.get("GOOGLE_CLOUD_PROJECT_ID"),
        Some(&"vertex-project".to_string())
    );
    assert_eq!(
        resolved.env.get("GOOGLE_CLOUD_LOCATION"),
        Some(&"global".to_string())
    );
    assert_eq!(
        resolved.env.get("GOOGLE_GENAI_USE_VERTEXAI"),
        Some(&"true".to_string())
    );
    assert_eq!(resolved.env.get("GEMINI_API_KEY"), Some(&String::new()));
    assert_eq!(resolved.env.get("GOOGLE_API_KEY"), Some(&String::new()));
    assert!(resolved.env.contains_key("HOME"));
    assert!(resolved.env.contains_key("GEMINI_CLI_HOME"));
    let settings = tokio::fs::read_to_string(
        gemini_endpoint_home(root.path(), &endpoint.id).join(".gemini/settings.json"),
    )
    .await
    .expect("read gemini settings");
    assert!(settings.contains("\"selectedType\": \"vertex-ai\""));
    let credentials = tokio::fs::read_to_string(
        gemini_endpoint_home(root.path(), &endpoint.id).join(".gemini/vertex-service-account.json"),
    )
    .await
    .expect("read vertex credentials");
    assert!(credentials.contains("\"project_id\":\"vertex-project\""));
    assert!(!resolved.env.contains_key("OPENAI_API_KEY"));
    assert!(!resolved.env.contains_key("OPENAI_BASE_URL"));
}

#[tokio::test]
async fn gemini_endpoint_rejects_custom_base_url() {
    let root = tempfile::tempdir().expect("tempdir");
    let err = upsert_provider_endpoint(
        root.path(),
        PROVIDER_GEMINI,
        HarnessEndpointUpsert {
            endpoint_id: None,
            name: "Gemini OpenAI".to_string(),
            base_url: Some("https://openrouter.ai/api/v1".to_string()),
            api_shape: Some(HarnessApiShape::OpenaiResponses),
            auth_type: Some(GEMINI_AUTH_TYPE_GEMINI_API_KEY.to_string()),
            model_override: Some("openai/gpt-5.2".to_string()),
            api_key: Some("openrouter-key".to_string()),
            service_account_json: None,
            project_id: None,
            location: None,
        },
    )
    .await
    .expect_err("upsert should fail");
    assert!(err
        .to_string()
        .contains("does not support custom endpoint base_url"));
}

#[tokio::test]
async fn gemini_endpoint_rejects_bearer_auth_type() {
    let root = tempfile::tempdir().expect("tempdir");
    let err = upsert_provider_endpoint(
        root.path(),
        PROVIDER_GEMINI,
        HarnessEndpointUpsert {
            endpoint_id: None,
            name: "Gemini Bearer".to_string(),
            base_url: None,
            api_shape: Some(HarnessApiShape::OpenaiResponses),
            auth_type: Some(CODEX_AUTH_TYPE_BEARER.to_string()),
            model_override: None,
            api_key: Some("key".to_string()),
            service_account_json: None,
            project_id: None,
            location: None,
        },
    )
    .await
    .expect_err("upsert should fail");
    assert!(err
        .to_string()
        .contains("auth_type 'bearer' is not supported"));
}

#[tokio::test]
async fn gemini_endpoint_rejects_unknown_auth_type() {
    let root = tempfile::tempdir().expect("tempdir");
    let err = upsert_provider_endpoint(
        root.path(),
        PROVIDER_GEMINI,
        HarnessEndpointUpsert {
            endpoint_id: None,
            name: "Gemini Invalid".to_string(),
            base_url: None,
            api_shape: Some(HarnessApiShape::OpenaiResponses),
            auth_type: Some("invalid".to_string()),
            model_override: None,
            api_key: Some("gemini-key".to_string()),
            service_account_json: None,
            project_id: None,
            location: None,
        },
    )
    .await
    .expect_err("upsert should fail");
    assert!(err
        .to_string()
        .contains("auth_type 'invalid' is not supported"));
}

#[tokio::test]
async fn kimi_endpoint_projects_env_for_run_resolution() {
    let root = tempfile::tempdir().expect("tempdir");
    let endpoint = upsert_provider_endpoint(
        root.path(),
        PROVIDER_KIMI,
        HarnessEndpointUpsert {
            endpoint_id: None,
            name: "Kimi Key".to_string(),
            base_url: Some("https://api.moonshot.ai/v1".to_string()),
            api_shape: Some(HarnessApiShape::OpenaiResponses),
            auth_type: None,
            model_override: Some("kimi-k2".to_string()),
            api_key: Some("kimi-key".to_string()),
            service_account_json: None,
            project_id: None,
            location: None,
        },
    )
    .await
    .expect("upsert");

    set_provider_source_selection(
        root.path(),
        PROVIDER_KIMI,
        HarnessSourceKind::Endpoint,
        Some(endpoint.id.clone()),
    )
    .await
    .expect("select");

    let resolved = resolve_provider_source_for_run(root.path(), PROVIDER_KIMI)
        .await
        .expect("resolve run");
    assert_eq!(resolved.source_kind, HarnessSourceKind::Endpoint);
    assert_eq!(
        resolved.env.get("KIMI_BASE_URL").map(String::as_str),
        Some("https://api.moonshot.ai/v1")
    );
    assert_eq!(
        resolved.env.get("OPENAI_BASE_URL").map(String::as_str),
        Some("https://api.moonshot.ai/v1")
    );
    assert_eq!(
        resolved.env.get("KIMI_API_KEY").map(String::as_str),
        Some("kimi-key")
    );
    assert_eq!(
        resolved.env.get("OPENAI_API_KEY").map(String::as_str),
        Some("kimi-key")
    );
    assert_eq!(
        resolved.env.get("KIMI_MODEL_NAME").map(String::as_str),
        Some("kimi-k2")
    );
    assert_eq!(
        resolved.env.get("OPENAI_MODEL").map(String::as_str),
        Some("kimi-k2")
    );
    assert_eq!(
        resolved
            .env
            .get("CTX_CRP_DISABLE_MODEL_OVERRIDE")
            .map(String::as_str),
        Some("1")
    );
    let kimi_share_dir = resolved
        .env
        .get("KIMI_SHARE_DIR")
        .expect("KIMI_SHARE_DIR should be set");
    let kimi_share_path = PathBuf::from(kimi_share_dir);
    assert!(kimi_share_path.starts_with(root.path()));
    let token_path = kimi_share_path.join("credentials").join("kimi-code.json");
    let token = tokio::fs::read_to_string(&token_path)
        .await
        .expect("read seeded kimi endpoint token");
    let parsed: serde_json::Value = serde_json::from_str(&token).expect("parse kimi token json");
    assert_eq!(
        parsed
            .get("access_token")
            .and_then(serde_json::Value::as_str),
        Some("ctx-endpoint-access-token")
    );
}

#[tokio::test]
async fn qwen_model_override_is_opaque_and_opencode_uses_endpoint_namespace() {
    let root = tempfile::tempdir().expect("tempdir");
    let base_url = "https://api.myawesomeprovider.example/v1";
    let model_override = "openai/gpt-5.2-codex";

    let qwen_endpoint = upsert_provider_endpoint(
        root.path(),
        PROVIDER_QWEN,
        HarnessEndpointUpsert {
            endpoint_id: None,
            name: "Qwen custom".to_string(),
            base_url: Some(base_url.to_string()),
            api_shape: Some(HarnessApiShape::OpenaiResponses),
            auth_type: None,
            model_override: Some(model_override.to_string()),
            api_key: Some("qwen-key".to_string()),
            service_account_json: None,
            project_id: None,
            location: None,
        },
    )
    .await
    .expect("upsert qwen endpoint");
    set_provider_source_selection(
        root.path(),
        PROVIDER_QWEN,
        HarnessSourceKind::Endpoint,
        Some(qwen_endpoint.id.clone()),
    )
    .await
    .expect("select qwen endpoint");
    let qwen_resolved = resolve_provider_source_for_run(root.path(), PROVIDER_QWEN)
        .await
        .expect("resolve qwen");
    assert_eq!(
        qwen_resolved.env.get("OPENAI_MODEL"),
        Some(&"openai/gpt-5.2-codex".to_string())
    );
    let qwen_home = PathBuf::from(
        qwen_resolved
            .env
            .get("HOME")
            .expect("HOME should be set for qwen endpoint"),
    );
    let qwen_settings = tokio::fs::read_to_string(qwen_home.join(".qwen").join("settings.json"))
        .await
        .expect("read qwen settings");
    assert!(qwen_settings.contains("\"selectedType\": \"openai\""));

    let opencode_endpoint = upsert_provider_endpoint(
        root.path(),
        PROVIDER_OPENCODE,
        HarnessEndpointUpsert {
            endpoint_id: None,
            name: "OpenCode custom".to_string(),
            base_url: Some(base_url.to_string()),
            api_shape: Some(HarnessApiShape::OpenaiResponses),
            auth_type: None,
            model_override: Some(model_override.to_string()),
            api_key: Some("opencode-key".to_string()),
            service_account_json: None,
            project_id: None,
            location: None,
        },
    )
    .await
    .expect("upsert opencode endpoint");
    set_provider_source_selection(
        root.path(),
        PROVIDER_OPENCODE,
        HarnessSourceKind::Endpoint,
        Some(opencode_endpoint.id.clone()),
    )
    .await
    .expect("select opencode endpoint");
    let opencode_resolved = resolve_provider_source_for_run(root.path(), PROVIDER_OPENCODE)
        .await
        .expect("resolve opencode");
    let config = opencode_resolved
        .env
        .get("OPENCODE_CONFIG_CONTENT")
        .expect("opencode config env");
    let parsed: serde_json::Value = serde_json::from_str(config).expect("valid json");
    assert_eq!(
        parsed.get("model").and_then(serde_json::Value::as_str),
        Some("myawesomeprovider/openai/gpt-5.2-codex")
    );
    assert_eq!(
        parsed
            .get("permission")
            .and_then(|permission| permission.get("edit"))
            .and_then(serde_json::Value::as_str),
        Some("deny")
    );
    assert_eq!(
        parsed
            .get("permission")
            .and_then(|permission| permission.get("bash"))
            .and_then(serde_json::Value::as_str),
        Some("allow")
    );
    assert!(
        parsed
            .get("provider")
            .and_then(|provider| provider.get("myawesomeprovider"))
            .is_some(),
        "expected namespaced provider config key"
    );
    assert!(
        !opencode_resolved.env.contains_key("OPENROUTER_API_KEY"),
        "custom namespace should not force OPENROUTER_* compatibility env vars"
    );
}

#[tokio::test]
async fn additional_provider_endpoint_env_projection_smoke() {
    let root = tempfile::tempdir().expect("tempdir");
    let cases: &[(&str, &[&str])] = &[
        (
            PROVIDER_QWEN,
            &["OPENAI_API_KEY", "OPENAI_BASE_URL", "HOME"],
        ),
        (
            PROVIDER_OPENCODE,
            &[
                "OPENAI_API_KEY",
                "OPENROUTER_API_KEY",
                "OPENCODE_CONFIG_CONTENT",
            ],
        ),
        (PROVIDER_MISTRAL, &["MISTRAL_API_KEY", "MISTRAL_BASE_URL"]),
        (
            PROVIDER_DROID,
            &[
                "HOME",
                "OPENAI_API_KEY",
                "OPENAI_BASE_URL",
                "DROID_DEFAULT_MODEL",
            ],
        ),
        (PROVIDER_COPILOT, &["GH_TOKEN", "GITHUB_TOKEN"]),
        (
            PROVIDER_PI,
            &["OPENAI_API_KEY", "PI_ACP_PROVIDER", "PI_ACP_MODEL"],
        ),
    ];

    for (provider_id, required_keys) in cases {
        let endpoint = upsert_provider_endpoint(
            root.path(),
            provider_id,
            HarnessEndpointUpsert {
                endpoint_id: None,
                name: format!("{provider_id} endpoint"),
                base_url: if *provider_id == PROVIDER_COPILOT || *provider_id == PROVIDER_PI {
                    None
                } else {
                    Some("https://openrouter.ai/api/v1".to_string())
                },
                api_shape: if *provider_id == PROVIDER_COPILOT || *provider_id == PROVIDER_PI {
                    None
                } else {
                    Some(HarnessApiShape::OpenaiResponses)
                },
                auth_type: None,
                model_override: Some("test-model".to_string()),
                api_key: Some("test-key".to_string()),
                service_account_json: None,
                project_id: None,
                location: None,
            },
        )
        .await
        .expect("upsert endpoint");

        set_provider_source_selection(
            root.path(),
            provider_id,
            HarnessSourceKind::Endpoint,
            Some(endpoint.id.clone()),
        )
        .await
        .expect("select endpoint");

        let resolved = resolve_provider_source_for_run(root.path(), provider_id)
            .await
            .expect("resolve run");
        for key in *required_keys {
            assert!(
                resolved.env.contains_key(*key),
                "{provider_id} missing env key {key}"
            );
        }
    }
}

#[tokio::test]
async fn droid_endpoint_writes_factory_settings_for_generic_endpoint() {
    let root = tempfile::tempdir().expect("tempdir");
    let endpoint = upsert_provider_endpoint(
        root.path(),
        PROVIDER_DROID,
        HarnessEndpointUpsert {
            endpoint_id: None,
            name: "Droid endpoint".to_string(),
            base_url: Some("https://openrouter.ai/api/v1".to_string()),
            api_shape: Some(HarnessApiShape::OpenaiResponses),
            auth_type: None,
            model_override: Some("openai/gpt-5.2-codex".to_string()),
            api_key: Some("sk-or-test".to_string()),
            service_account_json: None,
            project_id: None,
            location: None,
        },
    )
    .await
    .expect("upsert endpoint");
    set_provider_source_selection(
        root.path(),
        PROVIDER_DROID,
        HarnessSourceKind::Endpoint,
        Some(endpoint.id.clone()),
    )
    .await
    .expect("select endpoint");

    let resolved = resolve_provider_source_for_run(root.path(), PROVIDER_DROID)
        .await
        .expect("resolve run");
    let home = PathBuf::from(
        resolved
            .env
            .get("HOME")
            .expect("HOME should be set for droid endpoint"),
    );
    let settings = tokio::fs::read_to_string(home.join(".factory").join("settings.json"))
        .await
        .expect("read droid settings");
    assert!(settings.contains("\"provider\": \"generic-chat-completion-api\""));
    assert!(settings.contains("\"baseUrl\": \"https://openrouter.ai/api/v1\""));
    assert!(settings.contains("\"model\": \"openai/gpt-5.2-codex\""));
    assert!(settings.contains("\"displayName\": \"openai/gpt-5.2-codex [openrouter]\""));
    assert!(!resolved.env.contains_key("FACTORY_API_KEY"));
    assert_eq!(
        resolved.env.get("DROID_DEFAULT_MODEL"),
        Some(&"custom:openai/gpt-5.2-codex-[openrouter]-0".to_string())
    );
}

#[tokio::test]
async fn droid_endpoint_requires_explicit_model() {
    let root = tempfile::tempdir().expect("tempdir");
    let endpoint = upsert_provider_endpoint(
        root.path(),
        PROVIDER_DROID,
        HarnessEndpointUpsert {
            endpoint_id: None,
            name: "Droid endpoint".to_string(),
            base_url: Some("https://openrouter.ai/api/v1".to_string()),
            api_shape: Some(HarnessApiShape::OpenaiResponses),
            auth_type: None,
            model_override: None,
            api_key: Some("sk-or-test".to_string()),
            service_account_json: None,
            project_id: None,
            location: None,
        },
    )
    .await
    .expect("upsert endpoint");
    set_provider_source_selection(
        root.path(),
        PROVIDER_DROID,
        HarnessSourceKind::Endpoint,
        Some(endpoint.id.clone()),
    )
    .await
    .expect("select endpoint");

    let err = resolve_provider_source_for_run(root.path(), PROVIDER_DROID)
        .await
        .expect_err("droid endpoint should require explicit model");
    assert!(err.to_string().contains("missing a concrete model id"));
}

#[tokio::test]
async fn seed_droid_auth_from_host_path_copies_auth_encrypted_into_endpoint_home() {
    let root = tempfile::tempdir().expect("tempdir");
    let host = tempfile::tempdir().expect("tempdir");
    let host_auth_path = host.path().join("auth.encrypted");
    tokio::fs::write(&host_auth_path, b"seeded-droid-auth")
        .await
        .expect("write host auth");

    let endpoint_home = droid_endpoint_home(root.path(), "ep-1");
    let changed = seed_droid_auth_from_host_path(&endpoint_home, &host_auth_path)
        .await
        .expect("seed auth");

    assert!(changed);
    let copied = tokio::fs::read(endpoint_home.join(".factory").join("auth.encrypted"))
        .await
        .expect("read copied auth");
    assert_eq!(copied, b"seeded-droid-auth");
}

#[tokio::test]
async fn droid_endpoint_requires_base_url() {
    let root = tempfile::tempdir().expect("tempdir");
    let err = upsert_provider_endpoint(
        root.path(),
        PROVIDER_DROID,
        HarnessEndpointUpsert {
            endpoint_id: None,
            name: "droid endpoint".to_string(),
            base_url: None,
            api_shape: Some(HarnessApiShape::OpenaiResponses),
            auth_type: None,
            model_override: Some("openai/gpt-5.2-codex".to_string()),
            api_key: Some("sk-test".to_string()),
            service_account_json: None,
            project_id: None,
            location: None,
        },
    )
    .await
    .expect_err("base_url should be required");
    assert!(
        err.to_string().contains("base_url is required"),
        "droid should reject missing base_url: {err}"
    );
}

#[tokio::test]
async fn copilot_endpoint_allows_token_only_upsert() {
    let root = tempfile::tempdir().expect("tempdir");
    let endpoint = upsert_provider_endpoint(
        root.path(),
        PROVIDER_COPILOT,
        HarnessEndpointUpsert {
            endpoint_id: None,
            name: "Copilot token".to_string(),
            base_url: None,
            api_shape: None,
            auth_type: None,
            model_override: None,
            api_key: Some("ghp_test".to_string()),
            service_account_json: None,
            project_id: None,
            location: None,
        },
    )
    .await
    .expect("upsert endpoint");

    assert!(endpoint.base_url.is_none());
    assert_eq!(endpoint.api_shape, HarnessApiShape::OpenaiResponses);

    let cfg = get_provider_source_config(root.path(), PROVIDER_COPILOT)
        .await
        .expect("get source config");
    let stored = cfg
        .endpoints
        .iter()
        .find(|candidate| candidate.id == endpoint.id)
        .expect("stored endpoint");
    assert!(stored.base_url.is_none());
    assert_eq!(stored.api_shape, HarnessApiShape::OpenaiResponses);
}

#[tokio::test]
async fn pi_endpoint_allows_token_only_upsert_and_optional_base_url() {
    let root = tempfile::tempdir().expect("tempdir");
    let endpoint = upsert_provider_endpoint(
        root.path(),
        PROVIDER_PI,
        HarnessEndpointUpsert {
            endpoint_id: None,
            name: "Pi token".to_string(),
            base_url: None,
            api_shape: None,
            auth_type: None,
            model_override: Some("gpt-5".to_string()),
            api_key: Some("pi-key".to_string()),
            service_account_json: None,
            project_id: None,
            location: None,
        },
    )
    .await
    .expect("upsert endpoint");

    assert!(endpoint.base_url.is_none());
    assert_eq!(endpoint.api_shape, HarnessApiShape::OpenaiResponses);

    set_provider_source_selection(
        root.path(),
        PROVIDER_PI,
        HarnessSourceKind::Endpoint,
        Some(endpoint.id.clone()),
    )
    .await
    .expect("select endpoint");

    let resolved = resolve_provider_source_for_run(root.path(), PROVIDER_PI)
        .await
        .expect("resolve run");
    assert_eq!(
        resolved.env.get("OPENAI_API_KEY"),
        Some(&"pi-key".to_string())
    );
    assert_eq!(
        resolved.env.get("PI_ACP_PROVIDER"),
        Some(&"openai".to_string())
    );
    assert_eq!(resolved.env.get("PI_ACP_MODEL"), Some(&"gpt-5".to_string()));
    assert!(!resolved.env.contains_key("OPENAI_BASE_URL"));
}

#[tokio::test]
async fn deleting_codex_endpoint_removes_endpoint_home() {
    let root = tempfile::tempdir().expect("tempdir");
    let endpoint = upsert_provider_endpoint(
        root.path(),
        PROVIDER_CODEX,
        HarnessEndpointUpsert {
            endpoint_id: None,
            name: "OpenRouter".to_string(),
            base_url: Some("https://openrouter.ai/api/v1".to_string()),
            api_shape: Some(HarnessApiShape::OpenaiResponses),
            auth_type: None,
            model_override: None,
            api_key: Some("sk-test".to_string()),
            service_account_json: None,
            project_id: None,
            location: None,
        },
    )
    .await
    .expect("upsert");

    set_provider_source_selection(
        root.path(),
        PROVIDER_CODEX,
        HarnessSourceKind::Endpoint,
        Some(endpoint.id.clone()),
    )
    .await
    .expect("select");

    resolve_provider_source_for_probe(root.path(), PROVIDER_CODEX)
        .await
        .expect("resolve probe");

    let endpoint_home = codex_endpoint_home(root.path(), &endpoint.id);
    assert!(endpoint_home.join("auth.json").exists());

    delete_provider_endpoint(root.path(), PROVIDER_CODEX, &endpoint.id)
        .await
        .expect("delete endpoint");

    assert!(!endpoint_home.exists());
}

#[tokio::test]
async fn pi_endpoint_uses_openrouter_provider_for_openrouter_base_urls() {
    let root = tempfile::tempdir().expect("tempdir");
    let endpoint = upsert_provider_endpoint(
        root.path(),
        PROVIDER_PI,
        HarnessEndpointUpsert {
            endpoint_id: None,
            name: "Pi OpenRouter".to_string(),
            base_url: Some("https://openrouter.ai/api/v1".to_string()),
            api_shape: Some(HarnessApiShape::OpenaiResponses),
            auth_type: None,
            model_override: Some("google/gemini-3-flash-preview".to_string()),
            api_key: Some("pi-openrouter-key".to_string()),
            service_account_json: None,
            project_id: None,
            location: None,
        },
    )
    .await
    .expect("upsert endpoint");

    set_provider_source_selection(
        root.path(),
        PROVIDER_PI,
        HarnessSourceKind::Endpoint,
        Some(endpoint.id.clone()),
    )
    .await
    .expect("select endpoint");

    let resolved = resolve_provider_source_for_run(root.path(), PROVIDER_PI)
        .await
        .expect("resolve run");
    assert_eq!(
        resolved.env.get("PI_ACP_PROVIDER"),
        Some(&"openrouter".to_string())
    );
    assert_eq!(
        resolved.env.get("OPENAI_BASE_URL"),
        Some(&"https://openrouter.ai/api/v1".to_string())
    );
    assert_eq!(
        resolved.env.get("PI_ACP_MODEL"),
        Some(&"google/gemini-3-flash-preview".to_string())
    );
}

#[tokio::test]
async fn deleting_qwen_endpoint_removes_endpoint_home() {
    let root = tempfile::tempdir().expect("tempdir");
    let endpoint = upsert_provider_endpoint(
        root.path(),
        PROVIDER_QWEN,
        HarnessEndpointUpsert {
            endpoint_id: None,
            name: "Qwen endpoint".to_string(),
            base_url: Some("https://openrouter.ai/api/v1".to_string()),
            api_shape: Some(HarnessApiShape::OpenaiResponses),
            auth_type: None,
            model_override: Some("openai/gpt-5.2-codex".to_string()),
            api_key: Some("sk-test".to_string()),
            service_account_json: None,
            project_id: None,
            location: None,
        },
    )
    .await
    .expect("upsert");

    set_provider_source_selection(
        root.path(),
        PROVIDER_QWEN,
        HarnessSourceKind::Endpoint,
        Some(endpoint.id.clone()),
    )
    .await
    .expect("select");

    resolve_provider_source_for_probe(root.path(), PROVIDER_QWEN)
        .await
        .expect("resolve probe");

    let endpoint_home = qwen_endpoint_home(root.path(), &endpoint.id);
    assert!(endpoint_home.join(".qwen").join("settings.json").exists());

    delete_provider_endpoint(root.path(), PROVIDER_QWEN, &endpoint.id)
        .await
        .expect("delete endpoint");

    assert!(!endpoint_home.exists());
}

#[tokio::test]
async fn deleting_droid_endpoint_removes_endpoint_home() {
    let root = tempfile::tempdir().expect("tempdir");
    let endpoint = upsert_provider_endpoint(
        root.path(),
        PROVIDER_DROID,
        HarnessEndpointUpsert {
            endpoint_id: None,
            name: "Droid endpoint".to_string(),
            base_url: Some("https://openrouter.ai/api/v1".to_string()),
            api_shape: Some(HarnessApiShape::OpenaiResponses),
            auth_type: None,
            model_override: Some("openai/gpt-5.2-codex".to_string()),
            api_key: Some("sk-test".to_string()),
            service_account_json: None,
            project_id: None,
            location: None,
        },
    )
    .await
    .expect("upsert");

    set_provider_source_selection(
        root.path(),
        PROVIDER_DROID,
        HarnessSourceKind::Endpoint,
        Some(endpoint.id.clone()),
    )
    .await
    .expect("select");

    resolve_provider_source_for_probe(root.path(), PROVIDER_DROID)
        .await
        .expect("resolve probe");

    let endpoint_home = droid_endpoint_home(root.path(), &endpoint.id);
    assert!(endpoint_home
        .join(".factory")
        .join("settings.json")
        .exists());

    delete_provider_endpoint(root.path(), PROVIDER_DROID, &endpoint.id)
        .await
        .expect("delete endpoint");

    assert!(!endpoint_home.exists());
}

#[tokio::test]
async fn shape_compatibility_rejects_mismatch() {
    let root = tempfile::tempdir().expect("tempdir");
    let err = upsert_provider_endpoint(
        root.path(),
        PROVIDER_CODEX,
        HarnessEndpointUpsert {
            endpoint_id: None,
            name: "wrong".to_string(),
            base_url: Some("https://example.com".to_string()),
            api_shape: Some(HarnessApiShape::AnthropicMessages),
            auth_type: None,
            model_override: None,
            api_key: Some("k".to_string()),
            service_account_json: None,
            project_id: None,
            location: None,
        },
    )
    .await
    .expect_err("expected shape mismatch");
    assert!(err
        .to_string()
        .contains("codex requires api_shape=openai_responses"));
}

#[tokio::test]
async fn unsafe_endpoint_id_is_rejected() {
    let root = tempfile::tempdir().expect("tempdir");
    let err = upsert_provider_endpoint(
        root.path(),
        PROVIDER_CODEX,
        HarnessEndpointUpsert {
            endpoint_id: Some("../escape".to_string()),
            name: "bad".to_string(),
            base_url: Some("https://openrouter.ai/api/v1".to_string()),
            api_shape: Some(HarnessApiShape::OpenaiResponses),
            auth_type: None,
            model_override: None,
            api_key: Some("k".to_string()),
            service_account_json: None,
            project_id: None,
            location: None,
        },
    )
    .await
    .expect_err("unsafe endpoint id should fail");
    assert!(err
        .to_string()
        .contains("endpoint_id may only contain ASCII letters"));
}
