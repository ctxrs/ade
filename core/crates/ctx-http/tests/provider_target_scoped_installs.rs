#![cfg(unix)]

mod common;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use ctx_core::models::SessionEventType;
use ctx_http::daemon::AppState;
use ctx_http::installer::{
    refresh_provider_statuses, save_agent_server_config, AgentServerCommand, AgentServerConfigFile,
    ManagedInstallMetadata,
};
use ctx_http::settings::{
    save_settings, ContainerExecutionSettings, ContainerMountMode, ContainerNetworkMode,
    ExecutionMode, ExecutionSettings, Settings,
};
use ctx_providers::adapters::ProviderAdapter;
use ctx_providers::crp::Tier1CrpAdapter;
use ctx_store::Store;

struct SeededRuntime {
    host_command: String,
}

struct EnvVarGuard {
    key: &'static str,
    previous: Option<String>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(value) = self.previous.take() {
            std::env::set_var(self.key, value);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

fn write_executable(path: &Path, contents: &str) {
    use std::os::unix::fs::PermissionsExt;

    std::fs::write(path, contents).expect("write executable");
    let mut perms = std::fs::metadata(path).expect("metadata").permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).expect("set permissions");
}

fn write_fake_node_runtime(path: &Path, tag: &str) {
    let script = format!(
        r#"#!/bin/sh
set -eu
extract_field() {{
  printf '%s\n' "$1" | sed -n "s/.*\"$2\":\"\\([^\"]*\\)\".*/\\1/p"
}}
while IFS= read -r line; do
  case "$line" in
    *'"type":"models.list"'*)
      printf '{{"seq":1,"channel":"control","type":"models.list","models":[{{"id":"{tag}-model"}}],"current_model_id":"{tag}-model"}}\n'
      ;;
    *'"type":"session.open"'*)
      session_id="$(extract_field "$line" session_id)"
      if [ -z "$session_id" ]; then
        session_id="sess_{tag}"
      fi
      printf '{{"seq":2,"channel":"control","type":"session.opened","session_id":"%s","provider_session_id":"{tag}-provider"}}\n' "$session_id"
      ;;
    *'"type":"session.prompt"'*)
      session_id="$(extract_field "$line" session_id)"
      turn_id="$(extract_field "$line" turn_id)"
      if [ -z "$session_id" ]; then
        session_id="sess_{tag}"
      fi
      if [ -z "$turn_id" ]; then
        turn_id="turn_{tag}"
      fi
      printf '{{"seq":3,"channel":"control","type":"turn.started","session_id":"%s","turn_id":"%s"}}\n' "$session_id" "$turn_id"
      printf '{{"seq":4,"channel":"data","type":"message.final","session_id":"%s","turn_id":"%s","message_id":"msg_{tag}","content":"{tag}-runtime"}}\n' "$session_id" "$turn_id"
      printf '{{"seq":5,"channel":"control","type":"turn.completed","session_id":"%s","turn_id":"%s","status":"success"}}\n' "$session_id" "$turn_id"
      exit 0
      ;;
  esac
done
"#
    );
    write_executable(path, &script);
}

fn write_js_entrypoint(path: &Path) {
    write_executable(path, "#!/usr/bin/env node\n// fixture runtime\n");
}

async fn seed_target_scoped_codex_runtime(data_root: &Path) -> SeededRuntime {
    let host_bin_rel = "providers/runtimes/runtime-node-host/bin";
    let container_bin_rel = "providers/runtimes/runtime-node-container/bin";
    let host_bin_dir = data_root.join(host_bin_rel);
    let container_bin_dir = data_root.join(container_bin_rel);
    std::fs::create_dir_all(&host_bin_dir).expect("create host bin dir");
    std::fs::create_dir_all(&container_bin_dir).expect("create container bin dir");
    write_fake_node_runtime(&host_bin_dir.join("node"), "host");
    write_fake_node_runtime(&container_bin_dir.join("node"), "container");

    let host_install_rel = "providers/agent-servers/codex/host-fixture/bin/codex.js";
    let container_install_rel = "providers/agent-servers/codex/container-fixture/bin/codex.js";
    let host_command_path = data_root.join(host_install_rel);
    let container_command_path = data_root.join(container_install_rel);
    std::fs::create_dir_all(host_command_path.parent().expect("host parent"))
        .expect("create host runtime dir");
    std::fs::create_dir_all(container_command_path.parent().expect("container parent"))
        .expect("create container runtime dir");
    write_js_entrypoint(&host_command_path);
    write_js_entrypoint(&container_command_path);

    let mut cfg = AgentServerConfigFile::default();
    cfg.managed_provider_targets.insert(
        "codex".to_string(),
        HashMap::from([
            (
                "host".to_string(),
                AgentServerCommand {
                    command: host_command_path.to_string_lossy().to_string(),
                    args: Vec::new(),
                    dependencies: vec!["runtime-node-host".to_string()],
                    managed: Some(ManagedInstallMetadata {
                        package: Some("@openai/codex".to_string()),
                        version: Some("1.0.0-host".to_string()),
                        target: Some(ctx_http::installs::InstallTarget::Host),
                        install_dir_rel: Some(
                            "providers/agent-servers/codex/host-fixture".to_string(),
                        ),
                        bin_dir_rel: Some(
                            "providers/agent-servers/codex/host-fixture/bin".to_string(),
                        ),
                        last_success_at: None,
                        last_error: None,
                    }),
                },
            ),
            (
                "container".to_string(),
                AgentServerCommand {
                    command: container_command_path.to_string_lossy().to_string(),
                    args: Vec::new(),
                    dependencies: vec!["runtime-node-container".to_string()],
                    managed: Some(ManagedInstallMetadata {
                        package: Some("@openai/codex".to_string()),
                        version: Some("1.0.0-container".to_string()),
                        target: Some(ctx_http::installs::InstallTarget::Container),
                        install_dir_rel: Some(
                            "providers/agent-servers/codex/container-fixture".to_string(),
                        ),
                        bin_dir_rel: Some(
                            "providers/agent-servers/codex/container-fixture/bin".to_string(),
                        ),
                        last_success_at: None,
                        last_error: None,
                    }),
                },
            ),
        ]),
    );
    cfg.managed_install_targets.insert(
        "codex".to_string(),
        HashMap::from([
            (
                "host".to_string(),
                ManagedInstallMetadata {
                    package: Some("@openai/codex".to_string()),
                    version: Some("1.0.0-host".to_string()),
                    target: Some(ctx_http::installs::InstallTarget::Host),
                    install_dir_rel: Some("providers/agent-servers/codex/host-fixture".to_string()),
                    bin_dir_rel: Some("providers/agent-servers/codex/host-fixture/bin".to_string()),
                    last_success_at: None,
                    last_error: None,
                },
            ),
            (
                "container".to_string(),
                ManagedInstallMetadata {
                    package: Some("@openai/codex".to_string()),
                    version: Some("1.0.0-container".to_string()),
                    target: Some(ctx_http::installs::InstallTarget::Container),
                    install_dir_rel: Some(
                        "providers/agent-servers/codex/container-fixture".to_string(),
                    ),
                    bin_dir_rel: Some(
                        "providers/agent-servers/codex/container-fixture/bin".to_string(),
                    ),
                    last_success_at: None,
                    last_error: None,
                },
            ),
        ]),
    );
    cfg.managed_installs.insert(
        "runtime-node-host".to_string(),
        ManagedInstallMetadata {
            package: Some("node-runtime".to_string()),
            version: Some("24.14.0".to_string()),
            target: Some(ctx_http::installs::InstallTarget::Host),
            install_dir_rel: Some("providers/runtimes/runtime-node-host".to_string()),
            bin_dir_rel: Some(host_bin_rel.to_string()),
            last_success_at: None,
            last_error: None,
        },
    );
    cfg.managed_installs.insert(
        "runtime-node-container".to_string(),
        ManagedInstallMetadata {
            package: Some("node-runtime".to_string()),
            version: Some("24.14.0".to_string()),
            target: Some(ctx_http::installs::InstallTarget::Container),
            install_dir_rel: Some("providers/runtimes/runtime-node-container".to_string()),
            bin_dir_rel: Some(container_bin_rel.to_string()),
            last_success_at: None,
            last_error: None,
        },
    );
    save_agent_server_config(data_root, &cfg)
        .await
        .expect("save agent config");

    SeededRuntime {
        host_command: host_command_path.to_string_lossy().to_string(),
    }
}

async fn build_state_with_host_codex(data_root: &Path, host_command: &str) -> Arc<AppState> {
    let stores = common::setup_store(data_root).await;
    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert(
        "codex".to_string(),
        Arc::new(Tier1CrpAdapter::from_raw(
            "codex",
            host_command.to_string(),
            Vec::new(),
        )),
    );
    let state = common::build_state(
        data_root.to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:0",
    );
    refresh_provider_statuses(state.as_ref())
        .await
        .expect("refresh provider statuses");
    state
}

async fn save_settings_to_data_root(data_root: &Path, settings: &Settings) {
    let db_path = data_root.join("db").join("db.sqlite");
    let store = Store::open_sqlite(&db_path, None)
        .await
        .expect("open settings store");
    save_settings(&store, settings)
        .await
        .expect("save settings");
    store.close().await;
}

async fn configure_container_image_defaults(data_root: &Path) {
    let settings = Settings {
        execution: Some(ExecutionSettings {
            mode: ExecutionMode::Host,
            container: ContainerExecutionSettings {
                mount_mode: ContainerMountMode::HostMounted,
                network_mode: ContainerNetworkMode::All,
                allowlist: Vec::new(),
                image: Some("python:3.11".to_string()),
                ..Default::default()
            },
        }),
        ..Default::default()
    };
    save_settings_to_data_root(data_root, &settings).await;
}

async fn set_workspace_container_execution(
    app: &axum::Router,
    workspace_id: uuid::Uuid,
    environment: &str,
) {
    let (status, body): (StatusCode, serde_json::Value) = common::json_request(
        app,
        axum::http::Method::POST,
        format!("/api/workspaces/{workspace_id}/execution_config"),
        Some(serde_json::json!({
            "environment": environment,
            "network_mode": "all",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "execution config failed: {body:#?}");
}

async fn post_message(app: &axum::Router, session_id: uuid::Uuid, content: &str) {
    let (status, body): (StatusCode, serde_json::Value) = common::json_request(
        app,
        axum::http::Method::POST,
        format!("/api/sessions/{session_id}/messages"),
        Some(serde_json::json!({ "content": content })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "message post failed: {body:#?}");
}

async fn wait_for_done(state: &Arc<AppState>, session_id: ctx_core::ids::SessionId) {
    let store = state.store_for_session(session_id).await.expect("store");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
    loop {
        let events = store.list_session_events(session_id).await.expect("events");
        if events
            .iter()
            .any(|event| matches!(event.event_type, SessionEventType::Done))
        {
            assert!(
                !events
                    .iter()
                    .any(|event| matches!(event.event_type, SessionEventType::Error)),
                "unexpected error events: {events:#?}"
            );
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for done event: {events:#?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn assert_assistant_message_contains(events: &[ctx_core::models::SessionEvent], expected: &str) {
    assert!(
        events.iter().any(|event| {
            matches!(event.event_type, SessionEventType::AssistantMessageInserted)
                && event
                    .payload_json
                    .get("content")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|content| content.contains(expected))
        }),
        "expected assistant message to contain {expected:?}: {events:#?}"
    );
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

async fn podman_ready(podman: &Path) -> bool {
    tokio::process::Command::new(podman)
        .arg("version")
        .output()
        .await
        .ok()
        .is_some_and(|output| output.status.success())
}

async fn podman_has_image(podman: &Path, image: &str) -> bool {
    tokio::process::Command::new(podman)
        .args(["image", "exists", image])
        .output()
        .await
        .ok()
        .is_some_and(|output| output.status.success())
}

#[tokio::test]
async fn provider_status_http_keeps_host_and_container_installs_independent() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let runtime = seed_target_scoped_codex_runtime(data_dir.path()).await;
    configure_container_image_defaults(data_dir.path()).await;
    let state = build_state_with_host_codex(data_dir.path(), &runtime.host_command).await;
    let app = common::router(state.clone());

    let repo = common::init_git_repo(&[("note.txt", "hello\n")]).await;
    let ws = common::create_workspace(&app, repo.path(), "host-ws").await;

    let (host_status, host_body): (StatusCode, serde_json::Value) = common::json_request(
        &app,
        axum::http::Method::GET,
        "/api/providers/codex?target=host",
        None,
    )
    .await;
    assert_eq!(
        host_status,
        StatusCode::OK,
        "host provider failed: {host_body:#?}"
    );
    assert_eq!(
        host_body
            .get("installed")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        host_body
            .pointer("/details/managed_target")
            .and_then(serde_json::Value::as_str),
        Some("host")
    );
    assert_eq!(
        host_body
            .pointer("/details/managed_version")
            .and_then(serde_json::Value::as_str),
        Some("1.0.0-host")
    );

    let (container_status, container_body): (StatusCode, serde_json::Value) = common::json_request(
        &app,
        axum::http::Method::GET,
        "/api/providers/codex?target=container",
        None,
    )
    .await;
    assert_eq!(
        container_status,
        StatusCode::OK,
        "container provider failed: {container_body:#?}"
    );
    assert_eq!(
        container_body
            .get("installed")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        container_body
            .pointer("/details/managed_target")
            .and_then(serde_json::Value::as_str),
        Some("container")
    );
    assert_eq!(
        container_body
            .pointer("/details/managed_version")
            .and_then(serde_json::Value::as_str),
        Some("1.0.0-container")
    );

    let (options_status, options_body): (StatusCode, serde_json::Value) = common::json_request(
        &app,
        axum::http::Method::GET,
        format!("/api/workspaces/{}/providers/codex/options", ws.id.0),
        None,
    )
    .await;
    assert_eq!(
        options_status,
        StatusCode::OK,
        "host options request failed: {options_body:#?}"
    );
    assert_eq!(
        options_body
            .pointer("/models/current_model_id")
            .and_then(serde_json::Value::as_str),
        Some("host-model")
    );
}

#[tokio::test]
#[ignore]
async fn provider_target_scoped_installs_work_for_host_and_container_workspaces() {
    if std::env::var("CTX_E2E_PODMAN").ok().as_deref() != Some("1") {
        eprintln!("skipping: CTX_E2E_PODMAN not set");
        return;
    }
    if podman_binary_for_tests().is_none() {
        eprintln!("skipping: podman not available");
        return;
    }
    let podman = podman_binary_for_tests().expect("podman not available");
    let _podman_path = EnvVarGuard::set("CTX_PODMAN_PATH", &podman.to_string_lossy());
    if !podman_ready(&podman).await {
        eprintln!("skipping: podman connection is not ready");
        return;
    }
    if !podman_has_image(&podman, "python:3.11").await {
        eprintln!("skipping: python:3.11 image is not present locally in podman");
        return;
    }

    let data_dir = tempfile::tempdir().expect("tempdir");
    let runtime = seed_target_scoped_codex_runtime(data_dir.path()).await;
    configure_container_image_defaults(data_dir.path()).await;
    let state = build_state_with_host_codex(data_dir.path(), &runtime.host_command).await;
    let app = common::router(state.clone());

    let host_repo = common::init_git_repo(&[("note.txt", "host\n")]).await;
    let container_repo = common::init_git_repo(&[("note.txt", "container\n")]).await;

    let host_ws = common::create_workspace(&app, host_repo.path(), "host-ws").await;
    let container_ws = common::create_workspace(&app, container_repo.path(), "container-ws").await;
    set_workspace_container_execution(&app, container_ws.id.0, "container_host_mounted").await;

    let (host_options_status, host_options): (StatusCode, serde_json::Value) =
        common::json_request(
            &app,
            axum::http::Method::GET,
            format!("/api/workspaces/{}/providers/codex/options", host_ws.id.0),
            None,
        )
        .await;
    assert_eq!(
        host_options_status,
        StatusCode::OK,
        "host options failed: {host_options:#?}"
    );
    assert_eq!(
        host_options
            .pointer("/models/current_model_id")
            .and_then(serde_json::Value::as_str),
        Some("host-model")
    );

    let (container_options_status, container_options): (StatusCode, serde_json::Value) =
        common::json_request(
            &app,
            axum::http::Method::GET,
            format!(
                "/api/workspaces/{}/providers/codex/options",
                container_ws.id.0
            ),
            None,
        )
        .await;
    assert_eq!(
        container_options_status,
        StatusCode::OK,
        "container options failed: {container_options:#?}"
    );
    assert_eq!(
        container_options
            .pointer("/models/current_model_id")
            .and_then(serde_json::Value::as_str),
        Some("container-model"),
        "unexpected container options body: {container_options:#?}"
    );

    let host_task = common::create_task(&app, host_ws.id.0, "host-task").await;
    let container_task = common::create_task(&app, container_ws.id.0, "container-task").await;
    let host_session = common::create_session(&app, host_task.id.0, "codex", "host-model").await;
    let container_session =
        common::create_session(&app, container_task.id.0, "codex", "container-model").await;

    post_message(&app, host_session.id.0, "reply exactly once").await;
    post_message(&app, container_session.id.0, "reply exactly once").await;
    wait_for_done(&state, host_session.id).await;
    wait_for_done(&state, container_session.id).await;

    let store = state
        .store_for_session(host_session.id)
        .await
        .expect("host store");
    let host_events = store
        .list_session_events(host_session.id)
        .await
        .expect("host events");
    assert_assistant_message_contains(&host_events, "host-runtime");

    let store = state
        .store_for_session(container_session.id)
        .await
        .expect("container store");
    let container_events = store
        .list_session_events(container_session.id)
        .await
        .expect("container events");
    assert_assistant_message_contains(&container_events, "container-runtime");
}
