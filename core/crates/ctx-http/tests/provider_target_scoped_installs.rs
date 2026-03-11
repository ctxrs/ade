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
    load_agent_server_config, refresh_provider_statuses, save_agent_server_config,
    AgentServerCommand, AgentServerConfigFile, ManagedInstallMetadata,
};
use ctx_http::installs::{
    InstallId, InstallInfo, InstallProgressEvent, InstallStateKind, InstallTarget,
};
use ctx_http::provider_matrix::{
    matrix_cache_path, ProviderArchiveKind, ProviderArchiveTarget, ProviderInstall,
    ProviderInstallDependency as MatrixProviderInstallDependency, ProviderInstallDependencyRole,
    ProviderInstallDependencyTarget, ProviderMatrix, ProviderMatrixEntry, ProviderMatrixEntryKind,
    ProviderRelease, ProviderReleaseStatus,
};
use ctx_http::settings::{
    save_settings, ContainerExecutionSettings, ContainerMountMode, ContainerNetworkMode,
    ExecutionMode, ExecutionSettings, Settings,
};
use ctx_providers::adapters::{ProviderAdapter, ProviderHealth, ProviderStatus};
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

fn file_url(path: &Path) -> String {
    url::Url::from_file_path(path)
        .expect("file url")
        .to_string()
}

#[derive(Clone)]
struct DownloadFixture {
    body: Vec<u8>,
    delay_ms: u64,
}

async fn spawn_download_fixture_server(fixtures: Vec<(&str, Vec<u8>, u64)>) -> common::TestServer {
    let fixture_map = Arc::new(
        fixtures
            .into_iter()
            .map(|(name, body, delay_ms)| (name.to_string(), DownloadFixture { body, delay_ms }))
            .collect::<HashMap<_, _>>(),
    );

    async fn serve_fixture(
        axum::extract::State(fixtures): axum::extract::State<Arc<HashMap<String, DownloadFixture>>>,
        axum::extract::Path(name): axum::extract::Path<String>,
    ) -> impl axum::response::IntoResponse {
        let fixture = fixtures
            .get(&name)
            .cloned()
            .expect("missing download fixture");
        tokio::time::sleep(Duration::from_millis(fixture.delay_ms)).await;
        (
            [(axum::http::header::CONTENT_TYPE, "application/octet-stream")],
            fixture.body,
        )
    }

    common::spawn_http_server(
        axum::Router::new()
            .route("/:name", axum::routing::get(serve_fixture))
            .with_state(fixture_map),
    )
    .await
}

fn fixture_download_url(server: &common::TestServer, name: &str) -> String {
    format!("{}/{}", server.base_url, name)
}

fn local_archive_entry(url: String) -> ProviderArchiveTarget {
    ProviderArchiveTarget {
        url,
        sha256: None,
        size_bytes: None,
        archive: ProviderArchiveKind::None,
        bin_path: "bin/runtime".to_string(),
    }
}

async fn save_matrix_fixture(data_root: &Path, matrix: &ProviderMatrix) {
    let path = matrix_cache_path(data_root);
    let parent = path.parent().expect("matrix cache parent");
    tokio::fs::create_dir_all(parent)
        .await
        .expect("create matrix cache dir");
    tokio::fs::write(
        &path,
        serde_json::to_vec_pretty(matrix).expect("serialize matrix"),
    )
    .await
    .expect("write matrix cache");
}

fn archive_targets(url: String) -> HashMap<String, ProviderArchiveTarget> {
    let mut targets = HashMap::from([
        (
            "linux-aarch64".to_string(),
            local_archive_entry(url.clone()),
        ),
        ("linux-x86_64".to_string(), local_archive_entry(url.clone())),
    ]);
    if let Ok(host_target_key) = ctx_http::installer::resolve_matrix_target_key(InstallTarget::Host)
    {
        targets.insert(host_target_key.to_string(), local_archive_entry(url));
    }
    targets
}

fn bridge_fixture_entry(bridge_url: String) -> ProviderMatrixEntry {
    let bridge_targets = archive_targets(bridge_url);
    ProviderMatrixEntry {
        id: "acp-crp-bridge".to_string(),
        kind: ProviderMatrixEntryKind::Dependency,
        display_name: Some("ACP Bridge".to_string()),
        tier: Some("tier2".to_string()),
        command: None,
        managed_install: Some(ProviderInstall::Archive {
            version: "0.1.0".to_string(),
            args: Vec::new(),
            targets: bridge_targets,
        }),
        provider_dependencies: Vec::new(),
        dependencies: Vec::new(),
        version_probe: None,
        releases: vec![ProviderRelease {
            version: "0.1.0".to_string(),
            status: ProviderReleaseStatus::Supported,
            upstream_version: None,
            provenance: None,
            context_min: None,
            context_max: None,
            notes: None,
        }],
    }
}

fn acp_provider_fixture_entry(provider_id: &str, provider_url: String) -> ProviderMatrixEntry {
    ProviderMatrixEntry {
        id: provider_id.to_string(),
        kind: ProviderMatrixEntryKind::Harness,
        display_name: Some(provider_id.to_string()),
        tier: Some("tier2".to_string()),
        command: None,
        managed_install: Some(ProviderInstall::Archive {
            version: "0.1.0".to_string(),
            args: vec!["--provider".to_string()],
            targets: archive_targets(provider_url),
        }),
        provider_dependencies: Vec::new(),
        dependencies: Vec::new(),
        version_probe: None,
        releases: vec![ProviderRelease {
            version: "0.1.0".to_string(),
            status: ProviderReleaseStatus::Supported,
            upstream_version: None,
            provenance: None,
            context_min: None,
            context_max: None,
            notes: None,
        }],
    }
}

fn archive_fixture_entry(
    provider_id: &str,
    kind: ProviderMatrixEntryKind,
    provider_url: String,
) -> ProviderMatrixEntry {
    ProviderMatrixEntry {
        id: provider_id.to_string(),
        kind,
        display_name: Some(provider_id.to_string()),
        tier: Some("tier2".to_string()),
        command: None,
        managed_install: Some(ProviderInstall::Archive {
            version: "0.1.0".to_string(),
            args: Vec::new(),
            targets: archive_targets(provider_url),
        }),
        provider_dependencies: Vec::new(),
        dependencies: Vec::new(),
        version_probe: None,
        releases: vec![ProviderRelease {
            version: "0.1.0".to_string(),
            status: ProviderReleaseStatus::Supported,
            upstream_version: None,
            provenance: None,
            context_min: None,
            context_max: None,
            notes: None,
        }],
    }
}

fn provider_fixture_matrix_with_providers(
    bridge_url: String,
    providers: Vec<(&str, String)>,
) -> ProviderMatrix {
    let mut entries = vec![bridge_fixture_entry(bridge_url)];
    entries.extend(
        providers.into_iter().map(|(provider_id, provider_url)| {
            acp_provider_fixture_entry(provider_id, provider_url)
        }),
    );
    ProviderMatrix {
        version: 2,
        generated_at: None,
        providers: entries,
    }
}

fn provider_fixture_matrix(bridge_url: String, provider_url: String) -> ProviderMatrix {
    provider_fixture_matrix_with_providers(bridge_url, vec![("kimi", provider_url)])
}

async fn wait_for_install_completion(
    state: &Arc<AppState>,
    install_id: InstallId,
) -> ctx_http::installs::InstallInfo {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        let info = state
            .get_install_info(install_id)
            .await
            .expect("missing install info");
        if !matches!(info.state, InstallStateKind::Running) {
            return info;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for install {install_id}: {info:#?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn parse_install_ids(body: &serde_json::Value) -> HashMap<String, InstallId> {
    body.as_array()
        .cloned()
        .expect("install response should be an array")
        .into_iter()
        .map(|entry| {
            let provider_id = entry
                .get("provider_id")
                .and_then(serde_json::Value::as_str)
                .expect("provider id")
                .to_string();
            let install_id = entry
                .get("install_id")
                .and_then(serde_json::Value::as_str)
                .and_then(|raw| raw.parse::<InstallId>().ok())
                .expect("install id");
            (provider_id, install_id)
        })
        .collect()
}

fn install_stage_progress_value(stage: &str) -> Option<u32> {
    match stage {
        "start" => Some(2),
        "prerequisites" => Some(2),
        "download" => Some(10),
        "node" => Some(15),
        "node_download" => Some(18),
        "prepare" => Some(25),
        "venv" => Some(35),
        "npm_install" => Some(65),
        "pip_install" => Some(70),
        "extract" => Some(78),
        "entrypoint" => Some(80),
        "inspect" => Some(90),
        "refresh" => Some(95),
        "registry" => Some(98),
        _ => None,
    }
}

fn compute_polled_install_pct(info: &InstallInfo, previous_pct: Option<u32>) -> Option<u32> {
    if matches!(info.state, InstallStateKind::Succeeded) {
        return Some(100);
    }
    let last_event = info.last_event.as_ref()?;
    let staged = install_stage_progress_value(last_event.stage.as_str())?;
    Some(previous_pct.map_or(staged, |pct| pct.max(staged)))
}

async fn get_install_info_api(app: &axum::Router, install_id: InstallId) -> InstallInfo {
    let (status, body): (StatusCode, InstallInfo) = common::json_request(
        app,
        axum::http::Method::GET,
        format!("/api/providers/install/{install_id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "install info failed: {body:#?}");
    body
}

async fn wait_for_running_install_progress(
    state: &Arc<AppState>,
    install_id: InstallId,
) -> ctx_http::installs::InstallInfo {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let info = state
            .get_install_info(install_id)
            .await
            .expect("missing install info");
        if matches!(info.state, InstallStateKind::Running) && info.last_event.is_some() {
            return info;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for running install {install_id} to expose real progress: {info:#?}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn wait_for_running_install_id(
    state: &AppState,
    provider_id: &str,
    target: Option<InstallTarget>,
) -> InstallId {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(install_id) = state.find_running_install(provider_id, target).await {
            return install_id;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for running install {provider_id} with target {target:?}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn wait_for_prerequisite_visibility(
    state: &Arc<AppState>,
    app: &axum::Router,
    install_id: InstallId,
    prerequisite_install_id: InstallId,
) -> InstallInfo {
    let prerequisite_install_id_string = prerequisite_install_id.to_string();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let prerequisite_info = state
            .get_install_info(prerequisite_install_id)
            .await
            .expect("missing prerequisite install info");
        let info = get_install_info_api(app, install_id).await;
        if matches!(prerequisite_info.state, InstallStateKind::Running)
            && prerequisite_info.last_event.is_some()
            && info.last_event.as_ref().is_some_and(|event| {
                event.message.contains("acp-crp-bridge")
                    && event.message.contains(&prerequisite_install_id_string)
            })
        {
            return info;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for live prerequisite visibility on install {install_id}: prerequisite={prerequisite_info:#?} parent={info:#?}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn get_install_events_api(
    app: &axum::Router,
    install_id: InstallId,
) -> Vec<InstallProgressEvent> {
    let (status, body): (StatusCode, Vec<InstallProgressEvent>) = common::json_request(
        app,
        axum::http::Method::GET,
        format!("/api/providers/install/{install_id}/events"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "install events failed: {body:#?}");
    body
}

async fn save_invalid_container_bridge_runtime(data_root: &Path) {
    let mut cfg = load_agent_server_config(data_root)
        .await
        .unwrap_or_default();
    cfg.managed_provider_targets.insert(
        "acp-crp-bridge".to_string(),
        HashMap::from([(
            "container".to_string(),
            AgentServerCommand {
                command: "relative-bridge".to_string(),
                args: Vec::new(),
                dependencies: Vec::new(),
                managed: Some(ManagedInstallMetadata {
                    package: Some("acp-crp-bridge".to_string()),
                    version: Some("1.0.0".to_string()),
                    target: Some(ctx_http::installs::InstallTarget::Container),
                    install_dir_rel: Some(
                        "providers/agent-servers/acp-crp-bridge/invalid".to_string(),
                    ),
                    bin_dir_rel: Some("providers/agent-servers/acp-crp-bridge/invalid".to_string()),
                    last_success_at: None,
                    last_error: None,
                }),
            },
        )]),
    );
    save_agent_server_config(data_root, &cfg)
        .await
        .expect("save invalid bridge runtime config");
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
async fn acp_host_install_surfaces_bridge_as_installable_prerequisite() {
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
        "/api/providers/kimi?target=host",
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
        "host ACP installs should remain supported when the bridge is installable: {provider_body:#?}"
    );
    assert!(
        provider_body
            .pointer("/details/install_blocked_code")
            .is_none(),
        "installable bridge prerequisites must not be surfaced as blocked on host: {provider_body:#?}"
    );

    let (providers_status, providers_body): (StatusCode, serde_json::Value) = common::json_request(
        &app,
        axum::http::Method::GET,
        "/api/providers?target=host",
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
        "providers list should advertise installable ACP host targets: {kimi_status:#?}"
    );
    assert!(
        kimi_status.pointer("/details/install_blocked_code").is_none(),
        "providers list must not mark installable ACP bridge prerequisites as blocked on host: {kimi_status:#?}"
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
            .is_some_and(|value| {
                value.contains("Required prerequisite dependency 'acp-crp-bridge'")
                    && value.contains("target 'container'")
            }),
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
        !cfg.managed_provider_targets.contains_key("kimi"),
        "failed preflight must not write partial provider install state"
    );
    assert!(
        !cfg.managed_install_targets.contains_key("kimi"),
        "failed preflight must not write partial install metadata"
    );
}

#[tokio::test]
async fn provider_target_scoped_installs_install_all_repairs_invalid_bridge_and_keeps_acp_dependents_in_batch(
) {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let fixture_dir = data_dir.path().join("fixtures");
    std::fs::create_dir_all(&fixture_dir).expect("create fixture dir");
    let bridge_fixture = fixture_dir.join("acp-crp-bridge");
    let kimi_fixture = fixture_dir.join("kimi-acp");
    let qwen_fixture = fixture_dir.join("qwen-acp");
    write_executable(&bridge_fixture, "#!/bin/sh\nsleep 0.3\nexit 0\n");
    write_executable(&kimi_fixture, "#!/bin/sh\nexit 0\n");
    write_executable(&qwen_fixture, "#!/bin/sh\nexit 0\n");
    save_matrix_fixture(
        data_dir.path(),
        &provider_fixture_matrix_with_providers(
            file_url(&bridge_fixture),
            vec![
                ("kimi", file_url(&kimi_fixture)),
                ("qwen", file_url(&qwen_fixture)),
            ],
        ),
    )
    .await;

    save_invalid_container_bridge_runtime(data_dir.path()).await;

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
        "/api/providers/install_all?target=container",
        None,
    )
    .await;
    assert_eq!(
        install_status,
        StatusCode::OK,
        "bulk install should accept the bridge repair flow: {install_body:#?}"
    );
    let install_ids = parse_install_ids(&install_body);
    assert_eq!(
        install_ids.len(),
        3,
        "bulk install should keep the bridge and ACP dependents in the same batch: {install_body:#?}"
    );
    assert!(
        install_ids.contains_key("acp-crp-bridge"),
        "bridge repair must stay in the batch: {install_body:#?}"
    );
    assert!(
        install_ids.contains_key("kimi"),
        "kimi should be deferred behind the bridge repair instead of skipped: {install_body:#?}"
    );
    assert!(
        install_ids.contains_key("qwen"),
        "qwen should be deferred behind the bridge repair instead of skipped: {install_body:#?}"
    );

    for provider_id in ["acp-crp-bridge", "kimi", "qwen"] {
        let install_info = wait_for_install_completion(
            &state,
            *install_ids
                .get(provider_id)
                .expect("missing install id from bulk response"),
        )
        .await;
        assert!(
            matches!(install_info.state, InstallStateKind::Succeeded),
            "{provider_id} should succeed after the bridge repair batch: {install_info:#?}"
        );
    }

    let reloaded_stores = common::setup_store(data_dir.path()).await;
    let reloaded_state = common::build_state(
        data_dir.path().to_path_buf(),
        reloaded_stores,
        HashMap::new(),
        "http://127.0.0.1:0",
    );
    let reloaded_app = common::router(reloaded_state);

    for provider_id in ["kimi", "qwen"] {
        let (provider_status, provider_body): (StatusCode, serde_json::Value) =
            common::json_request(
                &reloaded_app,
                axum::http::Method::GET,
                format!("/api/providers/{provider_id}?target=container"),
                None,
            )
            .await;
        assert_eq!(
            provider_status,
            StatusCode::OK,
            "provider status failed after bulk repair/install: {provider_body:#?}"
        );
        assert_eq!(
            provider_body
                .get("installed")
                .and_then(serde_json::Value::as_bool),
            Some(true),
            "{provider_id} should be installed by the same bulk repair batch: {provider_body:#?}"
        );
    }
}

#[tokio::test]
async fn provider_target_scoped_installs_install_all_repairs_invalid_bridge_when_acp_dependents_precede_bridge_in_matrix(
) {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let fixture_dir = data_dir.path().join("fixtures");
    std::fs::create_dir_all(&fixture_dir).expect("create fixture dir");
    let bridge_fixture = fixture_dir.join("acp-crp-bridge");
    let kimi_fixture = fixture_dir.join("kimi-acp");
    let qwen_fixture = fixture_dir.join("qwen-acp");
    write_executable(&bridge_fixture, "#!/bin/sh\nsleep 0.3\nexit 0\n");
    write_executable(&kimi_fixture, "#!/bin/sh\nexit 0\n");
    write_executable(&qwen_fixture, "#!/bin/sh\nexit 0\n");
    let download_server = spawn_download_fixture_server(vec![
        (
            "bridge",
            std::fs::read(&bridge_fixture).expect("read bridge fixture"),
            1_600,
        ),
        (
            "kimi",
            std::fs::read(&kimi_fixture).expect("read kimi fixture"),
            0,
        ),
        (
            "qwen",
            std::fs::read(&qwen_fixture).expect("read qwen fixture"),
            0,
        ),
    ])
    .await;
    save_matrix_fixture(
        data_dir.path(),
        &ProviderMatrix {
            version: 2,
            generated_at: None,
            providers: vec![
                acp_provider_fixture_entry("kimi", fixture_download_url(&download_server, "kimi")),
                acp_provider_fixture_entry("qwen", fixture_download_url(&download_server, "qwen")),
                bridge_fixture_entry(fixture_download_url(&download_server, "bridge")),
            ],
        },
    )
    .await;

    save_invalid_container_bridge_runtime(data_dir.path()).await;

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
        "/api/providers/install_all?target=container",
        None,
    )
    .await;
    assert_eq!(
        install_status,
        StatusCode::OK,
        "bulk install should repair an invalid bridge even when ACP providers are listed first: {install_body:#?}"
    );
    let install_ids = parse_install_ids(&install_body);
    assert_eq!(
        install_ids.len(),
        3,
        "install_all should still return the bridge repair plus both ACP dependents: {install_body:#?}"
    );
    let bridge_install_id = *install_ids
        .get("acp-crp-bridge")
        .expect("missing bridge install id");
    let kimi_install_id = *install_ids.get("kimi").expect("missing kimi install id");
    let qwen_install_id = *install_ids.get("qwen").expect("missing qwen install id");

    let (kimi_polled, qwen_polled) = tokio::join!(
        wait_for_prerequisite_visibility(&state, &app, kimi_install_id, bridge_install_id),
        wait_for_prerequisite_visibility(&state, &app, qwen_install_id, bridge_install_id)
    );
    for (provider_id, polled_info) in [("kimi", kimi_polled), ("qwen", qwen_polled)] {
        assert!(
            matches!(polled_info.state, InstallStateKind::Running),
            "{provider_id} should remain queued behind the shared bridge repair while the prerequisite is active: {polled_info:#?}"
        );
        assert_eq!(
            polled_info
                .last_event
                .as_ref()
                .map(|event| event.stage.as_str()),
            Some("start"),
            "{provider_id} should expose bounded prerequisite progress while waiting on the bridge repair: {polled_info:#?}"
        );
    }

    for provider_id in ["acp-crp-bridge", "kimi", "qwen"] {
        let install_info = wait_for_install_completion(
            &state,
            *install_ids
                .get(provider_id)
                .expect("missing install id from bulk response"),
        )
        .await;
        assert!(
            matches!(install_info.state, InstallStateKind::Succeeded),
            "{provider_id} should succeed after the repaired bulk install finishes: {install_info:#?}"
        );
    }

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
        "deferred ACP installs should reuse one tracked bridge repair install even when the bridge is listed after them"
    );
    drop(installs);

    for (provider_id, install_id) in [("kimi", kimi_install_id), ("qwen", qwen_install_id)] {
        let events = get_install_events_api(&app, install_id).await;
        assert!(
            events.iter().any(|event| {
                event.stage == "start"
                    && event.message.contains(&format!(
                        "Prerequisite acp-crp-bridge (install {bridge_install_id}"
                    ))
            }),
            "{provider_id} should retain the shared bridge prerequisite in its event history: {events:#?}"
        );
    }
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
    write_executable(&provider_fixture, "#!/bin/sh\nsleep 0.5\nexit 0\n");
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

    let bridge_install_id =
        wait_for_running_install_id(&state, "acp-crp-bridge", Some(InstallTarget::Container)).await;
    let _ = wait_for_running_install_progress(&state, bridge_install_id).await;
    let polled_info =
        wait_for_prerequisite_visibility(&state, &app, install_id, bridge_install_id).await;
    assert!(
        matches!(polled_info.state, InstallStateKind::Running),
        "install should still be running while the bridge prerequisite is active: {polled_info:#?}"
    );
    assert!(
        polled_info
            .last_event
            .as_ref()
            .is_some_and(|event| matches!(event.stage.as_str(), "start" | "prerequisites")),
        "parent poll surface should keep prerequisite progress bounded while the bridge prerequisite runs: {polled_info:#?}"
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
            .is_some_and(|event| event.message.contains("acp-crp-bridge")),
        "parent poll should still expose prerequisite bridge activity: {polled_info:#?}"
    );

    let parent_events = get_install_events_api(&app, install_id).await;
    assert!(
        parent_events.iter().any(|event| {
            event.message.contains("acp-crp-bridge")
                && event.message.contains(&bridge_install_id.to_string())
                && matches!(event.stage.as_str(), "start" | "prerequisites")
        }),
        "parent install events should preserve prerequisite visibility via the real API surface: {parent_events:#?}"
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
    // The throttled fixture server can delay the first observable progress event under full-suite load.
    let _ = wait_for_running_install_progress(&state, bridge_install_id).await;

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

    let _ = wait_for_prerequisite_visibility(&state, &app, install_id, bridge_install_id).await;
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
        assert!(
            polled_info
                .last_event
                .as_ref()
                .is_some_and(|event| matches!(event.stage.as_str(), "start" | "prerequisites")),
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
                    event.message.contains("acp-crp-bridge")
                        && event.message.contains(&bridge_install_id.to_string())
                }),
            "the first poll should still be showing prerequisite-derived progress, not a rewritten parent event: {polled_info:#?}"
        );
    }

    let parent_events = get_install_events_api(&app, install_id).await;
    assert!(
        parent_events.iter().any(|event| {
            event.message.contains("acp-crp-bridge")
                && event.message.contains(&bridge_install_id.to_string())
                && matches!(event.stage.as_str(), "start" | "prerequisites")
        }),
        "parent install events should retain short prerequisite visibility on the real API surface: {parent_events:#?}"
    );

    let parent_owned_poll_info =
        {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
            loop {
                let info = get_install_info_api(&app, install_id).await;
                if info.last_event.as_ref().is_some_and(|event| {
                    !event.message.to_ascii_lowercase().contains("prerequisite")
                }) {
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
        matches!(
            parent_owned_poll_info.state,
            InstallStateKind::Running | InstallStateKind::Succeeded
        ),
        "the parent poll surface should switch off prerequisite-derived progress once the override window expires: {parent_owned_poll_info:#?}"
    );
    assert!(
        parent_owned_poll_info
            .last_event
            .as_ref()
            .is_some_and(|event| !event.message.to_ascii_lowercase().contains("prerequisite")),
        "once the prerequisite override window expires, polling should surface the parent install's own work: {parent_owned_poll_info:#?}"
    );
    assert!(
        parent_owned_poll_info
            .last_event
            .as_ref()
            .is_some_and(|event| event.stage != "prerequisites"),
        "the next poll after the prerequisite window should expose the parent install's own work or terminal completion instead of staying on the synthetic prerequisite stage: {parent_owned_poll_info:#?}"
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
    assert_eq!(
        final_parent_events
            .iter()
            .filter(|event| event.stage == "start" && !event.message.starts_with("Prerequisite "))
            .count(),
        1,
        "the parent install should keep exactly one parent-owned start event in history: {final_parent_events:#?}"
    );
    assert!(
        final_parent_events.iter().any(|event| {
            event.stage == "start"
                && !event.message.starts_with("Prerequisite ")
                && event.message.contains("Installing managed provider: kimi")
                && event.message.contains("target: container")
        }),
        "the parent install should preserve its richer canonical start message: {final_parent_events:#?}"
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
async fn claude_container_install_starts_host_cli_dependency_and_stays_not_ready_until_it_finishes()
{
    let data_dir = tempfile::tempdir().expect("tempdir");
    let fixture_dir = data_dir.path().join("fixtures");
    std::fs::create_dir_all(&fixture_dir).expect("create fixture dir");
    let claude_cli_fixture = fixture_dir.join("claude-cli");
    let claude_crp_fixture = fixture_dir.join("claude-crp");
    write_executable(&claude_cli_fixture, "#!/bin/sh\nsleep 1.6\nexit 0\n");
    write_executable(&claude_crp_fixture, "#!/bin/sh\nexit 0\n");
    let download_server = spawn_download_fixture_server(vec![
        (
            "claude-cli",
            std::fs::read(&claude_cli_fixture).expect("read claude-cli fixture"),
            3_200,
        ),
        (
            "claude-crp",
            std::fs::read(&claude_crp_fixture).expect("read claude-crp fixture"),
            0,
        ),
    ])
    .await;
    let mut claude_crp = archive_fixture_entry(
        "claude-crp",
        ProviderMatrixEntryKind::Harness,
        fixture_download_url(&download_server, "claude-crp"),
    );
    claude_crp.provider_dependencies = vec![MatrixProviderInstallDependency {
        id: "claude-cli".to_string(),
        role: ProviderInstallDependencyRole::Readiness,
        target: ProviderInstallDependencyTarget::Host,
    }];
    save_matrix_fixture(
        data_dir.path(),
        &ProviderMatrix {
            version: 2,
            generated_at: None,
            providers: vec![
                claude_crp,
                archive_fixture_entry(
                    "claude-cli",
                    ProviderMatrixEntryKind::Dependency,
                    fixture_download_url(&download_server, "claude-cli"),
                ),
            ],
        },
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
        "/api/providers/claude-crp/install?target=container",
        None,
    )
    .await;
    assert_eq!(
        install_status,
        StatusCode::OK,
        "claude install should start successfully: {install_body:#?}"
    );
    let install_id = install_body
        .get("install_id")
        .and_then(serde_json::Value::as_str)
        .and_then(|raw| raw.parse::<InstallId>().ok())
        .expect("install id");

    let dependency_deadline = tokio::time::Instant::now() + Duration::from_secs(6);
    let claude_cli_install_id = loop {
        if let Some(install_id) = state
            .find_running_install("claude-cli", Some(InstallTarget::Host))
            .await
        {
            break install_id;
        }
        assert!(
            tokio::time::Instant::now() < dependency_deadline,
            "timed out waiting for claude-cli dependency install to start"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    };

    let visibility_deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    let (parent_poll, parent_status_body) = loop {
        let parent_poll = get_install_info_api(&app, install_id).await;
        let (provider_status, provider_body): (StatusCode, serde_json::Value) =
            common::json_request(
                &app,
                axum::http::Method::GET,
                "/api/providers/claude-crp?target=container",
                None,
            )
            .await;
        assert_eq!(
            provider_status,
            StatusCode::OK,
            "provider status failed while claude-cli was still installing: {provider_body:#?}"
        );
        if matches!(parent_poll.state, InstallStateKind::Running)
            && parent_poll.progress_pct == Some(99)
        {
            break (parent_poll, provider_body);
        }
        assert!(
            tokio::time::Instant::now() < visibility_deadline,
            "timed out waiting for claude readiness gating to surface: {parent_poll:#?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(
        parent_poll.progress_pct,
        Some(99),
        "readiness dependency waiting should pin parent polling progress at 99: {parent_poll:#?}"
    );
    assert!(
        parent_poll
            .last_event
            .as_ref()
            .is_some_and(|event| event.message.contains("claude-cli")),
        "parent poll should expose dependency activity while claude-cli is still installing: {parent_poll:#?}"
    );
    assert_eq!(
        parent_status_body
            .get("installed")
            .and_then(serde_json::Value::as_bool),
        Some(true),
        "claude-crp should already be installed while waiting on claude-cli readiness: {parent_status_body:#?}"
    );
    assert_eq!(
        parent_status_body
            .pointer("/details/managed_target")
            .and_then(serde_json::Value::as_str),
        Some("container"),
        "claude-crp should keep its container-managed target while waiting: {parent_status_body:#?}"
    );
    assert_eq!(
        parent_status_body
            .pointer("/details/ready_for_use")
            .and_then(serde_json::Value::as_str),
        Some("false"),
        "claude-crp should stay not-ready until claude-cli finishes: {parent_status_body:#?}"
    );
    assert_eq!(
        parent_status_body
            .pointer("/details/required_dependency_ids")
            .and_then(serde_json::Value::as_str),
        Some("claude-cli"),
        "status should expose the declared Claude dependency set: {parent_status_body:#?}"
    );
    assert_eq!(
        parent_status_body
            .pointer("/details/pending_dependency_ids")
            .and_then(serde_json::Value::as_str),
        Some("claude-cli"),
        "status should keep claude-cli pending until the dependency install completes: {parent_status_body:#?}"
    );
    assert_eq!(
        parent_status_body
            .pointer("/details/install_target")
            .and_then(serde_json::Value::as_str),
        Some("container"),
        "status should remain target-aware while waiting for the host dependency: {parent_status_body:#?}"
    );

    let dependency_info = wait_for_install_completion(&state, claude_cli_install_id).await;
    assert!(
        matches!(dependency_info.state, InstallStateKind::Succeeded),
        "claude-cli dependency install should succeed: {dependency_info:#?}"
    );
    let parent_info = wait_for_install_completion(&state, install_id).await;
    assert!(
        matches!(parent_info.state, InstallStateKind::Succeeded),
        "claude-crp install should complete after its host dependency finishes: {parent_info:#?}"
    );

    let cfg = load_agent_server_config(data_dir.path())
        .await
        .expect("load agent server config");
    assert!(
        cfg.managed_provider_targets
            .get("claude-crp")
            .and_then(|targets| targets.get("container"))
            .is_some(),
        "claude-crp container runtime should be registered"
    );
    assert!(
        cfg.managed_install_targets
            .get("claude-crp")
            .and_then(|targets| targets.get("container"))
            .is_some(),
        "claude-crp container install metadata should be registered"
    );
    assert!(
        cfg.managed_provider_targets
            .get("claude-cli")
            .and_then(|targets| targets.get("host"))
            .is_some(),
        "claude-cli host runtime should be registered"
    );
    assert!(
        cfg.managed_install_targets
            .get("claude-cli")
            .and_then(|targets| targets.get("host"))
            .is_some(),
        "claude-cli host install metadata should be registered"
    );
    let claude_crp_runtime = cfg
        .managed_provider_targets
        .get("claude-crp")
        .and_then(|targets| targets.get("container"))
        .expect("claude-crp container runtime should exist");
    assert_eq!(
        claude_crp_runtime.dependencies,
        vec!["claude-cli".to_string()],
        "claude-crp runtime should persist the managed dependency edge"
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
        "/api/providers/claude-crp?target=container",
        None,
    )
    .await;
    assert_eq!(
        provider_status,
        StatusCode::OK,
        "provider status failed after claude install completion: {provider_body:#?}"
    );
    assert_eq!(
        provider_body
            .get("installed")
            .and_then(serde_json::Value::as_bool),
        Some(true),
        "claude-crp should remain installed after its dependency completes: {provider_body:#?}"
    );
    assert_eq!(
        provider_body
            .pointer("/details/ready_for_use")
            .and_then(serde_json::Value::as_str),
        Some("true"),
        "claude-crp should become ready once claude-cli finishes: {provider_body:#?}"
    );
    assert!(
        provider_body.pointer("/details/pending_dependency_ids").is_none(),
        "pending dependency ids should clear once the host dependency is installed: {provider_body:#?}"
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
