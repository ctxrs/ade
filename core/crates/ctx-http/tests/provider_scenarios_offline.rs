use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use tower::ServiceExt;

use ctx_core::models::SessionEventType;
use ctx_http::daemon::AppState;
use ctx_store::StoreManager;

mod common;

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn lock_env() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner())
}

struct EnvGuard {
    key: &'static str,
    prev: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let prev = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, prev }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        if let Some(value) = self.prev.take() {
            std::env::set_var(self.key, value);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

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

async fn wait_for_done(state: &Arc<AppState>, session_id: ctx_core::ids::SessionId) {
    let store = state.store_for_session(session_id).await.unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    loop {
        let events = store.list_session_events(session_id).await.unwrap();
        if events
            .iter()
            .any(|e| matches!(e.event_type, SessionEventType::Done))
        {
            if events
                .iter()
                .any(|e| matches!(e.event_type, SessionEventType::Error))
            {
                panic!("saw Error event(s): {events:#?}");
            }
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("timed out waiting for Done event: {events:#?}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn check_no_session_gap(events: &[ctx_core::models::SessionEvent]) -> Result<(), String> {
    let saw_gap = events.iter().any(|event| {
        matches!(event.event_type, SessionEventType::Notice)
            && event
                .payload_json
                .get("kind")
                .and_then(|value| value.as_str())
                == Some("session_gap")
    });
    if saw_gap {
        Err(format!("unexpected session_gap event: {events:#?}"))
    } else {
        Ok(())
    }
}

fn count_events(events: &[ctx_core::models::SessionEvent], event_type: SessionEventType) -> usize {
    let target = std::mem::discriminant(&event_type);
    events
        .iter()
        .filter(|e| std::mem::discriminant(&e.event_type) == target)
        .count()
}

fn check_event_count_at_least(
    events: &[ctx_core::models::SessionEvent],
    event_type: SessionEventType,
    min_count: usize,
) -> Result<(), String> {
    let count = count_events(events, event_type.clone());
    if count < min_count {
        Err(format!(
            "expected at least {min_count} {event_type:?} events; saw {count}: {events:#?}"
        ))
    } else {
        Ok(())
    }
}

fn check_assistant_message_inserted_contains(
    events: &[ctx_core::models::SessionEvent],
    expected: &str,
) -> Result<(), String> {
    let saw = events.iter().any(|e| {
        matches!(e.event_type, SessionEventType::AssistantMessageInserted)
            && e.payload_json
                .get("content")
                .and_then(|v| v.as_str())
                .is_some_and(|v| v.contains(expected))
    });
    if saw {
        Ok(())
    } else {
        Err(format!(
            "expected AssistantMessageInserted to contain {expected:?}; events: {events:#?}"
        ))
    }
}

fn check_turn_thought_partial_contains(
    turns: &[ctx_core::models::SessionTurn],
    expected: &str,
) -> Result<(), String> {
    let Some(last) = turns.last() else {
        return Err("expected at least one turn; saw none".to_string());
    };
    let thought = last.thought_partial.as_deref().unwrap_or("");
    if thought.contains(expected) {
        Ok(())
    } else {
        Err(format!(
            "expected thought_partial to contain {expected:?}; saw {thought:?}; turns: {turns:#?}"
        ))
    }
}

#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn provider_scenarios_offline_crp_fixtures() {
    let _env_lock = lock_env();

    let Some(python) = common::crp_fixture_runtime::python_binary() else {
        eprintln!("skipping: python3/python not found");
        return;
    };

    let fixtures_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("provider_scenarios");
    let _guard_fixtures = EnvGuard::set("CTX_TEST_FIXTURES_DIR", &fixtures_dir.to_string_lossy());
    let _guard_scenario = EnvGuard::set("CTX_TEST_SCENARIO", "basic");

    let provider_ids: &[&str] = &[
        "codex-crp",
        "codex",
        "claude-crp",
        "claude",
        // ACP bridge providers
        "gemini",
        "qwen",
        "opencode",
        "mistral",
        "goose",
        "kimi",
        "auggie",
        "cagent",
        "amp",
        "droid",
        "copilot",
        "kiro",
        "rovo",
        "cody",
        "continue",
        "cline",
        "swe-agent",
        "openhands",
    ];

    let repo = common::init_git_repo(&[("note.txt", "hello\n")]).await;
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let script_path = common::crp_fixture_runtime::write_crp_fixture_runtime(data_dir.path());
    let providers = common::crp_fixture_runtime::build_crp_fixture_providers(
        provider_ids,
        &python,
        &script_path,
    );

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

    let mut failures: HashMap<&str, String> = HashMap::new();
    for provider_id in provider_ids {
        let session = common::create_session(&app, task.id.0, provider_id, "fake-model").await;

        post_message(&app, session.id.0, "hi").await;
        wait_for_done(&state, session.id).await;

        let store = state.store_for_session(session.id).await.unwrap();
        let events = store.list_session_events(session.id).await.unwrap();
        let turns = store
            .list_session_turns_page_by_seq(session.id, None, Some(10))
            .await
            .unwrap();

        let mut errs: Vec<String> = Vec::new();
        if let Err(err) = check_no_session_gap(&events) {
            errs.push(err);
        }
        if let Err(err) = check_event_count_at_least(&events, SessionEventType::ToolCall, 1) {
            errs.push(err);
        }
        if let Err(err) = check_event_count_at_least(&events, SessionEventType::ToolResult, 1) {
            errs.push(err);
        }
        if let Err(err) = check_assistant_message_inserted_contains(&events, provider_id) {
            errs.push(err);
        }
        if let Err(err) = check_turn_thought_partial_contains(&turns, provider_id) {
            errs.push(err);
        }

        if !errs.is_empty() {
            failures.insert(*provider_id, errs.join("\n"));
        }
    }

    assert!(
        failures.is_empty(),
        "provider scenario assertion failures: {failures:#?}"
    );
}
