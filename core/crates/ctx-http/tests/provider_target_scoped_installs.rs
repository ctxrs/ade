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
async fn acp_container_install_surfaces_bridge_as_installable_prerequisite() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let stores = common::setup_store(data_dir.path()).await;
    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:0",
    );
    let app = common::router(state.clone());

    state.providers.statuses.lock().await.insert(
        "kimi".to_string(),
        ProviderStatus {
            provider_id: "kimi".to_string(),
            installed: false,
            detected_path: None,
            version: None,
            capabilities: None,
            health: ProviderHealth::Missing,
            diagnostics: Vec::new(),
            details: HashMap::new(),
        },
    );

    let (provider_status, provider_body): (StatusCode, serde_json::Value) = common::json_request(
        &app,
        axum::http::Method::GET,
        "/api/providers/kimi?target=container",
        None,
    )
    .await;
    assert_eq!(
        provider_status,
        StatusCode::OK,
        "provider status failed: {provider_body:#?}"
    );
    assert_eq!(
        provider_body
            .pointer("/details/install_supported")
            .and_then(serde_json::Value::as_str),
        Some("true"),
        "container ACP installs should remain supported when the bridge is installable: {provider_body:#?}"
    );
    assert!(
        provider_body
            .pointer("/details/install_blocked_code")
            .is_none(),
        "installable bridge prerequisites must not be surfaced as blocked: {provider_body:#?}"
    );

    let (providers_status, providers_body): (StatusCode, serde_json::Value) = common::json_request(
        &app,
        axum::http::Method::GET,
        "/api/providers?target=container",
        None,
    )
    .await;
    assert_eq!(
        providers_status,
        StatusCode::OK,
        "providers list failed: {providers_body:#?}"
    );
    let kimi_status = providers_body
        .as_array()
        .and_then(|providers| {
            providers.iter().find(|provider| {
                provider
                    .get("provider_id")
                    .and_then(serde_json::Value::as_str)
                    == Some("kimi")
            })
        })
        .cloned()
        .expect("kimi must appear in provider list");
    assert_eq!(
        kimi_status
            .pointer("/details/install_supported")
            .and_then(serde_json::Value::as_str),
        Some("true"),
        "providers list should advertise installable ACP container targets: {kimi_status:#?}"
    );
    assert!(
        kimi_status.pointer("/details/install_blocked_code").is_none(),
        "providers list must not mark installable ACP bridge prerequisites as blocked: {kimi_status:#?}"
    );
}

#[tokio::test]
async fn acp_container_install_is_blocked_before_start_when_bridge_runtime_is_invalid() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let stores = common::setup_store(data_dir.path()).await;
    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:0",
    );
    let app = common::router(state.clone());

    save_invalid_container_bridge_runtime(data_dir.path()).await;

    state.providers.statuses.lock().await.insert(
        "kimi".to_string(),
        ProviderStatus {
            provider_id: "kimi".to_string(),
            installed: false,
            detected_path: None,
            version: None,
            capabilities: None,
            health: ProviderHealth::Missing,
            diagnostics: Vec::new(),
            details: HashMap::new(),
        },
    );

    let (provider_status, provider_body): (StatusCode, serde_json::Value) = common::json_request(
        &app,
        axum::http::Method::GET,
        "/api/providers/kimi?target=container",
        None,
    )
    .await;
    assert_eq!(
        provider_status,
        StatusCode::OK,
        "provider status failed: {provider_body:#?}"
    );
    assert_eq!(
        provider_body
            .pointer("/details/install_supported")
            .and_then(serde_json::Value::as_str),
        Some("false"),
        "invalid bridge config must suppress container ACP installs: {provider_body:#?}"
    );
    assert_eq!(
        provider_body
            .pointer("/details/install_blocked_code")
            .and_then(serde_json::Value::as_str),
        Some("acp_bridge_invalid"),
        "expected explicit install blocker code: {provider_body:#?}"
    );
    assert_eq!(
        provider_body
            .pointer("/details/error_code")
            .and_then(serde_json::Value::as_str),
        Some("acp_bridge_invalid"),
        "provider status should preserve invalid bridge classification: {provider_body:#?}"
    );
    assert!(
        provider_body
            .get("diagnostics")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|diagnostics| diagnostics.iter().any(|value| {
                value
                    .as_str()
                    .is_some_and(|text| text.contains("invalid runtime command for acp-crp-bridge"))
            })),
        "provider diagnostics should describe the invalid bridge runtime: {provider_body:#?}"
    );

    let (install_status, install_body): (StatusCode, serde_json::Value) = common::json_request(
        &app,
        axum::http::Method::POST,
        "/api/providers/kimi/install?target=container",
        None,
    )
    .await;
    assert_eq!(
        install_status,
        StatusCode::BAD_REQUEST,
        "install should fail before start when bridge is invalid: {install_body:#?}"
    );
    assert_eq!(
        install_body.get("code").and_then(serde_json::Value::as_str),
        Some("acp_bridge_invalid"),
        "expected explicit install failure code: {install_body:#?}"
    );
    assert!(
        install_body
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| value.contains("ACP bridge runtime")),
        "expected explicit bridge contract error: {install_body:#?}"
    );
    assert!(
        state
            .find_running_install("kimi", Some(ctx_http::installs::InstallTarget::Container))
            .await
            .is_none(),
        "no install should start when bridge contract is invalid"
    );

    let cfg = load_agent_server_config(data_dir.path())
        .await
        .expect("load agent server config");
    assert!(
        cfg.managed_provider_targets.get("kimi").is_none(),
        "failed preflight must not write partial provider install state"
    );
    assert!(
        cfg.managed_install_targets.get("kimi").is_none(),
        "failed preflight must not write partial install metadata"
    );
}

#[tokio::test]
async fn acp_container_install_happy_path_installs_bridge_prerequisite_and_keeps_registry_entries()
{
    let data_dir = tempfile::tempdir().expect("tempdir");
    let fixture_dir = data_dir.path().join("fixtures");
    std::fs::create_dir_all(&fixture_dir).expect("create fixture dir");
    let bridge_fixture = fixture_dir.join("acp-crp-bridge");
    let provider_fixture = fixture_dir.join("kimi-acp");
    write_executable(&bridge_fixture, "#!/bin/sh\nexit 0\n");
    write_executable(&provider_fixture, "#!/bin/sh\nexit 0\n");
    save_matrix_fixture(
        data_dir.path(),
        &provider_fixture_matrix(file_url(&bridge_fixture), file_url(&provider_fixture)),
    )
    .await;

    let stores = common::setup_store(data_dir.path()).await;
    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:0",
    );
    let app = common::router(state.clone());

    let (install_status, install_body): (StatusCode, serde_json::Value) = common::json_request(
        &app,
        axum::http::Method::POST,
        "/api/providers/kimi/install?target=container",
        None,
    )
    .await;
    assert_eq!(
        install_status,
        StatusCode::OK,
        "install should start successfully: {install_body:#?}"
    );
    let install_id = install_body
        .get("install_id")
        .and_then(serde_json::Value::as_str)
        .and_then(|raw| raw.parse::<InstallId>().ok())
        .expect("install id");

    let install_info = wait_for_install_completion(&state, install_id).await;
    assert!(
        matches!(install_info.state, InstallStateKind::Succeeded),
        "kimi install should succeed with bridge prerequisite: {install_info:#?}"
    );

    let installs = state.providers.installs.lock().await;
    let bridge_install = installs
        .iter()
        .find_map(|(id, install)| {
            (install.provider_id == "acp-crp-bridge"
                && install.target == Some(InstallTarget::Container))
            .then(|| install.info(*id))
        })
        .expect("bridge prerequisite install entry");
    assert!(
        matches!(bridge_install.state, InstallStateKind::Succeeded),
        "bridge prerequisite install should be tracked and succeed: {bridge_install:#?}"
    );
    drop(installs);

    let cfg = load_agent_server_config(data_dir.path())
        .await
        .expect("load agent server config");
    assert!(
        cfg.managed_provider_targets
            .get("acp-crp-bridge")
            .and_then(|targets| targets.get("container"))
            .is_some(),
        "bridge target-scoped runtime command should remain registered"
    );
    assert!(
        cfg.managed_provider_targets
            .get("kimi")
            .and_then(|targets| targets.get("container"))
            .is_some(),
        "provider target-scoped runtime command should remain registered"
    );
    assert!(
        cfg.managed_install_targets
            .get("acp-crp-bridge")
            .and_then(|targets| targets.get("container"))
            .is_some(),
        "bridge target-scoped install metadata should remain registered"
    );
    assert!(
        cfg.managed_install_targets
            .get("kimi")
            .and_then(|targets| targets.get("container"))
            .is_some(),
        "provider target-scoped install metadata should remain registered"
    );

    let reloaded_stores = common::setup_store(data_dir.path()).await;
    let reloaded_state = common::build_state(
        data_dir.path().to_path_buf(),
        reloaded_stores,
        HashMap::new(),
        "http://127.0.0.1:0",
    );
    let reloaded_app = common::router(reloaded_state);

    let (provider_status, provider_body): (StatusCode, serde_json::Value) = common::json_request(
        &reloaded_app,
        axum::http::Method::GET,
        "/api/providers/kimi?target=container",
        None,
    )
    .await;
    assert_eq!(
        provider_status,
        StatusCode::OK,
        "provider status failed after install: {provider_body:#?}"
    );
    assert_eq!(
        provider_body
            .get("installed")
            .and_then(serde_json::Value::as_bool),
        Some(true),
        "provider should be installed after happy-path bridge prerequisite install: {provider_body:#?}"
    );
    assert_eq!(
        provider_body
            .pointer("/details/managed_target")
            .and_then(serde_json::Value::as_str),
        Some("container"),
        "provider should keep its container managed-target record: {provider_body:#?}"
    );
}

#[tokio::test]
async fn acp_container_install_parent_polling_stays_bounded_while_bridge_prerequisite_runs() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let fixture_dir = data_dir.path().join("fixtures");
    std::fs::create_dir_all(&fixture_dir).expect("create fixture dir");
    let bridge_fixture = fixture_dir.join("acp-crp-bridge");
    let provider_fixture = fixture_dir.join("kimi-acp");
    write_executable(&bridge_fixture, "#!/bin/sh\nsleep 1.6\nexit 0\n");
    write_executable(&provider_fixture, "#!/bin/sh\nexit 0\n");
    let download_server = spawn_download_fixture_server(vec![
        (
            "bridge",
            std::fs::read(&bridge_fixture).expect("read bridge fixture"),
            1_600,
        ),
        (
            "provider",
            std::fs::read(&provider_fixture).expect("read provider fixture"),
            0,
        ),
    ])
    .await;
    save_matrix_fixture(
        data_dir.path(),
        &provider_fixture_matrix(
            fixture_download_url(&download_server, "bridge"),
            fixture_download_url(&download_server, "provider"),
        ),
    )
    .await;

    let stores = common::setup_store(data_dir.path()).await;
    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:0",
    );
    let app = common::router(state.clone());

    let (install_status, install_body): (StatusCode, serde_json::Value) = common::json_request(
        &app,
        axum::http::Method::POST,
        "/api/providers/kimi/install?target=container",
        None,
    )
    .await;
    assert_eq!(
        install_status,
        StatusCode::OK,
        "kimi install should start successfully: {install_body:#?}"
    );
    let install_id = install_body
        .get("install_id")
        .and_then(serde_json::Value::as_str)
        .and_then(|raw| raw.parse::<InstallId>().ok())
        .expect("install id");

    tokio::time::sleep(Duration::from_millis(950)).await;
    let polled_info = get_install_info_api(&app, install_id).await;
    assert!(
        matches!(polled_info.state, InstallStateKind::Running),
        "install should still be running while the bridge prerequisite is active: {polled_info:#?}"
    );
    assert_eq!(
        polled_info
            .last_event
            .as_ref()
            .map(|event| event.stage.as_str()),
        Some("start"),
        "parent poll surface should keep prerequisite progress visible without copying the child high-water stage: {polled_info:#?}"
    );
    assert_eq!(
        compute_polled_install_pct(&polled_info, None),
        Some(2),
        "workbench/settings polling should observe bounded parent progress while the bridge prerequisite runs: {polled_info:#?}"
    );
    assert!(
        polled_info
            .last_event
            .as_ref()
            .is_some_and(|event| event.message.contains("Prerequisite acp-crp-bridge")),
        "parent poll should still expose prerequisite bridge activity: {polled_info:#?}"
    );

    let parent_events = get_install_events_api(&app, install_id).await;
    assert!(
        parent_events.iter().any(|event| {
            event.message.contains("Prerequisite acp-crp-bridge")
                && event.stage == "start"
                && event.message.contains("stage")
        }),
        "parent install events should preserve prerequisite visibility via the real API surface: {parent_events:#?}"
    );
    assert!(
        parent_events
            .iter()
            .filter(|event| event.message.contains("Prerequisite acp-crp-bridge"))
            .all(|event| event.stage == "start"),
        "mirrored prerequisite events must stay on the bounded parent stage instead of copying child high-water stages: {parent_events:#?}"
    );

    let install_info = wait_for_install_completion(&state, install_id).await;
    assert!(
        matches!(install_info.state, InstallStateKind::Succeeded),
        "kimi install should succeed after the bridge prerequisite finishes: {install_info:#?}"
    );
}

#[tokio::test]
async fn acp_container_install_joins_existing_bridge_install_and_surfaces_short_prerequisites_to_polling(
) {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let fixture_dir = data_dir.path().join("fixtures");
    std::fs::create_dir_all(&fixture_dir).expect("create fixture dir");
    let bridge_fixture = fixture_dir.join("acp-crp-bridge");
    let provider_fixture = fixture_dir.join("kimi-acp");
    write_executable(&bridge_fixture, "#!/bin/sh\nsleep 0.2\nexit 0\n");
    write_executable(&provider_fixture, "#!/bin/sh\nexit 0\n");
    let download_server = spawn_download_fixture_server(vec![
        (
            "bridge",
            std::fs::read(&bridge_fixture).expect("read bridge fixture"),
            200,
        ),
        (
            "provider",
            std::fs::read(&provider_fixture).expect("read provider fixture"),
            3_000,
        ),
    ])
    .await;
    save_matrix_fixture(
        data_dir.path(),
        &provider_fixture_matrix(
            fixture_download_url(&download_server, "bridge"),
            fixture_download_url(&download_server, "provider"),
        ),
    )
    .await;

    let stores = common::setup_store(data_dir.path()).await;
    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:0",
    );
    let app = common::router(state.clone());

    let (bridge_status, bridge_body): (StatusCode, serde_json::Value) = common::json_request(
        &app,
        axum::http::Method::POST,
        "/api/providers/acp-crp-bridge/install?target=container",
        None,
    )
    .await;
    assert_eq!(
        bridge_status,
        StatusCode::OK,
        "bridge install should start successfully: {bridge_body:#?}"
    );
    let bridge_install_id = bridge_body
        .get("install_id")
        .and_then(serde_json::Value::as_str)
        .and_then(|raw| raw.parse::<InstallId>().ok())
        .expect("bridge install id");
    let bridge_poll_deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        let bridge_info = get_install_info_api(&app, bridge_install_id).await;
        if bridge_info.last_event.is_some() {
            break;
        }
        assert!(
            tokio::time::Instant::now() < bridge_poll_deadline,
            "timed out waiting for bridge install to expose running progress: {bridge_info:#?}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    let (install_status, install_body): (StatusCode, serde_json::Value) = common::json_request(
        &app,
        axum::http::Method::POST,
        "/api/providers/kimi/install?target=container",
        None,
    )
    .await;
    assert_eq!(
        install_status,
        StatusCode::OK,
        "kimi install should join the running bridge prerequisite: {install_body:#?}"
    );
    let install_id = install_body
        .get("install_id")
        .and_then(serde_json::Value::as_str)
        .and_then(|raw| raw.parse::<InstallId>().ok())
        .expect("install id");

    tokio::time::sleep(Duration::from_millis(950)).await;
    let reader_a = app.clone();
    let reader_b = app.clone();
    let (polled_info_a, polled_info_b) = tokio::join!(
        get_install_info_api(&reader_a, install_id),
        get_install_info_api(&reader_b, install_id)
    );
    for polled_info in [&polled_info_a, &polled_info_b] {
        assert!(
            matches!(polled_info.state, InstallStateKind::Running),
            "install should still be running on the first polling tick: {polled_info:#?}"
        );
        assert_eq!(
            polled_info
                .last_event
                .as_ref()
                .map(|event| event.stage.as_str()),
            Some("start"),
            "short prerequisite installs must still leave a bounded visible parent stage on the first poll: {polled_info:#?}"
        );
        assert_eq!(
            compute_polled_install_pct(polled_info, None),
            Some(2),
            "short bridge prerequisites should remain visible across the shipped polling cadence without overstating progress: {polled_info:#?}"
        );
        assert!(
            polled_info
                .last_event
                .as_ref()
                .is_some_and(|event| {
                    event.message.contains(&format!(
                        "Prerequisite acp-crp-bridge (install {bridge_install_id}"
                    ))
                }),
            "the first poll should still be showing prerequisite-derived progress, not a rewritten parent event: {polled_info:#?}"
        );
    }
    assert_eq!(
        polled_info_a
            .last_event
            .as_ref()
            .map(|event| (event.stage.clone(), event.message.clone())),
        polled_info_b
            .last_event
            .as_ref()
            .map(|event| (event.stage.clone(), event.message.clone())),
        "concurrent pollers should observe the same prerequisite-derived first visible state: {polled_info_a:#?} vs {polled_info_b:#?}"
    );

    let parent_events = get_install_events_api(&app, install_id).await;
    assert!(
        parent_events.iter().any(|event| {
            event.message.contains(&format!(
                "Prerequisite acp-crp-bridge (install {bridge_install_id}"
            )) && event.stage == "start"
        }),
        "parent install events should retain short prerequisite visibility on the real API surface: {parent_events:#?}"
    );

    let parent_owned_poll_info = {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        loop {
            let info = get_install_info_api(&app, install_id).await;
            if info
                .last_event
                .as_ref()
                .is_some_and(|event| !event.message.starts_with("Prerequisite "))
            {
                break info;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for parent-owned running progress on the poll surface: {info:#?}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    };
    assert!(
        matches!(parent_owned_poll_info.state, InstallStateKind::Running),
        "the parent poll surface should switch off prerequisite-derived progress before install completion: {parent_owned_poll_info:#?}"
    );
    assert!(
        parent_owned_poll_info
            .last_event
            .as_ref()
            .is_some_and(|event| !event.message.starts_with("Prerequisite ")),
        "once the prerequisite override window expires, polling should surface the parent install's own work: {parent_owned_poll_info:#?}"
    );
    assert!(
        matches!(
            parent_owned_poll_info
                .last_event
                .as_ref()
                .map(|event| event.stage.as_str()),
            Some("start") | Some("download")
        ),
        "the next poll after the prerequisite window should expose the parent install's own early running stage instead of staying on prerequisite progress: {parent_owned_poll_info:#?}"
    );

    let install_info = wait_for_install_completion(&state, install_id).await;
    assert!(
        matches!(install_info.state, InstallStateKind::Succeeded),
        "kimi install should succeed after joining the bridge prerequisite: {install_info:#?}"
    );
    let final_parent_events = get_install_events_api(&app, install_id).await;
    assert!(
        final_parent_events.iter().any(|event| {
            !event.message.starts_with("Prerequisite ") && event.stage == "download"
        }),
        "the parent install event history should still record the parent-owned download stage after the prerequisite handoff: {final_parent_events:#?}"
    );

    let bridge_info = state
        .get_install_info(bridge_install_id)
        .await
        .expect("missing bridge install info");
    assert!(
        matches!(bridge_info.state, InstallStateKind::Succeeded),
        "bridge prerequisite install should remain tracked as succeeded: {bridge_info:#?}"
    );

    let installs = state.providers.installs.lock().await;
    let bridge_install_ids = installs
        .iter()
        .filter_map(|(id, install)| {
            (install.provider_id == "acp-crp-bridge"
                && install.target == Some(InstallTarget::Container))
            .then_some(*id)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        bridge_install_ids,
        vec![bridge_install_id],
        "joining ACP installs must reuse the same tracked bridge install id"
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
