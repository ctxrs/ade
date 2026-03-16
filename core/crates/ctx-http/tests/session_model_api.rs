use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::json;

use ctx_core::models::{Session, SessionEventType, SessionHeadSnapshot};
use ctx_providers::adapters::{
    ProviderAdapter, ProviderCapabilities, ProviderHealth, ProviderProcessInfo,
    ProviderRestartMode, ProviderStatus, RunHandle, TurnInput,
};
use ctx_providers::events::NormalizedEvent;

mod common;

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
