use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::json;
use tokio::sync::mpsc;

use ctx_core::models::{Session, SessionEventType, SessionHeadSnapshot};
use ctx_providers::adapters::{
    ProviderAdapter, ProviderCapabilities, ProviderHealth, ProviderProcessInfo,
    ProviderRestartMode, ProviderStatus, RunHandle, TurnInput,
};
use ctx_providers::crp::Tier1CrpAdapter;
use ctx_providers::events::NormalizedEvent;

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

#[derive(Default)]
struct RecordingSetModelAdapter {
    calls: Mutex<Vec<(String, String)>>,
    live_session: bool,
    failure: Option<String>,
}

impl RecordingSetModelAdapter {
    fn live() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            live_session: true,
            failure: None,
        }
    }

    fn live_failing(message: &str) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            live_session: true,
            failure: Some(message.to_string()),
        }
    }
}

#[async_trait]
impl ProviderAdapter for RecordingSetModelAdapter {
    async fn inspect(&self) -> anyhow::Result<ProviderStatus> {
        Ok(ProviderStatus {
            provider_id: "fake-set-model".to_string(),
            installed: true,
            detected_path: None,
            version: Some("test".to_string()),
            capabilities: Some(ProviderCapabilities {
                stream_events: false,
                stream_format: "none".to_string(),
                has_turn_boundaries: false,
                has_tool_call_ids: false,
                has_file_change_events: false,
                has_command_events: false,
                supports_resume: false,
                supports_stable_session_id: true,
                supports_fork_or_rewind: false,
                supports_headless: true,
                supports_server_mode: false,
                supports_interactive_tui: false,
                supports_private_state_dir: false,
                supports_sandbox_flags: false,
                supports_approval_flags: false,
                notes: vec![],
            }),
            health: ProviderHealth::Ok,
            diagnostics: vec![],
            details: HashMap::new(),
            usability: ctx_providers::adapters::ProviderUsability::default(),
        })
    }

    async fn run(
        &self,
        _input: TurnInput,
        _workdir: PathBuf,
        _env: HashMap<String, String>,
        _event_sink: tokio::sync::mpsc::Sender<NormalizedEvent>,
    ) -> anyhow::Result<RunHandle> {
        anyhow::bail!("test adapter does not implement run");
    }

    async fn cancel(&self, _handle: RunHandle) -> anyhow::Result<()> {
        Ok(())
    }

    async fn list_processes(&self) -> Vec<ProviderProcessInfo> {
        Vec::new()
    }

    async fn restart(&self, _reason: &str, _mode: ProviderRestartMode) -> anyhow::Result<()> {
        Ok(())
    }

    async fn has_live_session(&self, _session_key: &str) -> bool {
        self.live_session
    }

    async fn set_session_model(&self, session_key: String, model_id: String) -> anyhow::Result<()> {
        if let Some(message) = self.failure.as_deref() {
            anyhow::bail!("{message}");
        }
        self.calls
            .lock()
            .expect("recording calls")
            .push((session_key, model_id));
        Ok(())
    }
}

#[tokio::test]
async fn set_session_model_updates_session_and_appends_init_event() {
    let repo = common::init_git_repo(&[("file.txt", "hello\n")]).await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let stores = common::setup_store(data_dir.path()).await;
    let adapter = Arc::new(RecordingSetModelAdapter::live());
    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert("fake-set-model".to_string(), adapter.clone());

    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:0",
    );
    let app = common::router(state);
    let server = common::spawn_http_server(app).await;
    let base = &server.base_url;
    let client = &server.client;

    let workspace: ctx_core::models::Workspace = client
        .post(format!("{base}/api/workspaces"))
        .json(&json!({"root_path": repo.path(), "name": "ws"}))
        .send()
        .await
        .expect("create workspace")
        .json()
        .await
        .expect("workspace json");

    let task: ctx_core::models::Task = client
        .post(format!("{base}/api/workspaces/{}/tasks", workspace.id.0))
        .json(&json!({"title":"session-model"}))
        .send()
        .await
        .expect("create task")
        .json()
        .await
        .expect("task json");

    let session: Session = client
        .post(format!("{base}/api/tasks/{}/sessions", task.id.0))
        .json(&json!({"provider_id":"fake-set-model","model_id":"start-model"}))
        .send()
        .await
        .expect("create session")
        .json()
        .await
        .expect("session json");

    let updated: Session = client
        .post(format!("{base}/api/sessions/{}/model", session.id.0))
        .json(&json!({"model_id":"next-model"}))
        .send()
        .await
        .expect("set session model")
        .json()
        .await
        .expect("updated session json");

    assert_eq!(updated.model_id, "next-model");
    assert_eq!(
        adapter.calls.lock().expect("calls").as_slice(),
        &[(session.id.0.to_string(), "next-model".to_string())]
    );

    let head: SessionHeadSnapshot = client
        .get(format!(
            "{base}/api/sessions/{}/head?limit=10&include_events=true",
            session.id.0
        ))
        .send()
        .await
        .expect("get session head")
        .json()
        .await
        .expect("session head json");

    let init_event = head
        .events
        .iter()
        .rev()
        .find(|event| matches!(event.event_type, SessionEventType::Init))
        .expect("init event appended");
    assert_eq!(
        init_event.payload_json.get("current_model_id"),
        Some(&json!("next-model"))
    );
}

#[tokio::test]
async fn set_session_model_skips_adapter_when_session_is_not_live() {
    let repo = common::init_git_repo(&[("file.txt", "hello\n")]).await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let stores = common::setup_store(data_dir.path()).await;
    let adapter = Arc::new(RecordingSetModelAdapter::default());
    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert("fake-set-model".to_string(), adapter.clone());

    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:0",
    );
    let app = common::router(state);
    let server = common::spawn_http_server(app).await;
    let base = &server.base_url;
    let client = &server.client;

    let workspace: ctx_core::models::Workspace = client
        .post(format!("{base}/api/workspaces"))
        .json(&json!({"root_path": repo.path(), "name": "ws"}))
        .send()
        .await
        .expect("create workspace")
        .json()
        .await
        .expect("workspace json");

    let task: ctx_core::models::Task = client
        .post(format!("{base}/api/workspaces/{}/tasks", workspace.id.0))
        .json(&json!({"title":"session-model"}))
        .send()
        .await
        .expect("create task")
        .json()
        .await
        .expect("task json");

    let session: Session = client
        .post(format!("{base}/api/tasks/{}/sessions", task.id.0))
        .json(&json!({"provider_id":"fake-set-model","model_id":"start-model"}))
        .send()
        .await
        .expect("create session")
        .json()
        .await
        .expect("session json");

    let updated: Session = client
        .post(format!("{base}/api/sessions/{}/model", session.id.0))
        .json(&json!({"model_id":"queued-model"}))
        .send()
        .await
        .expect("set session model")
        .json()
        .await
        .expect("updated session json");

    assert_eq!(updated.model_id, "queued-model");
    assert!(
        adapter.calls.lock().expect("calls").is_empty(),
        "non-live sessions should update stored model without adapter forwarding"
    );

    let head: SessionHeadSnapshot = client
        .get(format!(
            "{base}/api/sessions/{}/head?limit=10&include_events=true",
            session.id.0
        ))
        .send()
        .await
        .expect("get session head")
        .json()
        .await
        .expect("session head json");

    let init_event = head
        .events
        .iter()
        .rev()
        .find(|event| matches!(event.event_type, SessionEventType::Init))
        .expect("init event appended");
    assert_eq!(
        init_event.payload_json.get("current_model_id"),
        Some(&json!("queued-model"))
    );
}

#[tokio::test]
async fn set_session_model_returns_structured_error_when_live_switch_fails() {
    let repo = common::init_git_repo(&[("file.txt", "hello\n")]).await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let stores = common::setup_store(data_dir.path()).await;
    let adapter = Arc::new(RecordingSetModelAdapter::live_failing(
        "timed out waiting for session model update",
    ));
    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert("fake-set-model".to_string(), adapter);

    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:0",
    );
    let app = common::router(state);
    let server = common::spawn_http_server(app).await;
    let base = &server.base_url;
    let client = &server.client;

    let workspace: ctx_core::models::Workspace = client
        .post(format!("{base}/api/workspaces"))
        .json(&json!({"root_path": repo.path(), "name": "ws"}))
        .send()
        .await
        .expect("create workspace")
        .json()
        .await
        .expect("workspace json");

    let task: ctx_core::models::Task = client
        .post(format!("{base}/api/workspaces/{}/tasks", workspace.id.0))
        .json(&json!({"title":"session-model"}))
        .send()
        .await
        .expect("create task")
        .json()
        .await
        .expect("task json");

    let session: Session = client
        .post(format!("{base}/api/tasks/{}/sessions", task.id.0))
        .json(&json!({"provider_id":"fake-set-model","model_id":"start-model"}))
        .send()
        .await
        .expect("create session")
        .json()
        .await
        .expect("session json");

    let response = client
        .post(format!("{base}/api/sessions/{}/model", session.id.0))
        .json(&json!({"model_id":"next-model"}))
        .send()
        .await
        .expect("set session model");

    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let payload: serde_json::Value = response.json().await.expect("error payload");
    let error = payload
        .get("error")
        .and_then(|value| value.as_str())
        .expect("error string");
    assert!(
        error.contains("failed to switch the live fake-set-model session"),
        "unexpected error: {error}"
    );
    assert!(
        error.contains("timed out waiting for session model update"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn create_session_splits_legacy_combined_model_id_into_reasoning_effort() {
    let repo = common::init_git_repo(&[("file.txt", "hello\n")]).await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let stores = common::setup_store(data_dir.path()).await;
    let adapter = Arc::new(RecordingSetModelAdapter::live());
    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert("fake-set-model".to_string(), adapter);

    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:0",
    );
    let app = common::router(state);
    let server = common::spawn_http_server(app).await;
    let base = &server.base_url;
    let client = &server.client;

    let workspace: ctx_core::models::Workspace = client
        .post(format!("{base}/api/workspaces"))
        .json(&json!({"root_path": repo.path(), "name": "ws"}))
        .send()
        .await
        .expect("create workspace")
        .json()
        .await
        .expect("workspace json");

    let task: ctx_core::models::Task = client
        .post(format!("{base}/api/workspaces/{}/tasks", workspace.id.0))
        .json(&json!({"title":"session-model"}))
        .send()
        .await
        .expect("create task")
        .json()
        .await
        .expect("task json");

    let session: Session = client
        .post(format!("{base}/api/tasks/{}/sessions", task.id.0))
        .json(&json!({
            "provider_id":"fake-set-model",
            "model_id":"gpt-5/xhigh"
        }))
        .send()
        .await
        .expect("create session")
        .json()
        .await
        .expect("session json");

    assert_eq!(session.model_id, "gpt-5");
    assert_eq!(session.reasoning_effort.as_deref(), Some("xhigh"));
}

#[tokio::test]
async fn set_session_model_persists_reasoning_effort_and_forwards_full_model_id() {
    let repo = common::init_git_repo(&[("file.txt", "hello\n")]).await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let stores = common::setup_store(data_dir.path()).await;
    let adapter = Arc::new(RecordingSetModelAdapter::live());
    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert("fake-set-model".to_string(), adapter.clone());

    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:0",
    );
    let app = common::router(state);
    let server = common::spawn_http_server(app).await;
    let base = &server.base_url;
    let client = &server.client;

    let workspace: ctx_core::models::Workspace = client
        .post(format!("{base}/api/workspaces"))
        .json(&json!({"root_path": repo.path(), "name": "ws"}))
        .send()
        .await
        .expect("create workspace")
        .json()
        .await
        .expect("workspace json");

    let task: ctx_core::models::Task = client
        .post(format!("{base}/api/workspaces/{}/tasks", workspace.id.0))
        .json(&json!({"title":"session-model"}))
        .send()
        .await
        .expect("create task")
        .json()
        .await
        .expect("task json");

    let session: Session = client
        .post(format!("{base}/api/tasks/{}/sessions", task.id.0))
        .json(&json!({"provider_id":"fake-set-model","model_id":"start-model"}))
        .send()
        .await
        .expect("create session")
        .json()
        .await
        .expect("session json");

    let updated: Session = client
        .post(format!("{base}/api/sessions/{}/model", session.id.0))
        .json(&json!({
            "model_id":"gpt-5",
            "reasoning_effort":"xhigh"
        }))
        .send()
        .await
        .expect("set session model")
        .json()
        .await
        .expect("updated session json");

    assert_eq!(updated.model_id, "gpt-5");
    assert_eq!(updated.reasoning_effort.as_deref(), Some("xhigh"));
    assert_eq!(
        adapter.calls.lock().expect("calls").as_slice(),
        &[(session.id.0.to_string(), "gpt-5/xhigh".to_string())]
    );

    let head: SessionHeadSnapshot = client
        .get(format!(
            "{base}/api/sessions/{}/head?limit=10&include_events=true",
            session.id.0
        ))
        .send()
        .await
        .expect("get session head")
        .json()
        .await
        .expect("session head json");

    let init_event = head
        .events
        .iter()
        .rev()
        .find(|event| matches!(event.event_type, SessionEventType::Init))
        .expect("init event appended");
    assert_eq!(
        init_event.payload_json.get("current_model_id"),
        Some(&json!("gpt-5/xhigh"))
    );
    assert_eq!(
        init_event.payload_json.get("reasoning_effort"),
        Some(&json!("xhigh"))
    );
}

async fn assert_live_crp_session_model_switch_case(
    provider_id: &str,
    initial_model_id: &str,
    initial_reasoning_effort: &str,
    next_model_id: &str,
    next_reasoning_effort: &str,
) {
    let repo = common::init_git_repo(&[("file.txt", "hello\n")]).await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let fixtures_dir = tempfile::tempdir().expect("fixtures tempdir");
    let fixture_provider_dir = fixtures_dir.path().join(provider_id);
    std::fs::create_dir_all(&fixture_provider_dir).expect("create fixture provider dir");
    std::fs::write(
        fixture_provider_dir.join("basic.json"),
        serde_json::to_vec(&json!({
            "current_model_id": format!("{initial_model_id}/{initial_reasoning_effort}"),
            "models": [
                {
                    "id": format!("{initial_model_id}/{initial_reasoning_effort}"),
                    "name": format!("{provider_id} initial")
                },
                {
                    "id": format!("{next_model_id}/{next_reasoning_effort}"),
                    "name": format!("{provider_id} next")
                }
            ],
            "turns": [{}]
        }))
        .expect("fixture json"),
    )
    .expect("write fixture");
    let _fixture_root = EnvGuard::set(
        "CTX_TEST_FIXTURES_DIR",
        &fixtures_dir.path().to_string_lossy(),
    );
    let _fixture_scenario = EnvGuard::set("CTX_TEST_SCENARIO", "basic");

    let python = common::crp_fixture_runtime::python_binary().expect("python available");
    let script_path = common::crp_fixture_runtime::write_crp_fixture_runtime(data_dir.path());
    let adapter: Arc<Tier1CrpAdapter> = Arc::new(Tier1CrpAdapter::from_raw(
        provider_id,
        python.to_string_lossy().to_string(),
        vec![script_path.to_string_lossy().to_string()],
    ));
    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert(provider_id.to_string(), adapter.clone());

    let stores = common::setup_store(data_dir.path()).await;
    let global_store = stores.global().clone();
    let workspace = global_store
        .create_workspace(
            "ws".to_string(),
            repo.path().to_string_lossy().to_string(),
            ctx_core::models::VcsKind::Git,
        )
        .await
        .expect("create workspace");
    let store = stores
        .workspace(workspace.id)
        .await
        .expect("open workspace store");
    let worktree = store
        .create_worktree(
            workspace.id,
            repo.path().to_string_lossy().to_string(),
            "test-base".to_string(),
            None,
        )
        .await
        .expect("create worktree");
    global_store
        .upsert_workspace_worktree_index(worktree.id, workspace.id)
        .await
        .expect("index worktree");
    let task = store
        .create_task(workspace.id, "session-model".to_string(), None)
        .await
        .expect("create task");
    global_store
        .upsert_workspace_task_index(task.id, workspace.id)
        .await
        .expect("index task");
    let session = store
        .create_session_with_reasoning_effort(
            task.id,
            workspace.id,
            worktree.id,
            ctx_core::models::ExecutionEnvironment::Host,
            provider_id.to_string(),
            initial_model_id.to_string(),
            Some(initial_reasoning_effort.to_string()),
            "assistant".to_string(),
            None,
            None,
            None,
        )
        .await
        .expect("create session");
    global_store
        .upsert_workspace_session_index(session.id, workspace.id)
        .await
        .expect("index session");
    store
        .set_task_primary_session(task.id, session.id, worktree.id)
        .await
        .expect("set task primary session");

    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:0",
    );
    state.providers.options_cache.lock().await.insert(
        format!("{}/host/{provider_id}", workspace.id.0),
        ctx_http::daemon::CachedProviderOptions {
            cached_at: std::time::Instant::now(),
            value: json!({
                "models": {
                    "models": [
                        {
                            "id": format!("{initial_model_id}/{initial_reasoning_effort}"),
                            "name": format!("{provider_id} initial")
                        },
                        {
                            "id": format!("{next_model_id}/{next_reasoning_effort}"),
                            "name": format!("{provider_id} next")
                        }
                    ],
                    "current_model_id": format!("{initial_model_id}/{initial_reasoning_effort}"),
                    "meta": {
                        "source_kind": "subscription",
                        "refresh_pending": false
                    }
                }
            }),
        },
    );
    let app = common::router(state);
    let server = common::spawn_http_server(app).await;
    let base = &server.base_url;
    let client = &server.client;

    let (event_tx, _event_rx) = mpsc::channel::<NormalizedEvent>(8);
    adapter
        .authenticate_session(
            session.id.0.to_string(),
            repo.path().to_path_buf(),
            HashMap::new(),
            None,
            event_tx,
        )
        .await
        .expect("open live CRP session");
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        adapter.has_live_session(&session.id.0.to_string()).await,
        "expected adapter to track live session after authenticate"
    );

    let updated: Session = client
        .post(format!("{base}/api/sessions/{}/model", session.id.0))
        .json(&json!({
            "model_id": next_model_id,
            "reasoning_effort": next_reasoning_effort
        }))
        .send()
        .await
        .expect("set session model")
        .json()
        .await
        .expect("updated session json");

    assert_eq!(updated.model_id, next_model_id);
    assert_eq!(
        updated.reasoning_effort.as_deref(),
        Some(next_reasoning_effort)
    );

    let head: SessionHeadSnapshot = client
        .get(format!(
            "{base}/api/sessions/{}/head?limit=50&include_events=true",
            session.id.0
        ))
        .send()
        .await
        .expect("get session head")
        .json()
        .await
        .expect("session head json");

    let init_event = head
        .events
        .iter()
        .rev()
        .find(|event| matches!(event.event_type, SessionEventType::Init))
        .expect("init event appended");
    assert_eq!(
        init_event.payload_json.get("current_model_id"),
        Some(&json!(format!("{next_model_id}/{next_reasoning_effort}")))
    );
    assert_eq!(
        init_event.payload_json.get("reasoning_effort"),
        Some(&json!(next_reasoning_effort))
    );
}

#[tokio::test]
async fn live_crp_supported_harnesses_session_model_switch_succeeds_end_to_end() {
    let _env_lock = lock_env();

    assert_live_crp_session_model_switch_case("codex", "gpt-5.4", "medium", "gpt-5.4", "xhigh")
        .await;
    assert_live_crp_session_model_switch_case("claude-crp", "default", "medium", "default", "high")
        .await;
}

#[tokio::test]
async fn live_acp_runtime_catalog_harnesses_session_model_switch_succeeds_end_to_end() {
    let _env_lock = lock_env();

    for provider_id in ["amp", "copilot", "cursor", "gemini", "kimi", "qwen"] {
        assert_live_crp_session_model_switch_case(
            provider_id,
            "test-model",
            "medium",
            "test-model",
            "xhigh",
        )
        .await;
    }
}

#[tokio::test]
async fn set_session_model_allows_explicit_model_outside_cached_catalog() {
    let repo = common::init_git_repo(&[("file.txt", "hello\n")]).await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let stores = common::setup_store(data_dir.path()).await;
    let adapter = Arc::new(RecordingSetModelAdapter::live());
    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert("fake-set-model".to_string(), adapter.clone());

    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:0",
    );
    let app = common::router(state.clone());
    let server = common::spawn_http_server(app).await;
    let base = &server.base_url;
    let client = &server.client;

    let workspace: ctx_core::models::Workspace = client
        .post(format!("{base}/api/workspaces"))
        .json(&json!({"root_path": repo.path(), "name": "ws"}))
        .send()
        .await
        .expect("create workspace")
        .json()
        .await
        .expect("workspace json");

    state.providers.options_cache.lock().await.insert(
        format!("{}/host/fake-set-model", workspace.id.0),
        ctx_http::daemon::CachedProviderOptions {
            cached_at: std::time::Instant::now(),
            value: json!({
                "models": {
                    "models": [
                        { "id": "known-model" }
                    ],
                    "current_model_id": "known-model",
                    "meta": {
                        "source_kind": "subscription",
                        "refresh_pending": false
                    }
                }
            }),
        },
    );

    let task: ctx_core::models::Task = client
        .post(format!("{base}/api/workspaces/{}/tasks", workspace.id.0))
        .json(&json!({"title":"session-model"}))
        .send()
        .await
        .expect("create task")
        .json()
        .await
        .expect("task json");

    let session: Session = client
        .post(format!("{base}/api/tasks/{}/sessions", task.id.0))
        .json(&json!({"provider_id":"fake-set-model","model_id":"known-model"}))
        .send()
        .await
        .expect("create session")
        .json()
        .await
        .expect("session json");

    let updated: Session = client
        .post(format!("{base}/api/sessions/{}/model", session.id.0))
        .json(&json!({"model_id":"unknown-model"}))
        .send()
        .await
        .expect("set session model")
        .json()
        .await
        .expect("updated session json");

    assert_eq!(updated.model_id, "unknown-model");
    assert_eq!(
        adapter.calls.lock().expect("calls").as_slice(),
        &[(session.id.0.to_string(), "unknown-model".to_string())]
    );
}
