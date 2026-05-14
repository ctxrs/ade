use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use tower::ServiceExt;

use ctx_core::models::{MessageRole, SessionEventType, SessionTurnStatus};
use ctx_http::daemon::DaemonState;
use ctx_store::StoreManager;

mod common;

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
const STORAGE_GUARD_EMERGENCY_FREE_BYTES: u64 = 1024 * 1024 * 1024;
const CRP_FIXTURE_FIRST_EVENT_TIMEOUT_MS: &str = "60000";

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

fn host_has_sufficient_free_space(paths: &[&std::path::Path]) -> bool {
    let min_free_bytes = paths
        .iter()
        .filter_map(|path| fs2::available_space(path).ok())
        .min()
        .unwrap_or(u64::MAX);
    min_free_bytes > STORAGE_GUARD_EMERGENCY_FREE_BYTES
}

async fn configure_hermetic_codex_home() -> (tempfile::TempDir, EnvGuard) {
    let codex_home = tempfile::tempdir().unwrap();
    tokio::fs::write(
        codex_home.path().join("auth.json"),
        br#"{"OPENAI_API_KEY":"test-key"}"#,
    )
    .await
    .unwrap();
    let guard = EnvGuard::set("CTX_CODEX_HOME", &codex_home.path().to_string_lossy());
    (codex_home, guard)
}

async fn fixture_model_id_for_provider(
    app: &axum::Router,
    workspace_id: uuid::Uuid,
    provider_id: &str,
) -> String {
    let fallback_model_id = match provider_id {
        "codex" => Some("gpt-5.4/medium"),
        "claude-crp" => Some("default/medium"),
        _ => None,
    };
    let (status, body): (StatusCode, serde_json::Value) = common::json_request(
        app,
        Method::GET,
        format!("/api/workspaces/{workspace_id}/providers/{provider_id}/options"),
        None,
    )
    .await;
    if status != StatusCode::OK {
        return fallback_model_id.unwrap_or("fake-model").to_string();
    }
    body.pointer("/models/current_model_id")
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            body.pointer("/models/models")
                .and_then(serde_json::Value::as_array)
                .and_then(|models| models.first())
                .and_then(|model| model.get("id"))
                .and_then(serde_json::Value::as_str)
        })
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or(fallback_model_id)
        .unwrap_or("fake-model")
        .to_string()
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

async fn wait_for_done(state: &Arc<DaemonState>, session_id: ctx_core::ids::SessionId) {
    wait_for_done_inner(state, session_id, false).await;
}

async fn wait_for_done_and_completed_turn(
    state: &Arc<DaemonState>,
    session_id: ctx_core::ids::SessionId,
) {
    wait_for_done_inner(state, session_id, true).await;
}

async fn wait_for_done_inner(
    state: &Arc<DaemonState>,
    session_id: ctx_core::ids::SessionId,
    require_completed_turn: bool,
) {
    let store = state.store_for_session(session_id).await.unwrap();
    let timeout_secs = std::env::var("CTX_TEST_WAIT_FOR_DONE_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(120);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);
    loop {
        let events = store.list_session_events(session_id).await.unwrap();
        if events
            .iter()
            .any(|e| matches!(e.event_type, SessionEventType::Error))
        {
            let provider_logs = provider_log_snapshot(&state.core.data_root);
            panic!("saw Error event(s): {events:#?}\nprovider logs:\n{provider_logs}");
        }
        let saw_done = events
            .iter()
            .any(|e| matches!(e.event_type, SessionEventType::Done));
        if saw_done {
            if !require_completed_turn {
                return;
            }
            let turns = store
                .list_session_turns_page_by_seq(session_id, None, Some(1))
                .await
                .unwrap();
            if let Some(turn) = turns.last() {
                match turn.status {
                    SessionTurnStatus::Completed => return,
                    SessionTurnStatus::Failed | SessionTurnStatus::Interrupted => {
                        panic!("saw terminal non-completed turn after Done: {turn:#?}");
                    }
                    SessionTurnStatus::Queued
                    | SessionTurnStatus::Starting
                    | SessionTurnStatus::Running => {}
                }
            }
        }
        if tokio::time::Instant::now() >= deadline {
            let provider_logs = provider_log_snapshot(&state.core.data_root);
            let turns = store
                .list_session_turns_page_by_seq(session_id, None, Some(3))
                .await
                .unwrap_or_default();
            panic!(
                "timed out waiting for Done event after {timeout_secs}s: {events:#?}\nturns:\n{turns:#?}\nprovider logs:\n{provider_logs}"
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn provider_log_snapshot(data_root: &std::path::Path) -> String {
    let dir = data_root.join("logs").join("providers");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return format!("missing provider log dir: {}", dir.display());
    };
    let mut files = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    files.sort();

    if files.is_empty() {
        return format!("empty provider log dir: {}", dir.display());
    }

    files
        .into_iter()
        .map(|path| {
            let contents = std::fs::read_to_string(&path)
                .unwrap_or_else(|err| format!("<failed to read: {err}>"));
            format!("== {} ==\n{}", path.display(), contents)
        })
        .collect::<Vec<_>>()
        .join("\n")
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

fn check_event_count_exact(
    events: &[ctx_core::models::SessionEvent],
    event_type: SessionEventType,
    expected_count: usize,
) -> Result<(), String> {
    let count = count_events(events, event_type.clone());
    if count != expected_count {
        Err(format!(
            "expected exactly {expected_count} {event_type:?} events; saw {count}: {events:#?}"
        ))
    } else {
        Ok(())
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

fn read_event_order_seq(event: &ctx_core::models::SessionEvent) -> Option<i64> {
    event
        .payload_json
        .get("order_seq")
        .or_else(|| event.payload_json.get("orderSeq"))
        .and_then(|value| {
            value.as_i64().or_else(|| {
                value
                    .as_str()
                    .and_then(|text| text.trim().parse::<i64>().ok())
            })
        })
}

#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn provider_scenarios_offline_crp_fixtures() {
    let _env_lock = lock_env();

    let Some(python) = common::crp_fixture_runtime::python_binary() else {
        eprintln!("skipping: python3/python not found");
        return;
    };

    let fixtures_dir = common::resolve_manifest_dir()
        .join("tests")
        .join("fixtures")
        .join("provider_scenarios");
    let _guard_fixtures = EnvGuard::set("CTX_TEST_FIXTURES_DIR", &fixtures_dir.to_string_lossy());
    let _guard_scenario = EnvGuard::set("CTX_TEST_SCENARIO", "basic");
    let _guard_first_event_timeout = EnvGuard::set(
        "CTX_CRP_FIRST_EVENT_TIMEOUT_MS",
        CRP_FIXTURE_FIRST_EVENT_TIMEOUT_MS,
    );
    let _guard_ctx_mcp = common::set_ctx_mcp_command_env_for_test();

    let provider_ids: &[&str] = &[
        "codex",
        "claude-crp",
        "claude",
        // ACP bridge providers
        "gemini",
        "qwen",
        "cursor",
        "pi",
        "opencode",
        "mistral",
        "goose",
        "kimi",
        "auggie",
        "amp",
        "droid",
        "copilot",
        "continue",
        "cline",
        "openhands",
    ];

    let repo = common::init_git_repo(&[("note.txt", "hello\n")]).await;
    let data_dir = tempfile::tempdir().unwrap();
    if !host_has_sufficient_free_space(&[repo.path(), data_dir.path(), &std::env::temp_dir()]) {
        eprintln!("skipping: storage guard would trip on low-disk test host");
        return;
    }
    let (_codex_home, _guard_codex_home) = configure_hermetic_codex_home().await;
    let _guard_mcp_disabled = EnvGuard::set("CTX_MCP_DISABLED", "1");
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let script_path = common::crp_fixture_runtime::write_crp_fixture_runtime(data_dir.path());
    common::seed_managed_codex_cli_host_runtime_with_args(
        data_dir.path(),
        &python,
        vec![script_path.to_string_lossy().to_string()],
    )
    .await;
    let providers = common::crp_fixture_runtime::build_crp_fixture_providers(
        provider_ids,
        &python,
        &script_path,
    );

    let state = Arc::new(DaemonState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:0".to_string(),
        None,
    ));
    let app = ctx_http::api::router(state.clone());

    let ws = common::create_workspace(&app, repo.path(), "ws").await;

    let mut failures: HashMap<&str, String> = HashMap::new();
    for provider_id in provider_ids {
        let model_id = fixture_model_id_for_provider(&app, ws.id.0, provider_id).await;
        let (_task, session) =
            common::create_task_with_session(&app, ws.id.0, "t1", provider_id, &model_id).await;

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

#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn provider_scenarios_offline_interleaved_assistant_tools_do_not_fragment_messages() {
    let _env_lock = lock_env();

    let Some(python) = common::crp_fixture_runtime::python_binary() else {
        eprintln!("skipping: python3/python not found");
        return;
    };

    let fixtures_dir = common::resolve_manifest_dir()
        .join("tests")
        .join("fixtures")
        .join("provider_scenarios");
    let _guard_fixtures = EnvGuard::set("CTX_TEST_FIXTURES_DIR", &fixtures_dir.to_string_lossy());
    let _guard_scenario = EnvGuard::set("CTX_TEST_SCENARIO", "interleaved_assistant_tools");
    let _guard_first_event_timeout = EnvGuard::set(
        "CTX_CRP_FIRST_EVENT_TIMEOUT_MS",
        CRP_FIXTURE_FIRST_EVENT_TIMEOUT_MS,
    );
    let _guard_ctx_mcp = common::set_ctx_mcp_command_env_for_test();

    let repo = common::init_git_repo(&[("note.txt", "hello\n")]).await;
    let data_dir = tempfile::tempdir().unwrap();
    if !host_has_sufficient_free_space(&[repo.path(), data_dir.path(), &std::env::temp_dir()]) {
        eprintln!("skipping: storage guard would trip on low-disk test host");
        return;
    }
    let (_codex_home, _guard_codex_home) = configure_hermetic_codex_home().await;
    let _guard_mcp_disabled = EnvGuard::set("CTX_MCP_DISABLED", "1");
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let script_path = common::crp_fixture_runtime::write_crp_fixture_runtime(data_dir.path());
    common::seed_managed_codex_cli_host_runtime_with_args(
        data_dir.path(),
        &python,
        vec![script_path.to_string_lossy().to_string()],
    )
    .await;
    let providers =
        common::crp_fixture_runtime::build_crp_fixture_providers(&["codex"], &python, &script_path);

    let state = Arc::new(DaemonState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:0".to_string(),
        None,
    ));
    let app = ctx_http::api::router(state.clone());

    let ws = common::create_workspace(&app, repo.path(), "ws").await;
    let (_task, session) =
        common::create_task_with_session(&app, ws.id.0, "t1", "codex", "fake-model").await;

    post_message(&app, session.id.0, "hi").await;
    wait_for_done(&state, session.id).await;

    let store = state.store_for_session(session.id).await.unwrap();
    let events = store.list_session_events(session.id).await.unwrap();
    let messages = store.list_messages_for_session(session.id).await.unwrap();

    check_event_count_exact(&events, SessionEventType::AssistantMessageInserted, 1)
        .unwrap_or_else(|err| panic!("{err}"));

    let assistant_messages = messages
        .iter()
        .filter(|message| matches!(message.role, MessageRole::Assistant))
        .collect::<Vec<_>>();
    assert_eq!(
        assistant_messages.len(),
        1,
        "expected one persisted assistant message, saw: {assistant_messages:#?}"
    );
    assert_eq!(
        assistant_messages[0].content,
        "Synthetic assistant text crosses tool events. The fixture keeps the message contiguous across calls. The final text remains deterministic."
    );

    let assistant_inserted = events
        .iter()
        .find(|event| matches!(event.event_type, SessionEventType::AssistantMessageInserted))
        .unwrap_or_else(|| panic!("missing AssistantMessageInserted event: {events:#?}"));
    let first_tool_call = events
        .iter()
        .find(|event| matches!(event.event_type, SessionEventType::ToolCall))
        .unwrap_or_else(|| panic!("missing ToolCall event: {events:#?}"));

    let assistant_inserted_order = read_event_order_seq(assistant_inserted).unwrap_or_else(|| {
        panic!("AssistantMessageInserted missing order_seq: {assistant_inserted:#?}")
    });
    let tool_order = read_event_order_seq(first_tool_call)
        .unwrap_or_else(|| panic!("ToolCall missing order_seq: {first_tool_call:#?}"));
    let message_order = assistant_messages[0]
        .order_seq
        .unwrap_or_else(|| panic!("assistant message missing order_seq: {assistant_messages:#?}"));

    assert_eq!(
        message_order, assistant_inserted_order,
        "persisted assistant message should reuse assistant_message_inserted order_seq"
    );
    assert!(
        message_order < tool_order,
        "interleaved assistant message should stay anchored before later tool calls; message={message_order}, tool={tool_order}, events={events:#?}"
    );
}

#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn provider_scenarios_offline_crp_fixtures_persist_context_window_metrics() {
    let _env_lock = lock_env();

    let Some(python) = common::crp_fixture_runtime::python_binary() else {
        eprintln!("skipping: python3/python not found");
        return;
    };

    let fixtures_dir = common::resolve_manifest_dir()
        .join("tests")
        .join("fixtures")
        .join("provider_scenarios");
    let _guard_fixtures = EnvGuard::set("CTX_TEST_FIXTURES_DIR", &fixtures_dir.to_string_lossy());
    let _guard_scenario = EnvGuard::set("CTX_TEST_SCENARIO", "context_window");
    let _guard_first_event_timeout = EnvGuard::set(
        "CTX_CRP_FIRST_EVENT_TIMEOUT_MS",
        CRP_FIXTURE_FIRST_EVENT_TIMEOUT_MS,
    );
    let _guard_ctx_mcp = common::set_ctx_mcp_command_env_for_test();

    let repo = common::init_git_repo(&[("note.txt", "hello\n")]).await;
    let data_dir = tempfile::tempdir().unwrap();
    if !host_has_sufficient_free_space(&[repo.path(), data_dir.path(), &std::env::temp_dir()]) {
        eprintln!("skipping: storage guard would trip on low-disk test host");
        return;
    }
    let (_codex_home, _guard_codex_home) = configure_hermetic_codex_home().await;
    let _guard_mcp_disabled = EnvGuard::set("CTX_MCP_DISABLED", "1");
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let script_path = common::crp_fixture_runtime::write_crp_fixture_runtime(data_dir.path());
    common::seed_managed_codex_cli_host_runtime_with_args(
        data_dir.path(),
        &python,
        vec![script_path.to_string_lossy().to_string()],
    )
    .await;
    let provider_ids: &[&str] = &["codex", "claude-crp"];
    let providers = common::crp_fixture_runtime::build_crp_fixture_providers(
        provider_ids,
        &python,
        &script_path,
    );

    let state = Arc::new(DaemonState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:0".to_string(),
        None,
    ));
    let app = ctx_http::api::router(state.clone());

    let ws = common::create_workspace(&app, repo.path(), "ws").await;

    for provider_id in provider_ids {
        let model_id = fixture_model_id_for_provider(&app, ws.id.0, provider_id).await;
        let (_task, session) =
            common::create_task_with_session(&app, ws.id.0, "t1", provider_id, &model_id).await;

        post_message(&app, session.id.0, "hi").await;
        wait_for_done_and_completed_turn(&state, session.id).await;

        let store = state.store_for_session(session.id).await.unwrap();
        let turns = store
            .list_session_turns_page_by_seq(session.id, None, Some(10))
            .await
            .unwrap();
        let turn = turns
            .last()
            .unwrap_or_else(|| panic!("expected one completed turn for {provider_id}: {turns:#?}"));
        let metrics = turn.metrics_json.as_ref().unwrap_or_else(|| {
            panic!("expected metrics_json on final turn for {provider_id}: {turn:#?}")
        });

        assert_eq!(
            metrics
                .get("context_window_tokens")
                .and_then(serde_json::Value::as_u64),
            Some(200_000),
            "unexpected context_window_tokens for {provider_id}: {metrics:#?}"
        );
        assert_eq!(
            metrics
                .get("context_tokens_estimate")
                .and_then(serde_json::Value::as_u64),
            Some(50_000),
            "unexpected context_tokens_estimate for {provider_id}: {metrics:#?}"
        );
        assert_eq!(
            metrics
                .get("remaining_tokens_estimate")
                .and_then(serde_json::Value::as_u64),
            Some(150_000),
            "unexpected remaining_tokens_estimate for {provider_id}: {metrics:#?}"
        );
        let remaining_fraction = metrics
            .get("remaining_fraction")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or_else(|| {
                panic!("expected remaining_fraction for {provider_id}: {metrics:#?}")
            });
        assert!(
            (remaining_fraction - 0.75).abs() < 1e-9,
            "unexpected remaining_fraction for {provider_id}: {remaining_fraction}; metrics={metrics:#?}"
        );
    }
}
