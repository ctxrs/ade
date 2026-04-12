use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use tower::ServiceExt;

use ctx_core::models::SessionEventType;
use ctx_http::daemon::AppState;
use ctx_providers::adapters::{ProviderAdapter, ProviderSessionSweepConfig};
use ctx_providers::crp::Tier1CrpAdapter;
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

fn write_resume_fixture(root: &Path, provider_id: &str) {
    let provider_dir = root.join(provider_id);
    std::fs::create_dir_all(&provider_dir).unwrap();
    std::fs::write(
        provider_dir.join("resume.json"),
        serde_json::json!({
            "turns": [
                { "final": format!("first response from {provider_id}") },
                { "final": format!("second response from {provider_id}") }
            ]
        })
        .to_string(),
    )
    .unwrap();
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

async fn wait_for_done_count(
    state: &Arc<AppState>,
    session_id: ctx_core::ids::SessionId,
    expected_done_count: usize,
) {
    let store = state.store_for_session(session_id).await.unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        let events = store.list_session_events(session_id).await.unwrap();
        let done_count = events
            .iter()
            .filter(|event| matches!(event.event_type, SessionEventType::Done))
            .count();
        assert!(
            !events
                .iter()
                .any(|event| matches!(event.event_type, SessionEventType::Error)),
            "unexpected Error event(s): {events:#?}"
        );
        if done_count >= expected_done_count {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "timed out waiting for {expected_done_count} Done events; saw {done_count}: {events:#?}"
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn wait_for_provider_session_ref(
    state: &Arc<AppState>,
    session_id: ctx_core::ids::SessionId,
) -> String {
    let store = state.store_for_session(session_id).await.unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let session = store
            .get_session(session_id)
            .await
            .unwrap()
            .expect("session should exist");
        if let Some(provider_session_ref) = session.provider_session_ref {
            return provider_session_ref;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("timed out waiting for provider_session_ref");
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn assert_provider_session_resume_after_idle_reap(provider_id: &str, model_id: &str) {
    let Some(python) = common::crp_fixture_runtime::python_binary() else {
        eprintln!("skipping: python3/python not found");
        return;
    };

    let repo = common::init_git_repo(&[("note.txt", "hello\n")]).await;
    let data_dir = tempfile::tempdir().unwrap();
    let fixtures_dir = tempfile::tempdir().unwrap();
    let command_log = data_dir
        .path()
        .join(format!("{provider_id}-commands.jsonl"));
    write_resume_fixture(fixtures_dir.path(), provider_id);

    let _guard_fixtures = EnvGuard::set(
        "CTX_TEST_FIXTURES_DIR",
        &fixtures_dir.path().to_string_lossy(),
    );
    let _guard_scenario = EnvGuard::set("CTX_TEST_SCENARIO", "resume");
    let _guard_command_log =
        EnvGuard::set("CTX_TEST_CRP_COMMAND_LOG", &command_log.to_string_lossy());
    let _codex_home = if matches!(provider_id, "codex" | "codex-crp") {
        Some(configure_hermetic_codex_home().await)
    } else {
        None
    };

    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let script_path = common::crp_fixture_runtime::write_crp_fixture_runtime(data_dir.path());
    if provider_id == "codex" {
        common::seed_managed_codex_cli_host_runtime(data_dir.path(), &python).await;
    }
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(Tier1CrpAdapter::from_raw(
        provider_id,
        python.to_string_lossy().to_string(),
        vec![script_path.to_string_lossy().to_string()],
    ));
    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert(provider_id.to_string(), Arc::clone(&adapter));

    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:0".to_string(),
        None,
    ));
    let app = ctx_http::api::router(state.clone());

    let ws = common::create_workspace(&app, repo.path(), "ws").await;
    let task = common::create_task(&app, ws.id.0, "resume-test").await;
    let session = common::create_session(&app, task.id.0, provider_id, model_id).await;

    post_message(&app, session.id.0, "first").await;
    wait_for_done_count(&state, session.id, 1).await;

    let provider_session_ref = wait_for_provider_session_ref(&state, session.id).await;
    assert_eq!(provider_session_ref, format!("{provider_id}-thread"));
    assert_eq!(adapter.list_processes().await.len(), 1);

    let sweep_stats = adapter
        .reap_idle_sessions(ProviderSessionSweepConfig {
            idle_ttl: Duration::ZERO,
            max_idle_sessions: 0,
            interval: Duration::from_secs(60),
        })
        .await
        .unwrap();
    assert_eq!(sweep_stats.reaped, 1);
    assert!(adapter.list_processes().await.is_empty());

    post_message(&app, session.id.0, "second").await;
    wait_for_done_count(&state, session.id, 2).await;

    let open_commands = std::fs::read_to_string(&command_log)
        .unwrap()
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|value| value.get("type") == Some(&serde_json::json!("session.open")))
        .collect::<Vec<_>>();

    assert_eq!(
        open_commands.len(),
        2,
        "expected exactly two session.open commands after reaping: {open_commands:#?}"
    );
    assert_eq!(open_commands[0].get("provider_session_id"), None);
    assert_eq!(
        open_commands[1].get("provider_session_id"),
        Some(&serde_json::json!(provider_session_ref))
    );
}

#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn idle_reaped_crp_workers_resume_via_provider_session_ref() {
    let _env_lock = lock_env();

    assert_provider_session_resume_after_idle_reap("codex", "gpt-5.4/medium").await;
    assert_provider_session_resume_after_idle_reap("claude-crp", "default/medium").await;
}
