use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use serde_json::Value;
use tower::ServiceExt;

use ctx_core::models::SessionEventType;
use ctx_http::daemon::AppState;
use ctx_managed_installs::{save_agent_server_config, AgentServerCommand, AgentServerConfigFile};
use ctx_providers::adapters::{ProviderAdapter, ProviderHealth, ProviderStatus};
use ctx_providers::crp::Tier1CrpAdapter;
use ctx_store::StoreManager;

mod common;

async fn post_message(app: &axum::Router, session_id: uuid::Uuid, content: &str) {
    let req = Request::builder()
        .method(Method::POST)
        .uri(format!("/api/sessions/{session_id}/messages"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({ "content": content }).to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

async fn wait_for_terminal(state: &Arc<AppState>, session_id: ctx_core::ids::SessionId) {
    let store = state.store_for_session(session_id).await.unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(240);
    loop {
        let events = store.list_session_events(session_id).await.unwrap();
        if events
            .iter()
            .any(|e| matches!(e.event_type, SessionEventType::Done))
        {
            return;
        }
        if events.iter().any(|e| {
            matches!(
                e.event_type,
                SessionEventType::Error | SessionEventType::AuthRequired
            )
        }) {
            panic!("live canary saw terminal error/auth-required events: {events:#?}");
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("timed out waiting for Done event: {events:#?}");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

fn assistant_messages_from_events(events: &[ctx_core::models::SessionEvent]) -> Vec<String> {
    events
        .iter()
        .filter(|e| matches!(e.event_type, SessionEventType::AssistantMessageInserted))
        .filter_map(|e| {
            e.payload_json
                .get("content")
                .and_then(Value::as_str)
                .map(|s| s.to_string())
        })
        .collect()
}

fn resolve_live_claude_crp_command() -> Option<String> {
    if let Ok(raw) = std::env::var("CTX_LIVE_CLAUDE_CRP_COMMAND") {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            eprintln!(
                "skipping: CTX_LIVE_CLAUDE_CRP_COMMAND is set but empty; provide an absolute path"
            );
            return None;
        }
        let path = std::path::PathBuf::from(trimmed);
        if !path.is_absolute() || !path.exists() {
            eprintln!(
                "skipping: CTX_LIVE_CLAUDE_CRP_COMMAND must be an existing absolute path: {trimmed}"
            );
            return None;
        }
        return Some(path.to_string_lossy().to_string());
    }

    let output = Command::new("which").arg("claude-crp").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let detected = String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .to_string();
    if detected.is_empty() {
        return None;
    }
    let path = std::path::PathBuf::from(&detected);
    if !path.is_absolute() || !path.exists() {
        return None;
    }
    Some(path.to_string_lossy().to_string())
}

fn has_claude_cli() -> bool {
    Command::new("which")
        .arg("claude")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

async fn seed_claude_runtime_config(data_root: &Path, command_abs_path: &str) {
    let mut cfg = AgentServerConfigFile::default();
    let command = AgentServerCommand {
        command: command_abs_path.to_string(),
        args: Vec::new(),
        dependencies: Vec::new(),
        managed: None,
    };
    cfg.providers
        .insert("claude-crp".to_string(), command.clone());
    cfg.providers.insert("claude".to_string(), command);
    save_agent_server_config(data_root, &cfg)
        .await
        .expect("write agent server config");
}

async fn seed_provider_status_ok(state: &Arc<AppState>, provider_id: &str) {
    let mut statuses = state.providers.statuses.lock().await;
    statuses.insert(
        provider_id.to_string(),
        ProviderStatus {
            provider_id: provider_id.to_string(),
            installed: true,
            detected_path: None,
            version: None,
            capabilities: None,
            health: ProviderHealth::Ok,
            diagnostics: Vec::new(),
            details: HashMap::new(),
            usability: ctx_providers::adapters::ProviderUsability::default(),
        },
    );
}

#[tokio::test]
#[ignore]
async fn live_provider_canary_turn_invariants() {
    let provider_id = std::env::var("CTX_LIVE_PROVIDER_ID").ok();
    let model_id = std::env::var("CTX_LIVE_MODEL_ID").ok();
    if provider_id.is_none() || model_id.is_none() {
        eprintln!("skipping: set CTX_LIVE_PROVIDER_ID and CTX_LIVE_MODEL_ID to run live canaries");
        return;
    }
    let provider_id = provider_id.unwrap();
    let model_id = model_id.unwrap();

    let adapter: Arc<dyn ProviderAdapter> = match provider_id.as_str() {
        "codex-crp" | "codex-crp" => Arc::new(Tier1CrpAdapter::codex()),
        "claude" | "claude-crp" => Arc::new(Tier1CrpAdapter::claude()),
        _ => {
            eprintln!(
                "skipping: CTX_LIVE_PROVIDER_ID={provider_id} not supported by this canary yet"
            );
            return;
        }
    };

    let repo = common::init_git_repo(&[("note.txt", "hello\n")]).await;
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();

    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert(provider_id.clone(), adapter);

    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:0".to_string(),
        None,
    ));
    let app = ctx_http::api::router(state.clone());
    let ws = common::create_workspace(&app, repo.path(), "ws").await;
    let task = common::create_task(&app, ws.id.0, "t1").await;
    let session = common::create_session(&app, task.id.0, &provider_id, &model_id).await;

    let expected_token = format!("CTX_LIVE_PROVIDER_CANARY_OK_{}", uuid::Uuid::new_v4());
    post_message(
        &app,
        session.id.0,
        &format!("Reply with exactly this token: {expected_token}"),
    )
    .await;
    wait_for_terminal(&state, session.id).await;

    let store = state.store_for_session(session.id).await.unwrap();
    let events = store.list_session_events(session.id).await.unwrap();

    let assistant_messages = assistant_messages_from_events(&events);
    assert!(
        assistant_messages
            .iter()
            .any(|message| message.contains(&expected_token)),
        "expected assistant message containing {expected_token}; saw {assistant_messages:#?} in events {events:#?}"
    );
}

#[tokio::test]
#[ignore]
async fn live_codex_canary_can_edit_workspace_file() {
    let provider_id = std::env::var("CTX_LIVE_PROVIDER_ID")
        .ok()
        .filter(|value| matches!(value.as_str(), "codex-crp" | "codex-crp"))
        .unwrap_or_else(|| "codex-crp".to_string());
    let model_id = std::env::var("CTX_LIVE_MODEL_ID").ok();
    if model_id.is_none() {
        eprintln!("skipping: set CTX_LIVE_MODEL_ID to run the live Codex file-edit canary");
        return;
    }
    let model_id = model_id.unwrap();

    let repo = common::init_git_repo(&[("note.txt", "hello\n")]).await;
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();

    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert(provider_id.clone(), Arc::new(Tier1CrpAdapter::codex()));

    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:0".to_string(),
        None,
    ));
    let app = ctx_http::api::router(state.clone());
    let ws = common::create_workspace(&app, repo.path(), "ws").await;
    let task = common::create_task(&app, ws.id.0, "codex-write").await;
    let session = common::create_session(&app, task.id.0, &provider_id, &model_id).await;

    let expected_token = format!("CTX_LIVE_CODEX_WRITE_OK_{}", uuid::Uuid::new_v4());
    let relative_path = "live-codex-write-proof.txt";
    let prompt = format!(
        "Create or overwrite the workspace file {relative_path}. Write exactly this content and nothing else: {expected_token}. The file must contain exactly those characters with no trailing newline or extra whitespace. If you use a shell command to write the file, use printf rather than echo -n, because echo -n is not portable and may write the literal text -n. After writing the file, reply with exactly this token: {expected_token}"
    );
    post_message(&app, session.id.0, &prompt).await;
    wait_for_terminal(&state, session.id).await;

    let actual = tokio::fs::read_to_string(repo.path().join(relative_path))
        .await
        .expect("live Codex canary should create proof file");
    assert_eq!(
        actual.trim_end(),
        expected_token,
        "live Codex canary wrote unexpected file contents"
    );

    let store = state.store_for_session(session.id).await.unwrap();
    let events = store.list_session_events(session.id).await.unwrap();
    let assistant_messages = assistant_messages_from_events(&events);
    assert!(
        assistant_messages
            .iter()
            .any(|message| message.contains(&expected_token)),
        "expected assistant message containing {expected_token}; saw {assistant_messages:#?} in events {events:#?}"
    );
}

#[tokio::test]
#[ignore]
async fn live_claude_endpoint_profile_api_key_round_trip() {
    let api_key = std::env::var("ANTHROPIC_API_KEY")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    if api_key.is_none() {
        eprintln!("skipping: set ANTHROPIC_API_KEY to run live Claude endpoint canary");
        return;
    }
    let api_key = api_key.unwrap();

    let claude_crp_command = resolve_live_claude_crp_command();
    if claude_crp_command.is_none() {
        eprintln!(
            "skipping: set CTX_LIVE_CLAUDE_CRP_COMMAND to an absolute claude-crp path (or install claude-crp in PATH)"
        );
        return;
    }
    if !has_claude_cli() {
        eprintln!("skipping: Claude CLI is not installed (missing `claude` command)");
        return;
    }
    let claude_crp_command = claude_crp_command.unwrap();

    let base_url = std::env::var("CTX_LIVE_ANTHROPIC_BASE_URL")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "https://api.anthropic.com".to_string());

    let repo = common::init_git_repo(&[("note.txt", "hello\n")]).await;
    let data_dir = tempfile::tempdir().unwrap();
    seed_claude_runtime_config(data_dir.path(), &claude_crp_command).await;

    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let claude_adapter: Arc<dyn ProviderAdapter> = Arc::new(Tier1CrpAdapter::from_raw(
        "claude-crp",
        claude_crp_command.clone(),
        vec![],
    ));
    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert("claude-crp".to_string(), Arc::clone(&claude_adapter));
    providers.insert("claude".to_string(), claude_adapter);

    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:0".to_string(),
        None,
    ));
    let app = ctx_http::api::router(state.clone());
    let provider_id = "claude-crp".to_string();
    seed_provider_status_ok(&state, &provider_id).await;

    let ws = common::create_workspace(&app, repo.path(), "ws").await;
    let endpoint_name = format!("live-claude-endpoint-{}", uuid::Uuid::new_v4());

    let (upsert_status, upsert_body): (StatusCode, Value) = common::json_request(
        &app,
        Method::POST,
        format!("/api/providers/{provider_id}/harness_config/endpoints"),
        Some(serde_json::json!({
            "name": endpoint_name,
            "base_url": base_url,
            "api_shape": "anthropic_messages",
            "api_key": api_key,
        })),
    )
    .await;
    assert_eq!(
        upsert_status,
        StatusCode::OK,
        "failed to create claude endpoint profile: {upsert_body:#?}"
    );
    let endpoint_id = upsert_body
        .get("endpoints")
        .and_then(Value::as_array)
        .and_then(|rows| {
            rows.iter().find_map(|row| {
                let is_match = row
                    .get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|name| name == endpoint_name);
                if !is_match {
                    return None;
                }
                row.get("id").and_then(Value::as_str).map(|s| s.to_string())
            })
        })
        .expect("endpoint id for created claude endpoint");

    let (select_status, select_body): (StatusCode, Value) = common::json_request(
        &app,
        Method::POST,
        format!("/api/providers/{provider_id}/harness_config/select"),
        Some(serde_json::json!({
            "source_kind": "endpoint",
            "endpoint_id": endpoint_id,
        })),
    )
    .await;
    assert_eq!(
        select_status,
        StatusCode::OK,
        "failed to select claude endpoint source: {select_body:#?}"
    );

    let (verify_status, verify_body): (StatusCode, Value) = common::json_request(
        &app,
        Method::POST,
        format!("/api/workspaces/{}/providers/{provider_id}/verify", ws.id.0),
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(
        verify_status,
        StatusCode::OK,
        "provider verify API call failed: {verify_body:#?}"
    );
    assert_eq!(
        verify_body.get("status").and_then(Value::as_str),
        Some("ok"),
        "provider verify did not return ok: {verify_body:#?}"
    );

    let (options_status, options_body): (StatusCode, Value) = common::json_request(
        &app,
        Method::GET,
        format!(
            "/api/workspaces/{}/providers/{provider_id}/options",
            ws.id.0
        ),
        None,
    )
    .await;
    assert_eq!(
        options_status,
        StatusCode::OK,
        "failed to load claude provider options: {options_body:#?}"
    );

    let model_id = std::env::var("CTX_LIVE_MODEL_ID")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .or_else(|| {
            options_body
                .get("models")
                .and_then(|m| m.get("current_model_id"))
                .and_then(Value::as_str)
                .map(|s| s.to_string())
        })
        .or_else(|| {
            options_body
                .get("models")
                .and_then(|m| m.get("models"))
                .and_then(Value::as_array)
                .and_then(|rows| rows.first())
                .and_then(|row| row.get("id"))
                .and_then(Value::as_str)
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "sonnet".to_string());

    let task = common::create_task(&app, ws.id.0, "live-claude-endpoint-task").await;
    let session = common::create_session(&app, task.id.0, &provider_id, &model_id).await;

    post_message(
        &app,
        session.id.0,
        "Reply with exactly this token: CLAUDE_ENDPOINT_E2E_OK",
    )
    .await;
    wait_for_terminal(&state, session.id).await;

    let store = state.store_for_session(session.id).await.unwrap();
    let events = store.list_session_events(session.id).await.unwrap();
    let assistant_messages: Vec<String> = events
        .iter()
        .filter(|e| matches!(e.event_type, SessionEventType::AssistantMessageInserted))
        .filter_map(|e| {
            e.payload_json
                .get("content")
                .and_then(Value::as_str)
                .map(|s| s.to_string())
        })
        .collect();
    assert!(
        assistant_messages
            .iter()
            .any(|message| message.contains("CLAUDE_ENDPOINT_E2E_OK")),
        "expected assistant message containing CLAUDE_ENDPOINT_E2E_OK; saw {assistant_messages:#?} in events {events:#?}"
    );
}

#[tokio::test]
#[ignore]
async fn live_claude_openrouter_opus_v1_base_url_normalization_round_trip() {
    let api_key = std::env::var("OPENROUTER_API_KEY")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    if api_key.is_none() {
        eprintln!("skipping: set OPENROUTER_API_KEY to run live OpenRouter Claude endpoint canary");
        return;
    }
    let api_key = api_key.unwrap();

    let claude_crp_command = resolve_live_claude_crp_command();
    if claude_crp_command.is_none() {
        eprintln!(
            "skipping: set CTX_LIVE_CLAUDE_CRP_COMMAND to an absolute claude-crp path (or install claude-crp in PATH)"
        );
        return;
    }
    if !has_claude_cli() {
        eprintln!("skipping: Claude CLI is not installed (missing `claude` command)");
        return;
    }
    let claude_crp_command = claude_crp_command.unwrap();

    let openrouter_base = std::env::var("OPENROUTER_BASE_URL")
        .ok()
        .or_else(|| std::env::var("CTX_LIVE_OPENROUTER_BASE_URL").ok())
        .map(|v| v.trim().trim_end_matches('/').to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "https://openrouter.ai/api/v1".to_string());
    let input_base_with_v1 = if openrouter_base.to_ascii_lowercase().ends_with("/v1") {
        openrouter_base
    } else {
        format!("{openrouter_base}/v1")
    };
    let expected_normalized_base = input_base_with_v1
        .strip_suffix("/v1")
        .expect("input base URL has /v1 suffix")
        .to_string();

    let repo = common::init_git_repo(&[("note.txt", "hello\n")]).await;
    let data_dir = tempfile::tempdir().unwrap();
    seed_claude_runtime_config(data_dir.path(), &claude_crp_command).await;

    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let claude_adapter: Arc<dyn ProviderAdapter> = Arc::new(Tier1CrpAdapter::from_raw(
        "claude-crp",
        claude_crp_command.clone(),
        vec![],
    ));
    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert("claude-crp".to_string(), Arc::clone(&claude_adapter));
    providers.insert("claude".to_string(), claude_adapter);

    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:0".to_string(),
        None,
    ));
    let app = ctx_http::api::router(state.clone());
    let provider_id = "claude-crp".to_string();
    seed_provider_status_ok(&state, &provider_id).await;

    let ws = common::create_workspace(&app, repo.path(), "ws").await;
    let endpoint_name = format!("live-claude-openrouter-{}", uuid::Uuid::new_v4());
    let requested_model = "anthropic/claude-opus-4.6";

    let (upsert_status, upsert_body): (StatusCode, Value) = common::json_request(
        &app,
        Method::POST,
        format!("/api/providers/{provider_id}/harness_config/endpoints"),
        Some(serde_json::json!({
            "name": endpoint_name,
            "base_url": input_base_with_v1,
            "api_shape": "anthropic_messages",
            "api_key": api_key,
            "model_override": requested_model,
        })),
    )
    .await;
    assert_eq!(
        upsert_status,
        StatusCode::OK,
        "failed to create OpenRouter endpoint profile: {upsert_body:#?}"
    );
    let endpoint = upsert_body
        .get("endpoints")
        .and_then(Value::as_array)
        .and_then(|rows| {
            rows.iter().find(|row| {
                row.get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|name| name == endpoint_name)
            })
        })
        .expect("created endpoint in response");
    let endpoint_id = endpoint
        .get("id")
        .and_then(Value::as_str)
        .expect("created endpoint id");
    let endpoint_base = endpoint
        .get("base_url")
        .and_then(Value::as_str)
        .expect("created endpoint base_url");
    assert_eq!(
        endpoint_base, expected_normalized_base,
        "claude endpoint base URL should strip trailing /v1 for anthropic_messages"
    );

    let (select_status, select_body): (StatusCode, Value) = common::json_request(
        &app,
        Method::POST,
        format!("/api/providers/{provider_id}/harness_config/select"),
        Some(serde_json::json!({
            "source_kind": "endpoint",
            "endpoint_id": endpoint_id,
        })),
    )
    .await;
    assert_eq!(
        select_status,
        StatusCode::OK,
        "failed to select OpenRouter endpoint source: {select_body:#?}"
    );

    let (verify_status, verify_body): (StatusCode, Value) = common::json_request(
        &app,
        Method::POST,
        format!("/api/workspaces/{}/providers/{provider_id}/verify", ws.id.0),
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(
        verify_status,
        StatusCode::OK,
        "provider verify API call failed: {verify_body:#?}"
    );
    assert_eq!(
        verify_body.get("status").and_then(Value::as_str),
        Some("ok"),
        "provider verify did not return ok: {verify_body:#?}"
    );

    let task = common::create_task(&app, ws.id.0, "live-claude-openrouter-opus").await;
    let session = common::create_session(&app, task.id.0, &provider_id, requested_model).await;

    post_message(
        &app,
        session.id.0,
        "Reply with exactly this token: OPENROUTER_CLAUDE_OPUS_46_OK",
    )
    .await;
    wait_for_terminal(&state, session.id).await;

    let (head_status, head_body): (StatusCode, Value) = common::json_request(
        &app,
        Method::GET,
        format!("/api/sessions/{}/head?limit=120", session.id.0),
        None,
    )
    .await;
    assert_eq!(
        head_status,
        StatusCode::OK,
        "failed to load session head: {head_body:#?}"
    );
    let assistant_text = head_body
        .get("messages")
        .and_then(Value::as_array)
        .and_then(|rows| {
            rows.iter()
                .rev()
                .find(|row| row.get("role").and_then(Value::as_str) == Some("assistant"))
        })
        .and_then(|row| row.get("content"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    assert!(
        assistant_text.contains("OPENROUTER_CLAUDE_OPUS_46_OK"),
        "assistant response did not include expected marker: {assistant_text}"
    );
}
