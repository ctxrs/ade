use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use tower::ServiceExt;

use ctx_core::models::{SessionEventType, SessionTurnStatus};
use ctx_http::daemon::AppState;
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

    fn remove(key: &'static str) -> Self {
        let prev = std::env::var(key).ok();
        std::env::remove_var(key);
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

fn write_fake_codex_crp_script(root: &Path) -> PathBuf {
    let script_dir = root
        .join("providers")
        .join("agent-servers")
        .join("codex-crp")
        .join("fake");
    std::fs::create_dir_all(&script_dir).unwrap();
    let script_path = script_dir.join("fake_codex_crp.py");

    // This CRP runtime simulates Codex-style resume semantics that depend on stable
    // CODEX_HOME contents:
    // - If provider_session_id is provided and marker exists in CODEX_HOME, reuse it.
    // - Otherwise create a new provider_session_id and persist a marker.
    let script = r#"
import json
import os
import sys
import uuid

seq = 1
provider_session_id = None

CODEX_HOME = os.environ.get("CODEX_HOME")
if not CODEX_HOME:
    raise RuntimeError("missing CODEX_HOME")

MARKERS_DIR = os.path.join(CODEX_HOME, "rollouts")
os.makedirs(MARKERS_DIR, exist_ok=True)

def marker_path(thread_id):
    safe = thread_id.replace("/", "_")
    return os.path.join(MARKERS_DIR, safe + ".marker")

def send(msg):
    global seq
    msg["seq"] = seq
    seq += 1
    msg.setdefault("channel", "control")
    sys.stdout.write(json.dumps(msg))
    sys.stdout.write("\n")
    sys.stdout.flush()

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        msg = json.loads(line)
    except Exception:
        continue
    command_type = msg.get("type")
    if command_type == "session.open":
        session_id = msg.get("session_id") or "sess_1"
        requested = msg.get("provider_session_id")
        if requested and os.path.exists(marker_path(requested)):
            provider_session_id = requested
            send({
                "type": "session.notice",
                "session_id": session_id,
                "code": "resume_hit",
                "details": {"codex_home": CODEX_HOME},
            })
        else:
            provider_session_id = "thread-" + uuid.uuid4().hex
            with open(marker_path(provider_session_id), "w") as f:
                f.write("ok")
            send({
                "type": "session.notice",
                "session_id": session_id,
                "code": "resume_miss",
                "details": {"codex_home": CODEX_HOME},
            })
        send({
            "type": "session.opened",
            "session_id": session_id,
            "provider_session_id": provider_session_id,
        })
    elif command_type == "session.prompt":
        session_id = msg.get("session_id") or "sess_1"
        turn_id = msg.get("turn_id") or "turn_1"
        send({
            "type": "turn.started",
            "session_id": session_id,
            "turn_id": turn_id,
        })
        send({
            "type": "message.final",
            "session_id": session_id,
            "turn_id": turn_id,
            "message_id": "msg_1",
            "content": "done",
        })
        send({
            "type": "turn.completed",
            "session_id": session_id,
            "turn_id": turn_id,
            "status": "success",
        })
    elif command_type == "models.list":
        send({
            "type": "models.list",
            "models": [{"id": "fake-model"}],
            "current_model_id": "fake-model",
        })
"#;

    std::fs::write(&script_path, script.trim()).unwrap();
    script_path
}

fn python_binary() -> Option<PathBuf> {
    which::which("python3")
        .or_else(|_| which::which("python"))
        .ok()
}

fn build_providers(
    python: &Path,
    script_path: &Path,
) -> HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> {
    let adapter: Arc<Tier1CrpAdapter> = Arc::new(Tier1CrpAdapter::from_raw(
        "codex-crp",
        python.to_string_lossy().to_string(),
        vec![script_path.to_string_lossy().to_string()],
    ));
    let mut providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
        HashMap::new();
    providers.insert("codex-crp".to_string(), adapter);
    providers
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

fn is_terminal_turn(status: &SessionTurnStatus) -> bool {
    matches!(
        status,
        SessionTurnStatus::Completed | SessionTurnStatus::Failed | SessionTurnStatus::Interrupted
    )
}

async fn wait_for_terminal_turn_count(
    state: &Arc<AppState>,
    session_id: ctx_core::ids::SessionId,
    expected_terminal_turns: usize,
) {
    let store = state.store_for_session(session_id).await.unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    loop {
        // Prefer turn status over `Done` events: providers may emit `TurnFinished` without `Done`,
        // and status is what gates subsequent turns.
        let turns = store
            .list_session_turns_page_by_seq(session_id, None, Some(50))
            .await
            .unwrap();
        let terminal_turns = turns.iter().filter(|t| is_terminal_turn(&t.status)).count();
        if terminal_turns >= expected_terminal_turns {
            if turns
                .iter()
                .any(|t| matches!(t.status, SessionTurnStatus::Failed))
            {
                panic!("saw Failed turn status: {turns:#?}");
            }
            if turns
                .iter()
                .any(|t| matches!(t.status, SessionTurnStatus::Interrupted))
            {
                panic!("saw Interrupted turn status: {turns:#?}");
            }
            let events = store.list_session_events(session_id).await.unwrap();
            if events
                .iter()
                .any(|e| matches!(e.event_type, SessionEventType::Error))
            {
                panic!("saw Error event(s): {events:#?}");
            }
            break;
        }

        let events = store.list_session_events(session_id).await.unwrap();
        if events
            .iter()
            .any(|e| matches!(e.event_type, SessionEventType::Error))
        {
            panic!("saw Error event(s) before terminal turns: {events:#?}");
        }
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "timed out waiting for {expected_terminal_turns} terminal turn(s): turns={turns:#?} events={events:#?}"
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn assert_no_session_gap(events: &[ctx_core::models::SessionEvent]) {
    let saw_gap = events.iter().any(|event| {
        matches!(event.event_type, SessionEventType::Notice)
            && event
                .payload_json
                .get("kind")
                .and_then(|value| value.as_str())
                == Some("session_gap")
    });
    assert!(!saw_gap, "unexpected session_gap event: {events:#?}");
}

fn assert_saw_notice_kind(events: &[ctx_core::models::SessionEvent], expected_kind: &str) {
    let saw = events.iter().any(|event| {
        matches!(event.event_type, SessionEventType::Notice)
            && event
                .payload_json
                .get("kind")
                .and_then(|value| value.as_str())
                == Some(expected_kind)
    });
    assert!(
        saw,
        "expected Notice(kind={expected_kind}) event; saw: {events:#?}"
    );
}

#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn codex_crp_resume_across_restart_keeps_provider_session_ref() {
    let _env_lock = lock_env();
    let repo = common::init_git_repo(&[("note.txt", "hello\n")]).await;
    let data_dir = tempfile::tempdir().unwrap();
    let _guard_codex_home = EnvGuard::remove("CODEX_HOME");
    let codex_home = tempfile::tempdir().unwrap();
    let _guard = EnvGuard::set("CTX_CODEX_HOME", &codex_home.path().to_string_lossy());

    let Some(python) = python_binary() else {
        eprintln!("skipping: python3/python not found");
        return;
    };

    let script_path = write_fake_codex_crp_script(data_dir.path());

    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        build_providers(&python, &script_path),
        "http://127.0.0.1:0".to_string(),
        None,
    ));
    let app = ctx_http::api::router(state.clone());

    let ws = common::create_workspace(&app, repo.path(), "ws").await;
    let task = common::create_task(&app, ws.id.0, "t1").await;
    let session = common::create_session(&app, task.id.0, "codex-crp", "fake-model").await;

    post_message(&app, session.id.0, "first").await;
    wait_for_terminal_turn_count(&state, session.id, 1).await;

    let store = state.store_for_session(session.id).await.unwrap();
    let stored = store.get_session(session.id).await.unwrap().unwrap();
    let provider_ref1 = stored
        .provider_session_ref
        .clone()
        .expect("expected provider session ref");
    assert!(provider_ref1.starts_with("thread-"));

    let events = store.list_session_events(session.id).await.unwrap();
    assert_no_session_gap(&events);
    assert_saw_notice_kind(&events, "resume_miss");

    drop(app);
    drop(state);

    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let state2 = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        build_providers(&python, &script_path),
        "http://127.0.0.1:0".to_string(),
        None,
    ));
    let app2 = ctx_http::api::router(state2.clone());

    post_message(&app2, session.id.0, "second").await;
    wait_for_terminal_turn_count(&state2, session.id, 2).await;

    let store2 = state2.store_for_session(session.id).await.unwrap();
    let stored2 = store2.get_session(session.id).await.unwrap().unwrap();
    let provider_ref2 = stored2
        .provider_session_ref
        .clone()
        .expect("expected provider session ref after restart");

    assert_eq!(provider_ref2, provider_ref1);

    let events2 = store2.list_session_events(session.id).await.unwrap();
    assert_no_session_gap(&events2);
    assert_saw_notice_kind(&events2, "resume_hit");
}

#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn codex_crp_resume_breaks_if_codex_home_changes() {
    let _env_lock = lock_env();
    let repo = common::init_git_repo(&[("note.txt", "hello\n")]).await;
    let data_dir = tempfile::tempdir().unwrap();
    let _guard_codex_home = EnvGuard::remove("CODEX_HOME");

    let Some(python) = python_binary() else {
        eprintln!("skipping: python3/python not found");
        return;
    };

    let script_path = write_fake_codex_crp_script(data_dir.path());

    let codex_home1 = tempfile::tempdir().unwrap();
    let guard1 = EnvGuard::set("CTX_CODEX_HOME", &codex_home1.path().to_string_lossy());

    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        build_providers(&python, &script_path),
        "http://127.0.0.1:0".to_string(),
        None,
    ));
    let app = ctx_http::api::router(state.clone());

    let ws = common::create_workspace(&app, repo.path(), "ws").await;
    let task = common::create_task(&app, ws.id.0, "t1").await;
    let session = common::create_session(&app, task.id.0, "codex-crp", "fake-model").await;

    post_message(&app, session.id.0, "first").await;
    wait_for_terminal_turn_count(&state, session.id, 1).await;

    let store = state.store_for_session(session.id).await.unwrap();
    let stored = store.get_session(session.id).await.unwrap().unwrap();
    let provider_ref1 = stored
        .provider_session_ref
        .clone()
        .expect("expected provider session ref");

    drop(app);
    drop(state);
    drop(guard1);

    let codex_home2 = tempfile::tempdir().unwrap();
    let _guard2 = EnvGuard::set("CTX_CODEX_HOME", &codex_home2.path().to_string_lossy());

    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let state2 = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        build_providers(&python, &script_path),
        "http://127.0.0.1:0".to_string(),
        None,
    ));
    let app2 = ctx_http::api::router(state2.clone());

    post_message(&app2, session.id.0, "second").await;
    wait_for_terminal_turn_count(&state2, session.id, 2).await;

    let store2 = state2.store_for_session(session.id).await.unwrap();
    let stored2 = store2.get_session(session.id).await.unwrap().unwrap();
    let provider_ref2 = stored2
        .provider_session_ref
        .clone()
        .expect("expected provider session ref after restart");

    assert_ne!(
        provider_ref2, provider_ref1,
        "expected provider session ref to rotate when CODEX_HOME changes"
    );
}

#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn codex_crp_resume_survives_ctx_default_codex_home() {
    let _env_lock = lock_env();
    let repo = common::init_git_repo(&[("note.txt", "hello\n")]).await;
    let data_dir = tempfile::tempdir().unwrap();
    let _guard_codex_home = EnvGuard::remove("CODEX_HOME");
    let _guard = EnvGuard::remove("CTX_CODEX_HOME");

    let Some(python) = python_binary() else {
        eprintln!("skipping: python3/python not found");
        return;
    };

    let script_path = write_fake_codex_crp_script(data_dir.path());

    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        build_providers(&python, &script_path),
        "http://127.0.0.1:0".to_string(),
        None,
    ));
    let app = ctx_http::api::router(state.clone());

    let ws = common::create_workspace(&app, repo.path(), "ws").await;
    let task = common::create_task(&app, ws.id.0, "t1").await;
    let session = common::create_session(&app, task.id.0, "codex-crp", "fake-model").await;

    post_message(&app, session.id.0, "first").await;
    wait_for_terminal_turn_count(&state, session.id, 1).await;

    let store = state.store_for_session(session.id).await.unwrap();
    let stored = store.get_session(session.id).await.unwrap().unwrap();
    let provider_ref1 = stored
        .provider_session_ref
        .clone()
        .expect("expected provider session ref");

    let codex_home = ctx_http::provider_accounts::codex_fallback_home(data_dir.path());
    let marker = codex_home
        .join("rollouts")
        .join(format!("{provider_ref1}.marker"));
    assert!(
        marker.exists(),
        "expected provider state persisted under ctx fallback CODEX_HOME at {marker:?}"
    );

    drop(app);
    drop(state);

    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let state2 = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        build_providers(&python, &script_path),
        "http://127.0.0.1:0".to_string(),
        None,
    ));
    let app2 = ctx_http::api::router(state2.clone());

    post_message(&app2, session.id.0, "second").await;
    wait_for_terminal_turn_count(&state2, session.id, 2).await;

    let store2 = state2.store_for_session(session.id).await.unwrap();
    let stored2 = store2.get_session(session.id).await.unwrap().unwrap();
    let provider_ref2 = stored2
        .provider_session_ref
        .clone()
        .expect("expected provider session ref after restart");

    assert_eq!(provider_ref2, provider_ref1);
}
