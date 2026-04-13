use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use tower::ServiceExt;

use crate::daemon::AppState;
use crate::settings::{
    ContainerExecutionSettings, ContainerMountMode, ContainerNetworkMode, ContainerRuntimeKind,
    ExecutionMode, ExecutionSettings, Settings,
};
use ctx_core::models::VcsKind;
use ctx_sandbox_materialization::set_test_preflight_storage_samples_override;
use ctx_storage_admission::{StorageAdmissionOperation, StorageAdmissionSample};
use ctx_store::StoreManager;

use crate::storage_guard::StorageGuardStatus;

fn git(args: &[&str], cwd: &Path) {
    let status = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .status()
        .expect("run git");
    assert!(status.success(), "git {:?} failed", args);
}

fn init_git_workspace(root: &Path) {
    git(&["init"], root);
    git(&["symbolic-ref", "HEAD", "refs/heads/main"], root);
    git(&["config", "user.email", "ctx@example.com"], root);
    git(&["config", "user.name", "Ctx Test"], root);
    std::fs::write(root.join("README.md"), "hello\n").expect("write readme");
    git(&["add", "README.md"], root);
    git(&["commit", "-m", "initial"], root);
}

async fn test_state(data_root: &Path) -> Arc<AppState> {
    Arc::new(AppState::new(
        data_root.to_path_buf(),
        StoreManager::open(data_root).await.expect("open stores"),
        HashMap::new(),
        "http://127.0.0.1:4311".to_string(),
        None,
    ))
}

async fn save_test_execution_settings(state: &Arc<AppState>, execution: ExecutionSettings) {
    crate::settings::save_settings(
        state.global_store(),
        &Settings {
            execution: Some(execution),
            ..Default::default()
        },
    )
    .await
    .expect("save test execution settings");
}

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

async fn post_json(
    app: &axum::Router,
    uri: impl Into<String>,
    payload: Value,
) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("POST")
        .uri(uri.into())
        .header("content-type", "application/json")
        .body(Body::from(payload.to_string()))
        .expect("build request");
    let res = app.clone().oneshot(req).await.expect("run request");
    let status = res.status();
    let body = to_bytes(res.into_body(), usize::MAX)
        .await
        .expect("read body");
    let json = serde_json::from_slice(&body).unwrap_or_else(|err| {
        panic!(
            "failed to parse response JSON (status {}): {}\nbody: {}",
            status,
            err,
            String::from_utf8_lossy(&body)
        )
    });
    (status, json)
}

#[cfg(unix)]
#[tokio::test]
async fn create_task_rejects_before_disk_isolated_copy_when_host_reserve_is_unreleased() {
    let _serial = crate::test_support::sandbox_cli_env_test_lock()
        .lock()
        .await;
    let temp = tempfile::tempdir().expect("tempdir");
    let data_root = temp.path().join("data");
    let repo_root = temp.path().join("repo");
    std::fs::create_dir_all(&data_root).expect("create data root");
    std::fs::create_dir_all(&repo_root).expect("create repo root");
    init_git_workspace(&repo_root);

    let state = test_state(&data_root).await;
    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            repo_root.to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .expect("create workspace");

    let log_path = temp.path().join("sandbox-cli.log");
    let container_name = ctx_workspace_container::workspace_container_name(workspace.id);
    let sandbox_cli_path = crate::test_support::write_running_container_sandbox_cli_shim(
        temp.path(),
        &log_path,
        &container_name,
    );
    let _sandbox_cli = EnvVarGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );
    let _sandbox_cli_available = EnvVarGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "1");

    save_test_execution_settings(
        &state,
        ExecutionSettings {
            mode: ExecutionMode::Sandbox,
            container: ContainerExecutionSettings {
                runtime: ContainerRuntimeKind::NativeContainer,
                mount_mode: ContainerMountMode::DiskIsolated,
                network_mode: ContainerNetworkMode::All,
                image: Some("ctx/test-sandbox:latest".to_string()),
                ..ContainerExecutionSettings::default()
            },
        },
    )
    .await;

    let app = crate::api::router(Arc::clone(&state));
    let workspace_id = workspace.id;
    let _storage_override = set_test_preflight_storage_samples_override(Arc::new(
        move |data_root,
              _mode,
              container_id,
              _estimated_copy_bytes,
              destination_probe_root,
              operation,
              required_bytes| {
            assert_eq!(
                container_id,
                ctx_workspace_container::workspace_container_name(workspace_id)
            );
            assert_eq!(
                operation,
                StorageAdmissionOperation::DiskIsolatedWorktreeMaterialization
            );
            assert_eq!(
                destination_probe_root,
                Path::new(ctx_sandbox_contract::CTX_CONTAINER_WORKSPACE_ROOT)
            );

            let guard = StorageGuardStatus::default();
            let reserve = guard.reserve_bytes;
            let total_bytes = required_bytes
                .saturating_add(reserve)
                .saturating_add(guard.warning_threshold_bytes);
            Ok((
                StorageAdmissionSample {
                    label: "CTX data root".to_string(),
                    path: data_root.to_string_lossy().to_string(),
                    mount_point: "/".to_string(),
                    free_bytes: required_bytes.saturating_sub(1),
                    total_bytes,
                },
                StorageAdmissionSample {
                    label: "sandbox workspace volume".to_string(),
                    path: destination_probe_root.to_string_lossy().to_string(),
                    mount_point: ctx_sandbox_contract::CTX_CONTAINER_WORKSPACE_ROOT.to_string(),
                    free_bytes: required_bytes.saturating_add(reserve),
                    total_bytes,
                },
            ))
        },
    ));

    let (status, body) = post_json(
        &app,
        format!("/api/workspaces/{}/tasks", workspace.id.0),
        json!({ "title": "storage admission" }),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::INSUFFICIENT_STORAGE,
        "unexpected task creation response: {body:#?}"
    );
    let error = body
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(
        error.contains("Insufficient storage capacity"),
        "expected storage admission guidance, got: {body:#?}"
    );
    assert!(
        error.contains("isolated task worktree"),
        "expected task worktree admission message, got: {error}"
    );
    assert!(
        error.contains("CTX data root"),
        "expected failing host path label in error, got: {error}"
    );

    let log = std::fs::read_to_string(&log_path).unwrap_or_default();
    assert!(
        log.contains(&format!("volume inspect ctx-ws-{}", workspace.id.0)),
        "expected workspace volume preflight in sandbox log:\n{log}"
    );
    assert!(
        log.contains(&format!("volume create ctx-ws-{}", workspace.id.0)),
        "expected workspace volume creation in sandbox log:\n{log}"
    );
    assert!(
        log.contains(&format!("container inspect {container_name}")),
        "expected container existence check in sandbox log:\n{log}"
    );
    assert!(
        log.contains(&format!(
            "container inspect --format {{{{.State.Running}}}} {container_name}"
        )),
        "expected running-container check in sandbox log:\n{log}"
    );
    assert!(
        log.contains(&format!("inspect {container_name}")),
        "expected disk-isolated mount verification in sandbox log:\n{log}"
    );
    assert!(
        !log.contains(" tar -xf -"),
        "disk-isolated copy should not stream into the container after admission failure:\n{log}"
    );
    assert!(
        !log.contains(" mkdir -p -- /ctx/ws/worktrees/"),
        "disk-isolated worktree root should not be created after admission failure:\n{log}"
    );
    assert!(
        !log.contains(" git checkout -B "),
        "disk-isolated checkout should not run after admission failure:\n{log}"
    );
    assert!(
        !data_root.join("disk-isolated").join("staging").exists(),
        "host-side staging root should not be created when admission rejects early"
    );
}
