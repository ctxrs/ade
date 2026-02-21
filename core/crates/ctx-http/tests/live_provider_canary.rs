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
use ctx_http::installer::{save_agent_server_config, AgentServerCommand, AgentServerConfigFile};
use ctx_providers::adapters::ProviderAdapter;
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
                "skipping: CTX_LIVE_CLAUDE_CRP_COMMAND must be an existing absolute path: {}",
                trimmed
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

async fn seed_claude_runtime_config(data_root: &Path, command_abs_path: String) {
    let mut cfg = AgentServerConfigFile::default();
    cfg.providers.insert(
        "claude-crp".to_string(),
        AgentServerCommand {
            command: command_abs_path,
            args: Vec::new(),
            dependencies: Vec::new(),
            managed: None,
        },
    );
    save_agent_server_config(data_root, &cfg)
        .await
        .expect("write agent server config");
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
        "codex" | "codex-crp" => Arc::new(Tier1CrpAdapter::codex()),
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

    post_message(&app, session.id.0, "Say hi in one short sentence.").await;
    wait_for_terminal(&state, session.id).await;

    let store = state.store_for_session(session.id).await.unwrap();
    let events = store.list_session_events(session.id).await.unwrap();

    let assistant_finals = events
        .iter()
        .filter(|e| matches!(e.event_type, SessionEventType::AssistantComplete))
        .count();
    assert!(
        assistant_finals >= 1,
        "expected at least 1 AssistantComplete; saw {assistant_finals}: {events:#?}"
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
    seed_claude_runtime_config(data_dir.path(), claude_crp_command).await;

    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert("claude-crp".to_string(), Arc::new(Tier1CrpAdapter::claude()));

    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:0".to_string(),
        None,
    ));
    let app = ctx_http::api::router(state.clone());

    let ws = common::create_workspace(&app, repo.path(), "ws").await;
    let endpoint_name = format!("live-claude-endpoint-{}", uuid::Uuid::new_v4());

    let (upsert_status, upsert_body): (StatusCode, Value) = common::json_request(
        &app,
        Method::POST,
        "/api/providers/claude-crp/harness_config/endpoints",
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
        "/api/providers/claude-crp/harness_config/select",
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
        format!("/api/workspaces/{}/providers/claude-crp/verify", ws.id.0),
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
        format!("/api/workspaces/{}/providers/claude-crp/options", ws.id.0),
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
    let session = common::create_session(&app, task.id.0, "claude-crp", &model_id).await;

    post_message(
        &app,
        session.id.0,
        "Reply with exactly this token: CLAUDE_ENDPOINT_E2E_OK",
    )
    .await;
    wait_for_terminal(&state, session.id).await;

    let store = state.store_for_session(session.id).await.unwrap();
    let events = store.list_session_events(session.id).await.unwrap();
    let assistant_finals = events
        .iter()
        .filter(|e| matches!(e.event_type, SessionEventType::AssistantComplete))
        .count();
    assert!(
        assistant_finals >= 1,
        "expected at least 1 AssistantComplete; saw {assistant_finals}: {events:#?}"
    );
}
