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
use ctx_providers::crp::Tier1CrpAdapter;
use ctx_store::StoreManager;

use ctx_http::api;
use ctx_http::daemon::AppState;
use ctx_managed_installs::{save_agent_server_config, AgentServerCommand, AgentServerConfigFile};
use ctx_settings_model::{
    ContainerExecutionSettings, ContainerMountMode, ContainerNetworkMode, ExecutionMode,
    ExecutionSettings, Settings,
};
use ctx_settings_service::{load_settings, save_settings};

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

fn sandbox_cli_binary_for_tests() -> Option<PathBuf> {
    if let Ok(raw) = std::env::var("CTX_HARNESS_SANDBOX_CLI_PATH") {
        let path = PathBuf::from(raw);
        if path.exists() {
            return Some(path);
        }
    }
    which::which("nerdctl").ok()
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

fn write_fake_crp_script(root: &Path) -> PathBuf {
    let script_dir = root
        .join("providers")
        .join("agent-servers")
        .join("codex")
        .join("fake");
    std::fs::create_dir_all(&script_dir).unwrap();
    let script_path = script_dir.join("fake_crp.py");
    let script = r#"
import json
import sys

next_session = 1
seq = 1

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
        session_id = msg.get("session_id")
        if not session_id:
            session_id = "sess_%d" % next_session
            next_session += 1
        send({
            "type": "session.opened",
            "session_id": session_id,
            "provider_session_id": session_id,
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

async fn load_settings_from_data_root(data_root: &Path) -> Settings {
    let db_path = data_root.join("db").join("db.sqlite");
    let store = ctx_store::Store::open_sqlite(&db_path, None).await.unwrap();
    let settings = load_settings(&store).await.unwrap();
    store.close().await;
    settings
}

async fn save_settings_to_data_root(data_root: &Path, settings: &Settings) {
    let db_path = data_root.join("db").join("db.sqlite");
    let store = ctx_store::Store::open_sqlite(&db_path, None).await.unwrap();
    save_settings(&store, settings).await.unwrap();
    store.close().await;
}

async fn configure_container_settings(
    data_root: &Path,
    mount_mode: ContainerMountMode,
    image: &str,
) {
    let settings = Settings {
        execution: Some(ExecutionSettings {
            mode: ExecutionMode::Sandbox,
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
    save_settings_to_data_root(data_root, &settings).await;
}

async fn configure_container_network_settings(
    data_root: &Path,
    mount_mode: ContainerMountMode,
    image: &str,
    network_mode: ContainerNetworkMode,
    allowlist: Vec<String>,
) {
    let settings = Settings {
        execution: Some(ExecutionSettings {
            mode: ExecutionMode::Sandbox,
            container: ContainerExecutionSettings {
                mount_mode,
                network_mode,
                allowlist,
                image: Some(image.to_string()),
                ..Default::default()
            },
        }),
        ..Default::default()
    };
    save_settings_to_data_root(data_root, &settings).await;
}

async fn run_container_python(container_name: &str, script: &str) -> std::process::Output {
    let sandbox_cli = sandbox_cli_binary_for_tests().expect("sandbox CLI required for e2e");
    Command::new(sandbox_cli)
        .arg("exec")
        .arg(container_name)
        .arg("python3")
        .arg("-c")
        .arg(script)
        .output()
        .await
        .unwrap()
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
        .body(Body::from(
            json!({
                "title": "t1",
                "default_session": {
                    "provider_id": provider_id,
                    "model_id": "fake-model"
                }
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let task: ctx_core::models::Task = serde_json::from_slice(&body).unwrap();

    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/tasks/{}/sessions", task.id.0))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let sessions: Vec<ctx_core::models::Session> = serde_json::from_slice(&body).unwrap();
    sessions
        .into_iter()
        .find(|session| Some(session.id) == task.primary_session_id)
        .expect("created task should list its default session")
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
async fn harness_container_sandbox_fake_acp() {
    if std::env::var("CTX_E2E_SANDBOX").ok().as_deref() != Some("1") {
        eprintln!("skipping: CTX_E2E_SANDBOX not set");
        return;
    }
    if sandbox_cli_binary_for_tests().is_none() {
        eprintln!("skipping: sandbox CLI not found");
        return;
    }
    let sandbox_cli = sandbox_cli_binary_for_tests().expect("sandbox CLI not found");
    let _guard = EnvGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli.to_string_lossy(),
    );

    let git_repo = setup_git_repo().await;
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();

    let script_path = write_fake_crp_script(data_dir.path());
    configure_fake_provider(data_dir.path(), &script_path).await;
    configure_container_settings(
        data_dir.path(),
        ContainerMountMode::DiskIsolated,
        "python:3.11",
    )
    .await;

    let mut providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
        HashMap::new();
    providers.insert(
        "codex".into(),
        Arc::new(Tier1CrpAdapter::from_raw(
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
async fn harness_container_sandbox_egress_allowlist() {
    let _ = tracing_subscriber::fmt::try_init();
    if std::env::var("CTX_E2E_SANDBOX").ok().as_deref() != Some("1") {
        eprintln!("skipping: CTX_E2E_SANDBOX not set");
        return;
    }
    if sandbox_cli_binary_for_tests().is_none() {
        eprintln!("skipping: sandbox CLI not found");
        return;
    }
    if std::env::var("CTX_EGRESS_PROXY_PATH").ok().is_none() {
        eprintln!("skipping: CTX_EGRESS_PROXY_PATH not set");
        return;
    }
    let sandbox_cli = sandbox_cli_binary_for_tests().expect("sandbox CLI not found");
    let _guard = EnvGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli.to_string_lossy(),
    );

    let image =
        std::env::var("CTX_E2E_SANDBOX_IMAGE").unwrap_or_else(|_| "python:3.11".to_string());
    let allow_host = "example.com";

    let git_repo = setup_git_repo().await;
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();

    let script_path = write_fake_crp_script(data_dir.path());
    configure_fake_provider(data_dir.path(), &script_path).await;
    configure_container_network_settings(
        data_dir.path(),
        ContainerMountMode::DiskIsolated,
        &image,
        ContainerNetworkMode::Allowlist,
        vec![allow_host.to_string()],
    )
    .await;
    let settings = load_settings_from_data_root(data_dir.path()).await;
    let execution_settings = settings.execution.clone().unwrap_or_default();
    assert_eq!(
        settings
            .execution
            .as_ref()
            .expect("execution settings")
            .container
            .network_mode,
        ContainerNetworkMode::Allowlist
    );

    let mut providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
        HashMap::new();
    providers.insert(
        "codex".into(),
        Arc::new(Tier1CrpAdapter::from_raw(
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
    let workspace = state
        .core
        .stores
        .global()
        .get_workspace(session.workspace_id)
        .await
        .unwrap()
        .expect("workspace");
    let workspace_store = state
        .core
        .stores
        .workspace(session.workspace_id)
        .await
        .unwrap();
    let worktree = workspace_store
        .get_worktree(session.worktree_id)
        .await
        .unwrap()
        .expect("worktree");
    state
        .execution
        .harness
        .prepare(
            &workspace,
            &worktree,
            &execution_settings,
            &state.core.daemon_url,
        )
        .await
        .expect("failed to prepare container runtime");

    let session_id = session.id.0.to_string();
    post_message(&mut app, &session_id, PROMPT).await;
    wait_for_done(&state, session.id).await;
    let container_status = state
        .execution
        .harness
        .container_status(session.workspace_id)
        .await
        .unwrap();
    assert!(
        container_status
            .as_ref()
            .and_then(|status| status.egress_guard)
            .unwrap_or(false),
        "egress guard was not configured"
    );

    let container_name = format!("ctx-harness-{}", session.workspace_id.0);
    let allow_script = r#"
import socket, ssl
host = "example.com"
ctx = ssl.create_default_context()
sock = ctx.wrap_socket(socket.socket(), server_hostname=host)
sock.settimeout(5)
sock.connect((host, 443))
sock.sendall(b"GET / HTTP/1.1\r\nHost: " + host.encode() + b"\r\nConnection: close\r\n\r\n")
sock.recv(4)
"#;
    let allow_output = run_container_python(&container_name, allow_script).await;
    assert!(
        allow_output.status.success(),
        "allowlist host failed: {}",
        String::from_utf8_lossy(&allow_output.stderr)
    );

    let deny_script = r#"
import socket, ssl, sys
host = "example.net"
ctx = ssl.create_default_context()
sock = ctx.wrap_socket(socket.socket(), server_hostname=host)
sock.settimeout(5)
try:
    sock.connect((host, 443))
    sock.sendall(b"GET / HTTP/1.1\r\nHost: " + host.encode() + b"\r\nConnection: close\r\n\r\n")
    sock.recv(4)
    sys.exit(0)
except Exception:
    sys.exit(2)
"#;
    let deny_output = run_container_python(&container_name, deny_script).await;
    assert_eq!(deny_output.status.code(), Some(2));
}

#[tokio::test]
#[ignore]
async fn harness_container_sandbox_egress_allow_all() {
    let _ = tracing_subscriber::fmt::try_init();
    if std::env::var("CTX_E2E_SANDBOX").ok().as_deref() != Some("1") {
        eprintln!("skipping: CTX_E2E_SANDBOX not set");
        return;
    }
    if sandbox_cli_binary_for_tests().is_none() {
        eprintln!("skipping: sandbox CLI not found");
        return;
    }
    if std::env::var("CTX_EGRESS_PROXY_PATH").ok().is_none() {
        eprintln!("skipping: CTX_EGRESS_PROXY_PATH not set");
        return;
    }
    let sandbox_cli = sandbox_cli_binary_for_tests().expect("sandbox CLI not found");
    let _guard = EnvGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli.to_string_lossy(),
    );

    let image =
        std::env::var("CTX_E2E_SANDBOX_IMAGE").unwrap_or_else(|_| "python:3.11".to_string());

    let git_repo = setup_git_repo().await;
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();

    let script_path = write_fake_crp_script(data_dir.path());
    configure_fake_provider(data_dir.path(), &script_path).await;
    configure_container_network_settings(
        data_dir.path(),
        ContainerMountMode::DiskIsolated,
        &image,
        ContainerNetworkMode::All,
        Vec::new(),
    )
    .await;
    let settings = load_settings_from_data_root(data_dir.path()).await;
    let execution_settings = settings.execution.clone().unwrap_or_default();

    let mut providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
        HashMap::new();
    providers.insert(
        "codex".into(),
        Arc::new(Tier1CrpAdapter::from_raw(
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
    let workspace = state
        .core
        .stores
        .global()
        .get_workspace(session.workspace_id)
        .await
        .unwrap()
        .expect("workspace");
    let workspace_store = state
        .core
        .stores
        .workspace(session.workspace_id)
        .await
        .unwrap();
    let worktree = workspace_store
        .get_worktree(session.worktree_id)
        .await
        .unwrap()
        .expect("worktree");
    state
        .execution
        .harness
        .prepare(
            &workspace,
            &worktree,
            &execution_settings,
            &state.core.daemon_url,
        )
        .await
        .expect("failed to prepare container runtime");

    let session_id = session.id.0.to_string();
    post_message(&mut app, &session_id, PROMPT).await;
    wait_for_done(&state, session.id).await;
    let container_status = state
        .execution
        .harness
        .container_status(session.workspace_id)
        .await
        .unwrap();
    assert_eq!(
        container_status
            .as_ref()
            .and_then(|status| status.egress_guard),
        Some(false),
        "egress guard should be disabled for allow-all"
    );

    let container_name = format!("ctx-harness-{}", session.workspace_id.0);
    for host in ["example.com", "example.net"] {
        let allow_script = format!(
            r#"
import socket, ssl
host = "{host}"
ctx = ssl.create_default_context()
sock = ctx.wrap_socket(socket.socket(), server_hostname=host)
sock.settimeout(5)
sock.connect((host, 443))
sock.sendall(b"GET / HTTP/1.1\r\nHost: " + host.encode() + b"\r\nConnection: close\r\n\r\n")
sock.recv(4)
"#
        );
        let allow_output = run_container_python(&container_name, &allow_script).await;
        assert!(
            allow_output.status.success(),
            "allow-all host failed ({host}): {}",
            String::from_utf8_lossy(&allow_output.stderr)
        );
    }
}

#[tokio::test]
#[ignore]
async fn harness_container_sandbox_egress_deny_all() {
    let _ = tracing_subscriber::fmt::try_init();
    if std::env::var("CTX_E2E_SANDBOX").ok().as_deref() != Some("1") {
        eprintln!("skipping: CTX_E2E_SANDBOX not set");
        return;
    }
    if sandbox_cli_binary_for_tests().is_none() {
        eprintln!("skipping: sandbox CLI not found");
        return;
    }
    if std::env::var("CTX_EGRESS_PROXY_PATH").ok().is_none() {
        eprintln!("skipping: CTX_EGRESS_PROXY_PATH not set");
        return;
    }
    let sandbox_cli = sandbox_cli_binary_for_tests().expect("sandbox CLI not found");
    let _guard = EnvGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli.to_string_lossy(),
    );

    let image =
        std::env::var("CTX_E2E_SANDBOX_IMAGE").unwrap_or_else(|_| "python:3.11".to_string());

    let git_repo = setup_git_repo().await;
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();

    let script_path = write_fake_crp_script(data_dir.path());
    configure_fake_provider(data_dir.path(), &script_path).await;
    configure_container_network_settings(
        data_dir.path(),
        ContainerMountMode::DiskIsolated,
        &image,
        ContainerNetworkMode::Allowlist,
        Vec::new(),
    )
    .await;
    let settings = load_settings_from_data_root(data_dir.path()).await;
    let execution_settings = settings.execution.clone().unwrap_or_default();

    let mut providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
        HashMap::new();
    providers.insert(
        "codex".into(),
        Arc::new(Tier1CrpAdapter::from_raw(
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
    let workspace = state
        .core
        .stores
        .global()
        .get_workspace(session.workspace_id)
        .await
        .unwrap()
        .expect("workspace");
    let workspace_store = state
        .core
        .stores
        .workspace(session.workspace_id)
        .await
        .unwrap();
    let worktree = workspace_store
        .get_worktree(session.worktree_id)
        .await
        .unwrap()
        .expect("worktree");
    state
        .execution
        .harness
        .prepare(
            &workspace,
            &worktree,
            &execution_settings,
            &state.core.daemon_url,
        )
        .await
        .expect("failed to prepare container runtime");

    let session_id = session.id.0.to_string();
    post_message(&mut app, &session_id, PROMPT).await;
    wait_for_done(&state, session.id).await;
    let container_status = state
        .execution
        .harness
        .container_status(session.workspace_id)
        .await
        .unwrap();
    assert!(
        container_status
            .as_ref()
            .and_then(|status| status.egress_guard)
            .unwrap_or(false),
        "egress guard was not configured"
    );

    let container_name = format!("ctx-harness-{}", session.workspace_id.0);
    let deny_script = r#"
import socket, ssl, sys
host = "example.com"
ctx = ssl.create_default_context()
sock = ctx.wrap_socket(socket.socket(), server_hostname=host)
sock.settimeout(5)
try:
    sock.connect((host, 443))
    sock.sendall(b"GET / HTTP/1.1\r\nHost: " + host.encode() + b"\r\nConnection: close\r\n\r\n")
    sock.recv(4)
    sys.exit(0)
except Exception:
    sys.exit(2)
"#;
    let deny_output = run_container_python(&container_name, deny_script).await;
    assert_eq!(deny_output.status.code(), Some(2));
}
