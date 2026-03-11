mod common;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use axum::http::StatusCode;
use ctx_http::api;
use ctx_http::daemon::AppState;
use ctx_http::installer::{
    save_agent_server_config, AgentServerCommand, AgentServerConfigFile, ManagedInstallMetadata,
};
use ctx_http::provider_accounts::add_copilot_account;
use ctx_providers::adapters::{ProviderAdapter, ProviderHealth, ProviderStatus};
use ctx_store::StoreManager;

struct EnvVarGuard {
    key: &'static str,
    prev: Option<String>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let prev = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, prev }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(prev) = self.prev.take() {
            std::env::set_var(self.key, prev);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

#[cfg(unix)]
fn write_executable(path: &Path, contents: &str) {
    use std::os::unix::fs::PermissionsExt;

    std::fs::write(path, contents).expect("write executable");
    let mut perms = std::fs::metadata(path).expect("metadata").permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).expect("set permissions");
}

#[cfg(unix)]
fn write_fake_podman(path: &Path) {
    write_executable(
        path,
        r#"#!/bin/sh
echo "fake podman unavailable" >&2
exit 1
"#,
    );
}

#[cfg(unix)]
fn setup_runtime_command_with_managed_interpreter(
    data_root: &Path,
    provider_id: &str,
) -> (String, String) {
    let dep_bin_rel = format!("managed/runtime-node-{provider_id}/bin");
    let dep_bin_dir = data_root.join(&dep_bin_rel);
    std::fs::create_dir_all(&dep_bin_dir).expect("create dep bin dir");

    let interpreter_name = format!("ctx-managed-probe-node-{provider_id}");
    let interpreter = dep_bin_dir.join(&interpreter_name);
    write_executable(
        &interpreter,
        r#"#!/bin/sh
while IFS= read -r line; do
  case "$line" in
    *'"type":"models.list"'*)
      echo '{"seq":1,"channel":"control","type":"models.list","models":[{"id":"fixture-model"}],"current_model_id":"fixture-model"}'
      exit 0
      ;;
  esac
done
exit 1
"#,
    );

    let runtime_dir = data_root
        .join("providers")
        .join("agent-servers")
        .join(provider_id)
        .join("fixture")
        .join("dist")
        .join("bin");
    std::fs::create_dir_all(&runtime_dir).expect("create runtime dir");
    let runtime_cmd = runtime_dir.join(format!("{provider_id}-acp.js"));
    write_executable(
        &runtime_cmd,
        &format!("#!/usr/bin/env {interpreter_name}\n// fixture runtime\n"),
    );

    (runtime_cmd.to_string_lossy().to_string(), dep_bin_rel)
}

async fn app_state(data_root: &Path) -> Arc<AppState> {
    let stores = StoreManager::open(data_root).await.expect("open stores");
    let providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    Arc::new(AppState::new(
        data_root.to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:0".to_string(),
        None,
    ))
}

async fn seed_runtime_and_status(
    state: &Arc<AppState>,
    provider_id: &str,
    runtime_cmd: String,
    dep_bin_rel: String,
) {
    let dep_id = "runtime-node-host".to_string();
    let mut cfg = AgentServerConfigFile::default();
    cfg.providers.insert(
        provider_id.to_string(),
        AgentServerCommand {
            command: runtime_cmd,
            args: Vec::new(),
            dependencies: vec![dep_id.clone()],
            managed: None,
        },
    );
    cfg.managed_installs.insert(
        dep_id,
        ManagedInstallMetadata {
            package: Some("node-runtime".to_string()),
            version: Some("fixture".to_string()),
            target: None,
            install_dir_rel: None,
            bin_dir_rel: Some(dep_bin_rel),
            last_success_at: None,
            last_error: None,
        },
    );
    save_agent_server_config(&state.core.data_root, &cfg)
        .await
        .expect("save runtime config");

    state.providers.statuses.lock().await.insert(
        provider_id.to_string(),
        ProviderStatus {
            provider_id: provider_id.to_string(),
            installed: true,
            detected_path: None,
            version: None,
            capabilities: None,
            health: ProviderHealth::Ok,
            diagnostics: Vec::new(),
            details: HashMap::new(),
        },
    );
}

#[cfg(unix)]
#[tokio::test]
async fn provider_options_probe_uses_managed_dependency_path() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let repo = common::init_git_repo(&[("note.txt", "hello\n")]).await;
    let state = app_state(data_dir.path()).await;
    let app = api::router(state.clone());

    let (runtime_cmd, dep_bin_rel) =
        setup_runtime_command_with_managed_interpreter(data_dir.path(), "codex");
    seed_runtime_and_status(&state, "codex", runtime_cmd, dep_bin_rel).await;

    let ws = common::create_workspace(&app, repo.path(), "ws").await;
    let (status, body): (StatusCode, serde_json::Value) = common::json_request(
        &app,
        axum::http::Method::GET,
        format!("/api/workspaces/{}/providers/codex/options", ws.id.0),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK, "options request failed: {body:#?}");
    assert_eq!(
        body.get("probe_ok").and_then(serde_json::Value::as_bool),
        Some(true),
        "expected probe_ok=true with managed runtime path injection: {body:#?}"
    );
    assert_eq!(
        body.pointer("/models/current_model_id")
            .and_then(serde_json::Value::as_str),
        Some("fixture-model"),
        "expected fixture model probe result: {body:#?}"
    );
}

#[tokio::test]
async fn copilot_provider_options_include_pinned_model_catalog() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let repo = common::init_git_repo(&[("note.txt", "hello\n")]).await;
    let state = app_state(data_dir.path()).await;
    let app = api::router(state.clone());

    add_copilot_account(
        data_dir.path(),
        Some("Copilot Test".to_string()),
        "gho_fixture_token".to_string(),
        Some("copilot@example.com".to_string()),
    )
    .await
    .expect("add copilot account");

    state.providers.statuses.lock().await.insert(
        "copilot".to_string(),
        ProviderStatus {
            provider_id: "copilot".to_string(),
            installed: true,
            detected_path: None,
            version: Some("1.0.3".to_string()),
            capabilities: None,
            health: ProviderHealth::Ok,
            diagnostics: Vec::new(),
            details: HashMap::new(),
        },
    );

    let ws = common::create_workspace(&app, repo.path(), "ws").await;
    let (status, body): (StatusCode, serde_json::Value) = common::json_request(
        &app,
        axum::http::Method::GET,
        format!("/api/workspaces/{}/providers/copilot/options", ws.id.0),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK, "options request failed: {body:#?}");
    assert_eq!(
        body.get("probe_ok").and_then(serde_json::Value::as_bool),
        Some(true),
        "expected probe_ok=true with active copilot auth: {body:#?}"
    );
    assert_eq!(
        body.get("has_active_auth")
            .and_then(serde_json::Value::as_bool),
        Some(true),
        "expected has_active_auth=true for active copilot account: {body:#?}"
    );
    assert_eq!(
        body.pointer("/models/catalog_source")
            .and_then(serde_json::Value::as_str),
        Some("copilot_version_pinned"),
        "expected pinned copilot model catalog in provider options: {body:#?}"
    );
    assert_eq!(
        body.pointer("/models/current_model_id")
            .and_then(serde_json::Value::as_str),
        Some("gpt-5-mini"),
        "expected bootstrap-safe copilot model in provider options: {body:#?}"
    );
    assert_eq!(
        body.pointer("/models/default_model_id")
            .and_then(serde_json::Value::as_str),
        Some("claude-sonnet-4.6"),
        "expected default copilot model in provider options: {body:#?}"
    );
}

#[tokio::test]
async fn providers_bootstrap_includes_pinned_codex_and_claude_catalogs() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let repo = common::init_git_repo(&[("note.txt", "hello\n")]).await;
    let state = app_state(data_dir.path()).await;
    let app = api::router(state.clone());

    state.providers.statuses.lock().await.insert(
        "codex".to_string(),
        ProviderStatus {
            provider_id: "codex".to_string(),
            installed: true,
            detected_path: None,
            version: Some("0.98.0".to_string()),
            capabilities: None,
            health: ProviderHealth::Ok,
            diagnostics: Vec::new(),
            details: HashMap::new(),
        },
    );
    state.providers.statuses.lock().await.insert(
        "claude-crp".to_string(),
        ProviderStatus {
            provider_id: "claude-crp".to_string(),
            installed: true,
            detected_path: None,
            version: Some("2.1.47".to_string()),
            capabilities: None,
            health: ProviderHealth::Ok,
            diagnostics: Vec::new(),
            details: HashMap::new(),
        },
    );

    let ws = common::create_workspace(&app, repo.path(), "ws").await;
    let (status, body): (StatusCode, serde_json::Value) = common::json_request(
        &app,
        axum::http::Method::GET,
        format!("/api/workspaces/{}/providers/bootstrap", ws.id.0),
        None,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "bootstrap request failed: {body:#?}"
    );
    assert_eq!(
        body.pointer("/provider_options/codex/models/meta/catalog_source")
            .and_then(serde_json::Value::as_str),
        Some("codex_bundle_pinned"),
        "expected pinned codex bootstrap catalog: {body:#?}"
    );
    assert_eq!(
        body.pointer("/provider_options/codex/models/current_model_id")
            .and_then(serde_json::Value::as_str),
        Some("gpt-5.4/medium"),
        "expected pinned codex bootstrap current model: {body:#?}"
    );
    assert_eq!(
        body.pointer("/provider_options/claude-crp/models/meta/catalog_source")
            .and_then(serde_json::Value::as_str),
        Some("claude_subscription_pinned"),
        "expected pinned claude bootstrap catalog: {body:#?}"
    );
    assert_eq!(
        body.pointer("/provider_options/claude-crp/models/current_model_id")
            .and_then(serde_json::Value::as_str),
        Some("default/medium"),
        "expected pinned claude bootstrap current model: {body:#?}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn provider_verify_probe_uses_managed_dependency_path() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let repo = common::init_git_repo(&[("note.txt", "hello\n")]).await;
    let state = app_state(data_dir.path()).await;
    let app = api::router(state.clone());

    let (runtime_cmd, dep_bin_rel) =
        setup_runtime_command_with_managed_interpreter(data_dir.path(), "codex");
    seed_runtime_and_status(&state, "codex", runtime_cmd, dep_bin_rel).await;

    let ws = common::create_workspace(&app, repo.path(), "ws").await;
    let (status, body): (StatusCode, serde_json::Value) = common::json_request(
        &app,
        axum::http::Method::POST,
        format!("/api/workspaces/{}/providers/codex/verify", ws.id.0),
        Some(serde_json::json!({})),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "verify request failed: {body:#?}");
    assert_eq!(
        body.get("status").and_then(serde_json::Value::as_str),
        Some("ok"),
        "expected verify status ok with managed runtime path injection: {body:#?}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn provider_options_probe_uses_workspace_runtime_context_for_container_mode() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let repo = common::init_git_repo(&[("note.txt", "hello\n")]).await;
    let state = app_state(data_dir.path()).await;
    let app = api::router(state.clone());

    let fake_podman = data_dir.path().join("podman");
    write_fake_podman(&fake_podman);
    let _podman_guard = EnvVarGuard::set(
        "CTX_PODMAN_PATH",
        fake_podman
            .to_str()
            .expect("fake podman path should be utf-8"),
    );

    let (runtime_cmd, dep_bin_rel) =
        setup_runtime_command_with_managed_interpreter(data_dir.path(), "codex");
    seed_runtime_and_status(&state, "codex", runtime_cmd, dep_bin_rel).await;

    let ws = common::create_workspace(&app, repo.path(), "ws").await;
    let (cfg_status, cfg_body): (StatusCode, serde_json::Value) = common::json_request(
        &app,
        axum::http::Method::POST,
        format!("/api/workspaces/{}/execution_config", ws.id.0),
        Some(serde_json::json!({
            "environment": "container_disk_isolated",
            "network_mode": "all",
        })),
    )
    .await;
    assert_eq!(
        cfg_status,
        StatusCode::OK,
        "execution config request failed: {cfg_body:#?}"
    );

    let (status, body): (StatusCode, serde_json::Value) = common::json_request(
        &app,
        axum::http::Method::GET,
        format!("/api/workspaces/{}/providers/codex/options", ws.id.0),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK, "options request failed: {body:#?}");
    assert_eq!(
        body.get("probe_ok").and_then(serde_json::Value::as_bool),
        Some(false),
        "container-mode probe must use workspace runtime context; host probe success is invalid: {body:#?}"
    );
    let diagnostics = body
        .get("diagnostics")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(
        diagnostics.iter().any(|value| value
            .as_str()
            .unwrap_or_default()
            .contains("does not verify target 'container'")),
        "expected explicit target mismatch diagnostic in container mode probe: {body:#?}"
    );
    assert_eq!(
        body.get("probe_error").and_then(serde_json::Value::as_str),
        Some("provider not installed or unhealthy"),
        "expected generic probe_error alongside target mismatch diagnostic: {body:#?}"
    );
}
