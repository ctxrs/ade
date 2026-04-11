use super::status::apply_target_aware_provider_status;
use super::*;
use crate::api::provider_launch::{
    endpoint_catalog_runtime_probe_failure, endpoint_catalog_verify_outcome,
    endpoint_models_payload, endpoint_supports_model_catalog_verify, get_install_statuses,
    selected_endpoint_from_harness_config, selected_endpoint_record_from_harness_config,
    GetInstallStatusesReq,
};
use crate::provider_launch::install::should_skip_install_for_healthy_provider;
use chrono::Utc;
use ctx_provider_install::install_state::{InstallEventLevel, InstallProgressEvent};
use ctx_providers::adapters::{
    ProviderAdapter, ProviderHealth, ProviderProcessInfo, ProviderRestartMode, ProviderStatus,
    RunHandle, TurnInput,
};
use ctx_store::StoreManager;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn test_endpoint(id: &str) -> harness_sources::HarnessEndpointRecord {
    harness_sources::HarnessEndpointRecord {
        id: id.to_string(),
        provider_id: "codex".to_string(),
        name: "Test endpoint".to_string(),
        base_url: Some("https://api.openai.com/v1".to_string()),
        api_shape: HarnessApiShape::OpenaiResponses,
        auth_type: "bearer".to_string(),
        model_override: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        last_verification_status: harness_sources::HarnessEndpointVerificationStatus::Unknown,
        last_verification_at: None,
        last_error: None,
        has_api_key: true,
        model_catalog_status: harness_sources::EndpointModelCatalogStatus::Unknown,
        model_catalog_fetched_at: None,
        model_catalog_error: None,
        model_catalog_models: Vec::new(),
        manual_model_ids: Vec::new(),
        model_catalog_source: None,
    }
}

#[test]
fn expected_callback_extracts_loopback_redirect() {
    let auth_url = "https://chat.openai.com/oauth/authorize?redirect_uri=http%3A%2F%2Flocalhost%3A6543%2Fauth%2Fcallback";
    let expected = expected_callback_from_auth_url(auth_url);
    assert_eq!(
        expected.as_deref(),
        Some("http://localhost:6543/auth/callback")
    );
}

#[test]
fn callback_validation_rejects_non_loopback_host() {
    let err = validate_callback_url(
        "http://example.com:1234/auth/callback?code=abc",
        Some("http://localhost:1234/auth/callback"),
    )
    .unwrap_err();
    assert!(err.to_string().contains("loopback"));
}

#[test]
fn callback_validation_accepts_expected_port_path_and_query() {
    validate_callback_url(
        "http://127.0.0.1:4321/auth/callback?code=abc&state=def",
        Some("http://localhost:4321/auth/callback"),
    )
    .expect("callback URL should validate");
}

#[test]
fn extract_auth_url_detects_urls_in_line() {
    let line = "Open this URL to continue: https://claude.ai/oauth/authorize?foo=bar";
    assert_eq!(
        extract_auth_url(line).as_deref(),
        Some("https://claude.ai/oauth/authorize?foo=bar")
    );
}

#[test]
fn extract_auth_url_detects_urls_in_browser_open_marker_line() {
    let line = "CTX_CLAUDE_AUTH_URL:https://claude.ai/oauth/authorize?redirect_uri=http%3A%2F%2Flocalhost%3A64111%2Fcallback&state=abc";
    assert_eq!(
        extract_auth_url(line).as_deref(),
        Some(
            "https://claude.ai/oauth/authorize?redirect_uri=http%3A%2F%2Flocalhost%3A64111%2Fcallback&state=abc"
        )
    );
}

#[test]
fn extract_auth_url_reconstructs_wrapped_url_lines() {
    let wrapped =
        "Open this URL: https://claude.ai/oauth/authorize?redirect_uri=http%3A%2F%2Flocalhost%3A\n64111%2Fauth%2Fcallback&state=abc";
    assert_eq!(
        extract_auth_url(wrapped).as_deref(),
        Some(
            "https://claude.ai/oauth/authorize?redirect_uri=http%3A%2F%2Flocalhost%3A64111%2Fauth%2Fcallback&state=abc"
        )
    );
}

#[test]
fn extract_auth_url_stops_before_duplicate_full_url() {
    let url = "https://claude.ai/oauth/authorize?code=true&client_id=test&response_type=code&redirect_uri=https%3A%2F%2Fplatform.claude.com%2Foauth%2Fcode%2Fcallback&scope=user%3Ainference&state=abc";
    let duplicated = format!("{url} {url}");
    assert_eq!(extract_auth_url(&duplicated).as_deref(), Some(url));
}

#[test]
fn normalize_claude_login_line_handles_osc_hyperlink_plus_visible_duplicate_url() {
    let url = "https://claude.ai/oauth/authorize?code=true&client_id=test&response_type=code&redirect_uri=https%3A%2F%2Fplatform.claude.com%2Foauth%2Fcode%2Fcallback&scope=user%3Ainference&state=abc";
    let raw = format!("\u{1b}]8;;{url}\u{7}{url}\u{1b}]8;;\u{7}\r");
    let normalized = normalize_claude_login_line(&raw);
    assert_eq!(extract_auth_url(&normalized).as_deref(), Some(url));
}

#[test]
fn extract_auth_url_reconstructs_wrapped_scheme_prefix() {
    let wrapped =
        "Open this URL: ht\ntps://claude.ai/oauth/authorize?redirect_uri=http%3A%2F%2Flocalhost%3A64111%2Fauth%2Fcallback&state=abc";
    assert_eq!(
        extract_auth_url(wrapped).as_deref(),
        Some(
            "https://claude.ai/oauth/authorize?redirect_uri=http%3A%2F%2Flocalhost%3A64111%2Fauth%2Fcallback&state=abc"
        )
    );
}

#[test]
fn extract_auth_url_from_value_detects_embedded_url_in_message() {
    let payload = serde_json::json!({
        "message": "Visit this link to sign in: https://accounts.google.com/o/oauth2/auth?foo=bar"
    });
    assert_eq!(
        extract_auth_url_from_value(&payload).as_deref(),
        Some("https://accounts.google.com/o/oauth2/auth?foo=bar")
    );
}

#[test]
fn normalize_claude_login_line_strips_ansi_sequences() {
    let raw = "\u{1b}[90mOpen URL:\u{1b}[0m https://claude.ai/oauth/authorize?foo=bar\r";
    let normalized = normalize_claude_login_line(raw);
    assert_eq!(
        extract_auth_url(&normalized).as_deref(),
        Some("https://claude.ai/oauth/authorize?foo=bar")
    );
}

#[test]
fn normalize_claude_login_line_extracts_url_from_osc8_sequence() {
    let raw = "\u{1b}]8;;https://claude.ai/oauth/authorize?foo=bar\u{7}Sign in\u{1b}]8;;\u{7}\r";
    let normalized = normalize_claude_login_line(raw);
    assert_eq!(
        extract_auth_url(&normalized).as_deref(),
        Some("https://claude.ai/oauth/authorize?foo=bar")
    );
}

#[test]
fn auth_url_looks_complete_rejects_non_loopback_redirect_callback() {
    let url = "https://claude.ai/oauth/authorize?redirect_uri=https%3A%2F%2Fplatform.claude.com%2Foauth%2Fcode%2Fcallback";
    assert!(!auth_url_looks_complete(url));
}

#[test]
fn auth_url_looks_complete_requires_port_for_loopback_callback() {
    let url = "https://claude.ai/oauth/authorize?redirect_uri=http%3A%2F%2Flocalhost%2Fcallback";
    assert!(!auth_url_looks_complete(url));
    let with_port =
        "https://claude.ai/oauth/authorize?redirect_uri=http%3A%2F%2Flocalhost%3A5999%2Fcallback";
    assert!(auth_url_looks_complete(with_port));
}

#[tokio::test]
async fn read_trailing_claude_login_lines_waits_for_late_arrival() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(40));
        let _ = tx.send("sk-ant-oat01-late-token".to_string());
    });
    let lines = read_trailing_claude_login_lines(&mut rx, Duration::from_secs(2)).await;
    assert_eq!(lines, vec!["sk-ant-oat01-late-token".to_string()]);
}

#[tokio::test]
async fn resolve_claude_login_runtime_requires_managed_or_configured_command() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data_root = temp.path().to_path_buf();

    let err = resolve_claude_login_runtime_from_config(&data_root)
        .await
        .expect_err("missing managed/configured claude login command should fail");
    assert!(err
        .to_string()
        .contains("runtime_command_missing: provider=claude-cli"));
    assert!(err
        .to_string()
        .contains("host PATH lookup is not supported"));
}

#[tokio::test]
async fn resolve_claude_login_runtime_uses_configured_runtime_command() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data_root = temp.path().to_path_buf();
    let runtime_path = data_root.join("claude-cli-mock.sh");
    std::fs::write(&runtime_path, "#!/bin/sh\nexit 0\n").expect("write runtime");
    let runtime_path_str = runtime_path.to_string_lossy().to_string();
    let mut cfg = installer::load_agent_server_config(&data_root)
        .await
        .expect("load config for runtime resolution test");
    cfg.providers.insert(
        "claude-cli".to_string(),
        installer::AgentServerCommand {
            command: runtime_path_str,
            args: vec!["--shim".to_string()],
            dependencies: vec!["dep-node".to_string()],
            managed: None,
        },
    );
    installer::save_agent_server_config(&data_root, &cfg)
        .await
        .expect("save config for runtime resolution test");

    let resolved = resolve_claude_login_runtime_from_config(&data_root)
        .await
        .expect("resolve runtime from config");
    assert!(resolved.command_abs_path.contains("claude-cli-mock.sh"));
    assert_eq!(resolved.args, vec!["--shim".to_string()]);
    assert_eq!(resolved.dependencies, vec!["dep-node".to_string()]);
    assert_eq!(
        resolved.source,
        installer::ProviderRuntimeCommandSource::UserOverride
    );
}

#[tokio::test]
async fn resolve_claude_login_runtime_prefers_configured_runtime_command_when_login_command_missing(
) {
    let temp = tempfile::tempdir().expect("tempdir");
    let data_root = temp.path().to_path_buf();
    let runtime_path = data_root.join("claude-cli-runtime-mock.sh");
    std::fs::write(&runtime_path, "#!/bin/sh\nexit 0\n").expect("write runtime");
    let runtime_path_str = runtime_path.to_string_lossy().to_string();
    let mut cfg = installer::load_agent_server_config(&data_root)
        .await
        .expect("load config for runtime resolution test");
    cfg.providers.insert(
        "claude-cli".to_string(),
        installer::AgentServerCommand {
            command: runtime_path_str,
            args: vec!["cli.js".to_string()],
            dependencies: Vec::new(),
            managed: None,
        },
    );
    installer::save_agent_server_config(&data_root, &cfg)
        .await
        .expect("save config for runtime resolution test");

    let resolved = resolve_claude_login_runtime_from_config(&data_root)
        .await
        .expect("resolve runtime from provider command");
    assert!(resolved
        .command_abs_path
        .contains("claude-cli-runtime-mock.sh"));
    assert_eq!(resolved.args, vec!["cli.js".to_string()]);
}

#[test]
fn cache_key_provider_matcher_works() {
    assert!(cache_key_matches_provider(
        "7f72430e-4c43-499f-b54d-6ce2deaed4a0/host/codex",
        "codex"
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

#[tokio::test]
async fn cursor_subscription_selection_requires_managed_account() {
    let root = tempfile::tempdir().expect("tempdir");
    let source = harness_sources::HarnessProviderSourceConfig {
        provider_id: "cursor".to_string(),
        selected_source_kind: HarnessSourceKind::Subscription,
        selected_endpoint_id: None,
        endpoints: vec![],
    };
    let active = provider_has_active_auth_config(root.path(), "cursor", Some(&source)).await;
    assert!(!active);
}

#[tokio::test]
async fn amp_subscription_selection_requires_managed_account() {
    let root = tempfile::tempdir().expect("tempdir");
    let source = harness_sources::HarnessProviderSourceConfig {
        provider_id: "amp".to_string(),
        selected_source_kind: HarnessSourceKind::Subscription,
        selected_endpoint_id: None,
        endpoints: vec![],
    };
    let active = provider_has_active_auth_config(root.path(), "amp", Some(&source)).await;
    assert!(!active);
}

#[tokio::test]
async fn amp_active_account_counts_as_active_auth_config() {
    let root = tempfile::tempdir().expect("tempdir");
    provider_accounts::upsert_amp_account(
        root.path(),
        Some("Amp Test".to_string()),
        Some("amp@example.com".to_string()),
    )
    .await
    .expect("upsert amp account");
    let source = harness_sources::HarnessProviderSourceConfig {
        provider_id: "amp".to_string(),
        selected_source_kind: HarnessSourceKind::Subscription,
        selected_endpoint_id: None,
        endpoints: vec![],
    };
    let active = provider_has_active_auth_config(root.path(), "amp", Some(&source)).await;
    assert!(active);
}

#[test]
fn import_result_restart_filter_treats_already_imported_as_mutation() {
    let already_imported = provider_auth_import::ProviderAuthImportResult {
        candidate_id: "cand-1".to_string(),
        provider_id: "claude-crp".to_string(),
        status: "already_imported".to_string(),
        profile_id: Some("acct-1".to_string()),
        message: Some("Matching credential already imported.".to_string()),
    };
    assert!(import_result_requires_provider_restart(&already_imported));
}

#[test]
fn import_result_restart_filter_ignores_non_mutating_statuses() {
    let unsupported = provider_auth_import::ProviderAuthImportResult {
        candidate_id: "cand-2".to_string(),
        provider_id: "cursor".to_string(),
        status: "unsupported".to_string(),
        profile_id: None,
        message: Some("Unsupported in this flow.".to_string()),
    };
    let error = provider_auth_import::ProviderAuthImportResult {
        candidate_id: "cand-3".to_string(),
        provider_id: "codex".to_string(),
        status: "error".to_string(),
        profile_id: None,
        message: Some("failed".to_string()),
    };
    assert!(!import_result_requires_provider_restart(&unsupported));
    assert!(!import_result_requires_provider_restart(&error));
}

#[test]
fn apply_install_target_status_marks_mismatched_managed_target_missing() {
    let mut status = ctx_providers::adapters::ProviderStatus {
        provider_id: "codex".to_string(),
        installed: true,
        detected_path: Some("/tmp/codex".to_string()),
        version: Some("1.0.0".to_string()),
        capabilities: None,
        health: ctx_providers::adapters::ProviderHealth::Ok,
        diagnostics: Vec::new(),
        details: HashMap::from([("managed_target".to_string(), "host".to_string())]),
        usability: ctx_providers::adapters::ProviderUsability::default(),
    };

    installer::apply_install_target_status(&mut status, InstallTarget::Container);

    assert!(!status.installed);
    assert!(matches!(
        status.health,
        ctx_providers::adapters::ProviderHealth::Missing
    ));
    assert_eq!(
        status.details.get("target_mismatch").map(String::as_str),
        Some("true")
    );
}

#[test]
fn apply_install_target_status_keeps_matching_managed_target_healthy() {
    let mut status = ctx_providers::adapters::ProviderStatus {
        provider_id: "codex".to_string(),
        installed: true,
        detected_path: Some("/tmp/codex".to_string()),
        version: Some("1.0.0".to_string()),
        capabilities: None,
        health: ctx_providers::adapters::ProviderHealth::Ok,
        diagnostics: Vec::new(),
        details: HashMap::from([("managed_target".to_string(), "container".to_string())]),
        usability: ctx_providers::adapters::ProviderUsability::default(),
    };

    installer::apply_install_target_status(&mut status, InstallTarget::Container);

    assert!(status.installed);
    assert!(matches!(
        status.health,
        ctx_providers::adapters::ProviderHealth::Ok
    ));
    assert!(!status.details.contains_key("target_mismatch"));
}

#[test]
fn apply_install_target_status_marks_host_detected_status_unverified_for_container() {
    let mut status = ctx_providers::adapters::ProviderStatus {
        provider_id: "codex".to_string(),
        installed: true,
        detected_path: Some("/usr/local/bin/codex".to_string()),
        version: Some("1.0.0".to_string()),
        capabilities: None,
        health: ctx_providers::adapters::ProviderHealth::Ok,
        diagnostics: Vec::new(),
        details: HashMap::new(),
        usability: ctx_providers::adapters::ProviderUsability::default(),
    };

    installer::apply_install_target_status(&mut status, InstallTarget::Container);

    assert!(!status.installed);
    assert!(matches!(
        status.health,
        ctx_providers::adapters::ProviderHealth::Missing
    ));
    assert_eq!(
        status.details.get("target_unverified").map(String::as_str),
        Some("true")
    );
}

#[test]
fn should_skip_install_for_healthy_provider_without_updates() {
    let status = ctx_providers::adapters::ProviderStatus {
        provider_id: "codex".to_string(),
        installed: true,
        detected_path: None,
        version: Some("1.0.0".to_string()),
        capabilities: None,
        health: ctx_providers::adapters::ProviderHealth::Ok,
        diagnostics: Vec::new(),
        details: HashMap::new(),
        usability: ctx_providers::adapters::ProviderUsability::default(),
    };
    assert!(should_skip_install_for_healthy_provider(&status));
}

#[test]
fn should_not_skip_install_for_healthy_provider_with_release_update() {
    let mut details = HashMap::new();
    details.insert("matrix_update_available".to_string(), "true".to_string());
    let status = ctx_providers::adapters::ProviderStatus {
        provider_id: "codex".to_string(),
        installed: true,
        detected_path: None,
        version: Some("1.0.0".to_string()),
        capabilities: None,
        health: ctx_providers::adapters::ProviderHealth::Ok,
        diagnostics: Vec::new(),
        details,
        usability: ctx_providers::adapters::ProviderUsability::default(),
    };
    assert!(!should_skip_install_for_healthy_provider(&status));
}

#[test]
fn should_not_skip_install_for_healthy_provider_with_dependency_update() {
    let mut details = HashMap::new();
    details.insert(
        "managed_dependency_update_available".to_string(),
        "true".to_string(),
    );
    let status = ctx_providers::adapters::ProviderStatus {
        provider_id: "codex".to_string(),
        installed: true,
        detected_path: None,
        version: Some("1.0.0".to_string()),
        capabilities: None,
        health: ctx_providers::adapters::ProviderHealth::Ok,
        diagnostics: Vec::new(),
        details,
        usability: ctx_providers::adapters::ProviderUsability::default(),
    };
    assert!(!should_skip_install_for_healthy_provider(&status));
}

#[test]
fn target_aware_status_does_not_skip_container_install_for_host_only_status() {
    let mut status = ctx_providers::adapters::ProviderStatus {
        provider_id: "codex".to_string(),
        installed: true,
        detected_path: Some("/usr/local/bin/codex".to_string()),
        version: Some("1.0.0".to_string()),
        capabilities: None,
        health: ctx_providers::adapters::ProviderHealth::Ok,
        diagnostics: Vec::new(),
        details: HashMap::new(),
        usability: ctx_providers::adapters::ProviderUsability::default(),
    };

    apply_target_aware_provider_status(
        &mut status,
        &installer::AgentServerConfigFile::default(),
        InstallTarget::Container,
    );

    assert!(!status.installed);
    assert!(matches!(
        status.health,
        ctx_providers::adapters::ProviderHealth::Missing
    ));
    assert!(!should_skip_install_for_healthy_provider(&status));
}

#[derive(Default)]
struct RestartTrackingAdapter {
    restart_calls: AtomicUsize,
}

#[async_trait::async_trait]
impl ProviderAdapter for RestartTrackingAdapter {
    async fn inspect(&self) -> anyhow::Result<ProviderStatus> {
        Ok(ProviderStatus {
            provider_id: "codex".to_string(),
            installed: true,
            detected_path: None,
            version: Some("test".to_string()),
            capabilities: None,
            health: ProviderHealth::Ok,
            diagnostics: Vec::new(),
            details: HashMap::new(),
            usability: ctx_providers::adapters::ProviderUsability::default(),
        })
    }

    async fn run(
        &self,
        _input: TurnInput,
        _workdir: PathBuf,
        _env: HashMap<String, String>,
        _event_sink: tokio::sync::mpsc::Sender<ctx_providers::events::NormalizedEvent>,
    ) -> anyhow::Result<RunHandle> {
        anyhow::bail!("run not used in this test")
    }

    async fn cancel(&self, _handle: RunHandle) -> anyhow::Result<()> {
        Ok(())
    }

    async fn list_processes(&self) -> Vec<ProviderProcessInfo> {
        Vec::new()
    }

    async fn restart(&self, _reason: &str, _mode: ProviderRestartMode) -> anyhow::Result<()> {
        self.restart_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test]
async fn restart_provider_for_auth_change_invalidates_only_matching_provider_probe_caches() {
    let temp = tempfile::tempdir().expect("tempdir");
    let stores = StoreManager::open(temp.path()).await.expect("open stores");
    let adapter = Arc::new(RestartTrackingAdapter::default());
    let state = Arc::new(AppState::new(
        temp.path().to_path_buf(),
        stores,
        HashMap::from([(
            "codex".to_string(),
            adapter.clone() as Arc<dyn ProviderAdapter>,
        )]),
        "http://127.0.0.1:4310".to_string(),
        None,
    ));

    state.providers.options_cache.lock().await.insert(
        "ws-a/host/codex".to_string(),
        crate::daemon::CachedProviderOptions {
            cached_at: std::time::Instant::now(),
            value: serde_json::json!({ "provider_id": "codex", "probe_ok": false }),
        },
    );
    state.providers.options_cache.lock().await.insert(
        "ws-b/container/claude-crp".to_string(),
        crate::daemon::CachedProviderOptions {
            cached_at: std::time::Instant::now(),
            value: serde_json::json!({ "provider_id": "claude-crp", "probe_ok": true }),
        },
    );
    state.providers.verify_cache.lock().await.insert(
        "ws-a/host/codex".to_string(),
        crate::daemon::CachedProviderVerify {
            cached_at: std::time::Instant::now(),
            value: serde_json::json!({ "status": "error" }),
        },
    );
    state.providers.verify_cache.lock().await.insert(
        "ws-b/container/claude-crp".to_string(),
        crate::daemon::CachedProviderVerify {
            cached_at: std::time::Instant::now(),
            value: serde_json::json!({ "status": "ok" }),
        },
    );

    restart_provider_for_auth_change(&state, "codex", "test auth updated").await;

    let options_cache = state.providers.options_cache.lock().await;
    assert!(!options_cache.contains_key("ws-a/host/codex"));
    assert!(options_cache.contains_key("ws-b/container/claude-crp"));
    drop(options_cache);

    let verify_cache = state.providers.verify_cache.lock().await;
    assert!(!verify_cache.contains_key("ws-a/host/codex"));
    assert!(verify_cache.contains_key("ws-b/container/claude-crp"));
    drop(verify_cache);

    assert_eq!(adapter.restart_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn select_provider_harness_source_invalidates_only_matching_provider_probe_caches() {
    let temp = tempfile::tempdir().expect("tempdir");
    let stores = StoreManager::open(temp.path()).await.expect("open stores");
    let state = Arc::new(AppState::new(
        temp.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:4310".to_string(),
        None,
    ));

    state.providers.options_cache.lock().await.insert(
        "ws-a/host/codex".to_string(),
        crate::daemon::CachedProviderOptions {
            cached_at: std::time::Instant::now(),
            value: serde_json::json!({ "provider_id": "codex", "probe_ok": false }),
        },
    );
    state.providers.options_cache.lock().await.insert(
        "ws-b/container/claude-crp".to_string(),
        crate::daemon::CachedProviderOptions {
            cached_at: std::time::Instant::now(),
            value: serde_json::json!({ "provider_id": "claude-crp", "probe_ok": true }),
        },
    );
    state.providers.verify_cache.lock().await.insert(
        "ws-a/host/codex".to_string(),
        crate::daemon::CachedProviderVerify {
            cached_at: std::time::Instant::now(),
            value: serde_json::json!({ "status": "error" }),
        },
    );
    state.providers.verify_cache.lock().await.insert(
        "ws-b/container/claude-crp".to_string(),
        crate::daemon::CachedProviderVerify {
            cached_at: std::time::Instant::now(),
            value: serde_json::json!({ "status": "ok" }),
        },
    );

    let Json(config) = select_provider_harness_source(
        State(Arc::clone(&state)),
        Path("codex".to_string()),
        Json(SelectHarnessSourceReq {
            source_kind: HarnessSourceKind::Subscription,
            endpoint_id: None,
        }),
    )
    .await
    .expect("select provider harness source");

    assert_eq!(config.provider_id, "codex");
    assert_eq!(config.selected_source_kind, HarnessSourceKind::Subscription);

    let options_cache = state.providers.options_cache.lock().await;
    assert!(!options_cache.contains_key("ws-a/host/codex"));
    assert!(options_cache.contains_key("ws-b/container/claude-crp"));
    drop(options_cache);

    let verify_cache = state.providers.verify_cache.lock().await;
    assert!(!verify_cache.contains_key("ws-a/host/codex"));
    assert!(verify_cache.contains_key("ws-b/container/claude-crp"));
}

#[tokio::test]
async fn get_install_statuses_returns_known_and_missing_installs_in_request_order() {
    let temp = tempfile::tempdir().expect("tempdir");
    let stores = StoreManager::open(temp.path()).await.expect("open stores");
    let state = Arc::new(AppState::new(
        temp.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:4310".to_string(),
        None,
    ));

    let (install_id, started_new) = state
        .start_install("codex".to_string(), Some(InstallTarget::Container))
        .await;
    assert!(started_new);
    state
        .emit_install_event(
            install_id,
            InstallProgressEvent {
                install_id,
                provider_id: "codex".to_string(),
                target: Some(InstallTarget::Container),
                at: Utc::now(),
                stage: "download".to_string(),
                message: "Downloading runtime".to_string(),
                level: InstallEventLevel::Info,
                bytes: Some(64),
                total_bytes: Some(128),
                attempt: Some(1),
                error_code: None,
            },
        )
        .await;
    let missing_install_id = InstallId::new_v4();

    let Json(resp) = get_install_statuses(
        State(state),
        Json(GetInstallStatusesReq {
            install_ids: vec![install_id.to_string(), missing_install_id.to_string()],
        }),
    )
    .await
    .expect("get install statuses should succeed");

    assert_eq!(resp.installs.len(), 2);
    assert_eq!(resp.installs[0].install_id, install_id.to_string());
    let info = resp.installs[0]
        .info
        .as_ref()
        .expect("known install should return status");
    assert_eq!(info.provider_id, "codex");
    assert_eq!(info.target, Some(InstallTarget::Container));
    assert!(matches!(
        info.state,
        ctx_provider_install::install_state::InstallStateKind::Running
    ));
    assert_eq!(
        info.last_event.as_ref().map(|event| event.stage.as_str()),
        Some("download")
    );
    assert_eq!(resp.installs[1].install_id, missing_install_id.to_string());
    assert!(resp.installs[1].info.is_none());
}

#[tokio::test]
async fn get_install_statuses_rejects_invalid_install_ids() {
    let temp = tempfile::tempdir().expect("tempdir");
    let stores = StoreManager::open(temp.path()).await.expect("open stores");
    let state = Arc::new(AppState::new(
        temp.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:4310".to_string(),
        None,
    ));

    let err = get_install_statuses(
        State(state),
        Json(GetInstallStatusesReq {
            install_ids: vec!["not-a-uuid".to_string()],
        }),
    )
    .await
    .expect_err("invalid install id should fail");
    let (status, Json(body)) = err;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body.error, "invalid install id: not-a-uuid");
}
