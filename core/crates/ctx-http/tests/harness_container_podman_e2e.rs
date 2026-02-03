use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use serde_json::json;
use tokio::process::Command;
use tower::ServiceExt;

use ctx_core::models::SessionEventType;
use ctx_providers::tier1::Tier1AcpAdapter;
use ctx_store::StoreManager;

use ctx_http::api;
use ctx_http::daemon::AppState;
use ctx_http::installer::{save_agent_server_config, AgentServerCommand, AgentServerConfigFile};
use ctx_http::settings::{
    save_settings, ContainerExecutionSettings, ContainerMountMode, ContainerNetworkMode,
    ExecutionMode, ExecutionSettings, Settings,
};

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

fn podman_binary_for_tests() -> Option<PathBuf> {
    if let Ok(raw) = std::env::var("CTX_PODMAN_PATH") {
        let path = PathBuf::from(raw);
        if path.exists() {
            return Some(path);
        }
    }
    which::which("podman").ok()
}

async fn run_git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn setup_git_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    run_git(root, &["init"]).await;
    run_git(root, &["config", "user.email", "test@example.com"]).await;
    run_git(root, &["config", "user.name", "Test"]).await;
    std::fs::write(root.join("note.txt"), "hello\n").unwrap();
    run_git(root, &["add", "."]).await;
    run_git(root, &["commit", "-m", "init"]).await;
    dir
}

fn write_fake_acp_script(root: &Path) -> PathBuf {
    let script_dir = root
        .join("providers")
        .join("agent-servers")
        .join("codex")
        .join("fake");
    std::fs::create_dir_all(&script_dir).unwrap();
    let script_path = script_dir.join("fake_acp.py");
    let script = r#"
import json
import sys

next_session = 1

def send(msg):
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
    method = msg.get("method")
    req_id = msg.get("id")
    if method == "initialize":
        send({
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {
                "protocolVersion": 1,
                "agentCapabilities": {
                    "promptCapabilities": {"image": False, "embeddedContext": True},
                    "loadSession": True,
                },
            },
        })
    elif method in ("session/new", "session/load"):
        session_id = msg.get("params", {}).get("sessionId")
        if not session_id:
            session_id = "sess_%d" % next_session
            next_session += 1
        send({
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {"sessionId": session_id},
        })
    elif method == "session/prompt":
        session_id = msg.get("params", {}).get("sessionId", "sess_1")
        send({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": session_id,
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": "done"},
                },
            },
        })
        send({
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {"stopReason": "end_turn"},
        })
    elif method == "session/cancel":
        continue
"#;
    std::fs::write(&script_path, script.trim_start()).unwrap();
    script_path
}

async fn configure_fake_provider(data_root: &Path, script_path: &Path) {
    let mut cfg = AgentServerConfigFile::default();
    cfg.providers.insert(
        "codex".to_string(),
        AgentServerCommand {
            command: "python3".to_string(),
            args: vec![script_path.to_string_lossy().to_string()],
            dependencies: Vec::new(),
            managed: None,
        },
    );
    save_agent_server_config(data_root, &cfg).await.unwrap();
}

async fn configure_container_settings(
    data_root: &Path,
    mount_mode: ContainerMountMode,
    image: &str,
) {
    let settings = Settings {
        execution: Some(ExecutionSettings {
            mode: ExecutionMode::Container,
            container: ContainerExecutionSettings {
                mount_mode,
                network_mode: ContainerNetworkMode::All,
                allowlist: Vec::new(),
                image: Some(image.to_string()),
                ..Default::default()
            },
        }),
        ..Default::default()
    };
    save_settings(data_root, &settings).await.unwrap();
}

async fn inspect_container_mount_sources(container_name: &str) -> Vec<String> {
    let podman = podman_binary_for_tests().expect("podman required for e2e");
    let output = Command::new(podman)
        .arg("container")
        .arg("inspect")
        .arg("--format")
        .arg("{{json .Mounts}}")
        .arg(container_name)
        .output()
        .await
        .unwrap();
    assert!(output.status.success(), "podman inspect failed");
    let mounts: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    if let Some(array) = mounts.as_array() {
        array
            .iter()
            .filter_map(|entry| entry.get("Source").and_then(|value| value.as_str()))
            .map(|value| value.to_string())
            .collect()
    } else {
        Vec::new()
    }
}

async fn create_session_with_provider(
    app: &mut axum::Router,
    git_repo_root: &Path,
    provider_id: &str,
) -> ctx_core::models::Session {
    let req = Request::builder()
        .method("POST")
        .uri("/api/workspaces")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "root_path": git_repo_root.to_string_lossy(),
                "name": "ws"
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let ws: ctx_core::models::Workspace = serde_json::from_slice(&body).unwrap();

    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/workspaces/{}/tasks", ws.id.0))
        .header("content-type", "application/json")
        .body(Body::from(json!({"title":"t1"}).to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let task: ctx_core::models::Task = serde_json::from_slice(&body).unwrap();

    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/tasks/{}/sessions", task.id.0))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"provider_id":provider_id,"model_id":"default"}).to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

async fn post_message(app: &mut axum::Router, session_id: &str, content: &str) {
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/sessions/{session_id}/messages"))
        .header("content-type", "application/json")
        .body(Body::from(json!({ "content": content }).to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

async fn wait_for_done(state: &Arc<AppState>, session_id: ctx_core::ids::SessionId) {
    let store = state.store_for_session(session_id).await.unwrap();
    let mut attempts = 0;
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
            break;
        }
        attempts += 1;
        if attempts > 240 {
            panic!("timed out waiting for Done event");
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

const PROMPT: &str = "Reply with the exact text: done";

#[tokio::test]
#[ignore]
async fn harness_container_podman_fake_acp() {
    if std::env::var("CTX_E2E_PODMAN").ok().as_deref() != Some("1") {
        eprintln!("skipping: CTX_E2E_PODMAN not set");
        return;
    }
    if podman_binary_for_tests().is_none() {
        eprintln!("skipping: podman not found");
        return;
    }
    let _guard = EnvGuard::set("CTX_ALLOW_SYSTEM_PODMAN", "1");

    let git_repo = setup_git_repo().await;
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();

    let script_path = write_fake_acp_script(data_dir.path());
    configure_fake_provider(data_dir.path(), &script_path).await;
    configure_container_settings(
        data_dir.path(),
        ContainerMountMode::HostMounted,
        "python:3.11",
    )
    .await;

    let mut providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
        HashMap::new();
    providers.insert(
        "codex".into(),
        Arc::new(Tier1AcpAdapter::from_raw(
            "codex",
            "python3".to_string(),
            vec![script_path.to_string_lossy().to_string()],
        )),
    );

    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:4399".to_string(),
        None,
    ));
    let mut app = api::router(state.clone());
    let session = create_session_with_provider(&mut app, git_repo.path(), "codex").await;

    let session_id = session.id.0.to_string();
    post_message(&mut app, &session_id, PROMPT).await;
    wait_for_done(&state, session.id).await;
}

#[tokio::test]
#[ignore]
async fn harness_container_podman_sealed_mounts() {
    if std::env::var("CTX_E2E_PODMAN").ok().as_deref() != Some("1") {
        eprintln!("skipping: CTX_E2E_PODMAN not set");
        return;
    }
    if podman_binary_for_tests().is_none() {
        eprintln!("skipping: podman not found");
        return;
    }
    let _guard = EnvGuard::set("CTX_ALLOW_SYSTEM_PODMAN", "1");

    let git_repo = setup_git_repo().await;
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();

    let script_path = write_fake_acp_script(data_dir.path());
    configure_fake_provider(data_dir.path(), &script_path).await;
    configure_container_settings(data_dir.path(), ContainerMountMode::Sealed, "python:3.11").await;

    let mut providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
        HashMap::new();
    providers.insert(
        "codex".into(),
        Arc::new(Tier1AcpAdapter::from_raw(
            "codex",
            "python3".to_string(),
            vec![script_path.to_string_lossy().to_string()],
        )),
    );

    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:4399".to_string(),
        None,
    ));
    let mut app = api::router(state.clone());
    let session = create_session_with_provider(&mut app, git_repo.path(), "codex").await;

    let session_id = session.id.0.to_string();
    post_message(&mut app, &session_id, PROMPT).await;
    wait_for_done(&state, session.id).await;

    let container_name = format!("ctx-harness-{}", session.workspace_id.0);
    let mounts = inspect_container_mount_sources(&container_name).await;
    let workspace_root = git_repo.path().to_string_lossy().to_string();
    assert!(
        !mounts.iter().any(|source| source == &workspace_root),
        "sealed mode should not mount workspace root"
    );
    let sealed_root = data_dir
        .path()
        .join("containers")
        .join("workspaces")
        .join(session.workspace_id.0.to_string())
        .join("sealed-worktrees")
        .join(session.worktree_id.0.to_string());
    let sealed_root = sealed_root.to_string_lossy().to_string();
    assert!(
        mounts.iter().any(|source| source == &sealed_root),
        "sealed worktree should be mounted"
    );
}
