#![cfg(unix)]

mod common;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use axum::http::StatusCode;
use ctx_http::installer::{
    save_agent_server_config, AgentServerCommand, AgentServerConfigFile, ManagedInstallMetadata,
};
use ctx_http::installs::InstallTarget;
use ctx_providers::adapters::{
    ProviderAdapter, ProviderCapabilities, ProviderHealth, ProviderStatus, RunHandle, TurnInput,
};
use ctx_providers::events::NormalizedEvent;

fn write_runtime_fixture(path: &Path) {
    std::fs::write(path, "#!/bin/sh\nexit 0\n").expect("write runtime fixture");
}

#[derive(Clone)]
struct StatusOnlyAdapter {
    status: ProviderStatus,
}

#[async_trait]
impl ProviderAdapter for StatusOnlyAdapter {
    async fn inspect(&self) -> Result<ProviderStatus> {
        Ok(self.status.clone())
    }

    async fn run(
        &self,
        _input: TurnInput,
        _workdir: std::path::PathBuf,
        _env: HashMap<String, String>,
        _event_sink: tokio::sync::mpsc::Sender<NormalizedEvent>,
    ) -> Result<RunHandle> {
        anyhow::bail!("not used in test")
    }

    async fn cancel(&self, _handle: RunHandle) -> Result<()> {
        Ok(())
    }
}

fn bridge_missing_status(provider_id: &str) -> ProviderStatus {
    ProviderStatus {
        provider_id: provider_id.to_string(),
        installed: false,
        detected_path: None,
        version: None,
        capabilities: None,
        health: ProviderHealth::Error,
        diagnostics: vec!["ACP bridge runtime is not configured or invalid".to_string()],
        details: HashMap::from([("error_code".to_string(), "acp_bridge_missing".to_string())]),
        usability: ctx_providers::adapters::ProviderUsability::default(),
    }
}

fn healthy_container_status(provider_id: &str, detected_path: &Path) -> ProviderStatus {
    ProviderStatus {
        provider_id: provider_id.to_string(),
        installed: true,
        detected_path: Some(detected_path.to_string_lossy().to_string()),
        version: Some("1.0.0".to_string()),
        capabilities: Some(ProviderCapabilities {
            stream_events: true,
            stream_format: "crp".to_string(),
            has_turn_boundaries: true,
            has_tool_call_ids: true,
            has_file_change_events: false,
            has_command_events: false,
            supports_resume: false,
            supports_stable_session_id: false,
            supports_fork_or_rewind: false,
            supports_headless: true,
            supports_server_mode: true,
            supports_interactive_tui: false,
            supports_private_state_dir: true,
            supports_sandbox_flags: false,
            supports_approval_flags: false,
            notes: Vec::new(),
        }),
        health: ProviderHealth::Ok,
        diagnostics: Vec::new(),
        details: HashMap::new(),
        usability: ctx_providers::adapters::ProviderUsability::default(),
    }
}

async fn seed_container_only_install(data_root: &Path, provider_id: &str) -> std::path::PathBuf {
    let install_dir = data_root
        .join("providers")
        .join("agent-servers")
        .join(provider_id)
        .join("fixture")
        .join("container");
    std::fs::create_dir_all(&install_dir).expect("create install dir");
    let command_path = install_dir.join(format!("{provider_id}-runtime"));
    write_runtime_fixture(&command_path);

    let meta = ManagedInstallMetadata {
        package: Some(format!("{provider_id}-pkg")),
        version: Some("1.0.0".to_string()),
        target: Some(InstallTarget::Container),
        install_dir_rel: Some(format!(
            "providers/agent-servers/{provider_id}/fixture/container"
        )),
        bin_dir_rel: Some(format!(
            "providers/agent-servers/{provider_id}/fixture/container"
        )),
        last_success_at: None,
        last_error: None,
    };

    let mut cfg = AgentServerConfigFile::default();
    cfg.managed_install_targets.insert(
        provider_id.to_string(),
        HashMap::from([("container".to_string(), meta.clone())]),
    );
    cfg.managed_provider_targets.insert(
        provider_id.to_string(),
        HashMap::from([(
            "container".to_string(),
            AgentServerCommand {
                command: command_path.to_string_lossy().to_string(),
                args: Vec::new(),
                dependencies: Vec::new(),
                managed: Some(meta),
            },
        )]),
    );
    save_agent_server_config(data_root, &cfg)
        .await
        .expect("save agent server config");

    command_path
}

#[tokio::test]
async fn host_target_reports_target_mismatch_for_container_only_acp_installs() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let stores = common::setup_store(data_dir.path()).await;
    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:0",
    );
    let app = common::router(state.clone());

    for provider_id in ["kimi", "mistral", "qwen"] {
        let _command_path = seed_container_only_install(data_dir.path(), provider_id).await;
        state
            .providers
            .statuses
            .lock()
            .await
            .insert(provider_id.to_string(), bridge_missing_status(provider_id));

        let (status, body): (StatusCode, serde_json::Value) = common::json_request(
            &app,
            axum::http::Method::GET,
            format!("/api/providers/{provider_id}?target=host"),
            None,
        )
        .await;

        assert_eq!(
            status,
            StatusCode::OK,
            "provider route failed for {provider_id}: {body:#?}"
        );
        assert_eq!(
            body.get("installed").and_then(serde_json::Value::as_bool),
            Some(false),
            "expected host-target install=false for {provider_id}: {body:#?}"
        );
        assert_eq!(
            body.pointer("/details/target_mismatch")
                .and_then(serde_json::Value::as_str),
            Some("true"),
            "expected target_mismatch for {provider_id}: {body:#?}"
        );
        assert_eq!(
            body.pointer("/details/managed_target")
                .and_then(serde_json::Value::as_str),
            Some("container"),
            "expected managed_target=container for {provider_id}: {body:#?}"
        );
        let diagnostics = body
            .get("diagnostics")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(
            diagnostics.iter().any(|value| {
                value
                    .as_str()
                    .is_some_and(|text| text.contains("installed for target 'container'"))
            }),
            "expected target mismatch diagnostic for {provider_id}: {body:#?}"
        );
        assert_ne!(
            body.pointer("/details/error_code")
                .and_then(serde_json::Value::as_str),
            Some("acp_bridge_missing"),
            "host target should not surface ACP bridge missing when only container install exists: {body:#?}"
        );
    }
}

#[tokio::test]
async fn acp_provider_reports_missing_bridge_as_blocking_dependency() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let stores = common::setup_store(data_dir.path()).await;
    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:0",
    );
    let app = common::router(state.clone());

    state
        .providers
        .statuses
        .lock()
        .await
        .insert("qwen".to_string(), bridge_missing_status("qwen"));

    let (status, body): (StatusCode, serde_json::Value) = common::json_request(
        &app,
        axum::http::Method::GET,
        "/api/providers/qwen?target=host",
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK, "provider route failed: {body:#?}");
    assert_eq!(
        body.pointer("/usability/usable")
            .and_then(serde_json::Value::as_bool),
        Some(false),
        "expected unusable ACP provider when bridge is missing: {body:#?}"
    );
    assert_eq!(
        body.pointer("/usability/status")
            .and_then(serde_json::Value::as_str),
        Some("blocked"),
        "expected blocked usability status when bridge is missing: {body:#?}"
    );
    assert_eq!(
        body.pointer("/usability/reason_code")
            .and_then(serde_json::Value::as_str),
        Some("missing_dependency"),
        "expected missing dependency reason when bridge is missing: {body:#?}"
    );
    assert_eq!(
        body.pointer("/usability/recommended_action")
            .and_then(serde_json::Value::as_str),
        Some("resolve_dependency"),
        "expected dependency resolution action when bridge is missing: {body:#?}"
    );
    let blocking_ids = body
        .pointer("/usability/blocking_provider_ids")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(
        blocking_ids
            .iter()
            .any(|value| value.as_str() == Some("acp-crp-bridge")),
        "expected blocking_provider_ids to include acp-crp-bridge: {body:#?}"
    );
    assert_eq!(
        body.pointer("/details/pending_dependency_ids")
            .and_then(serde_json::Value::as_str),
        Some("acp-crp-bridge"),
        "expected legacy pending_dependency_ids compatibility field: {body:#?}"
    );
}

#[tokio::test]
async fn workspace_options_use_workspace_target_status_for_acp_provider() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let repo = common::init_git_repo(&[("note.txt", "hello\n")]).await;
    let stores = common::setup_store(data_dir.path()).await;
    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:0",
    );
    let app = common::router(state.clone());

    let command_path = seed_container_only_install(data_dir.path(), "qwen").await;
    state
        .providers
        .statuses
        .lock()
        .await
        .insert("qwen".to_string(), bridge_missing_status("qwen"));
    state.providers.target_adapters.lock().await.insert(
        "qwen@container".to_string(),
        Arc::new(StatusOnlyAdapter {
            status: healthy_container_status("qwen", &command_path),
        }),
    );

    let ws = common::create_workspace(&app, repo.path(), "ws").await;
    let (host_status, host_body): (StatusCode, serde_json::Value) = common::json_request(
        &app,
        axum::http::Method::GET,
        format!("/api/workspaces/{}/providers/qwen/options", ws.id.0),
        None,
    )
    .await;

    assert_eq!(
        host_status,
        StatusCode::OK,
        "initial host-target options request failed: {host_body:#?}"
    );
    assert_eq!(
        host_body.get("installed").and_then(serde_json::Value::as_bool),
        Some(false),
        "host-target options should reflect unhealthy host status before switching targets: {host_body:#?}"
    );
    assert_eq!(
        host_body
            .get("probe_ok")
            .and_then(serde_json::Value::as_bool),
        Some(false),
        "host-target options should short-circuit unhealthy host state: {host_body:#?}"
    );

    let (cfg_status, cfg_body): (StatusCode, serde_json::Value) = common::json_request(
        &app,
        axum::http::Method::POST,
        format!("/api/workspaces/{}/execution_config", ws.id.0),
        Some(serde_json::json!({
            "environment": "container_host_mounted",
            "network_mode": "all",
        })),
    )
    .await;
    assert_eq!(
        cfg_status,
        StatusCode::OK,
        "execution config failed: {cfg_body:#?}"
    );

    let (status, body): (StatusCode, serde_json::Value) = common::json_request(
        &app,
        axum::http::Method::GET,
        format!("/api/workspaces/{}/providers/qwen/options", ws.id.0),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK, "options request failed: {body:#?}");
    assert_eq!(
        body.get("installed").and_then(serde_json::Value::as_bool),
        Some(true),
        "workspace options should use container-target installed state: {body:#?}"
    );
    assert!(
        body.get("probe_error").and_then(serde_json::Value::as_str)
            != Some("provider not installed or unhealthy"),
        "workspace options should not short-circuit on stale host status: {body:#?}"
    );
}
