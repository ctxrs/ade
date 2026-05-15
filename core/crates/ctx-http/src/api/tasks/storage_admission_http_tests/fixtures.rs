use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use serde_json::Value;
use tower::ServiceExt;

use ctx_core::ids::WorkspaceId;
use ctx_daemon::test_support::TestDaemon;
use ctx_providers::fake::FakeProviderAdapter;
use ctx_sandbox_materialization::set_test_preflight_storage_samples_override;
use ctx_settings_model::{ExecutionSettings, Settings};
use ctx_storage_admission::{
    StorageAdmissionOperation, StorageAdmissionSample, StorageGuardStatus,
};
use ctx_store::StoreManager;

pub(super) fn init_git_workspace(root: &Path) {
    git(&["init"], root);
    git(&["symbolic-ref", "HEAD", "refs/heads/main"], root);
    git(&["config", "user.email", "ctx@example.com"], root);
    git(&["config", "user.name", "Ctx Test"], root);
    std::fs::write(root.join("README.md"), "hello\n").expect("write readme");
    git(&["add", "README.md"], root);
    git(&["commit", "-m", "initial"], root);
}

fn git(args: &[&str], cwd: &Path) {
    let status = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .status()
        .expect("run git");
    assert!(status.success(), "git {args:?} failed");
}

pub(super) async fn test_state(data_root: &Path) -> TestDaemon {
    let mut providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
        HashMap::new();
    providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));
    TestDaemon::new(
        data_root.to_path_buf(),
        StoreManager::open(data_root).await.expect("open stores"),
        providers,
        "http://127.0.0.1:4311".to_string(),
        None,
    )
}

pub(super) async fn save_test_execution_settings(state: &TestDaemon, execution: ExecutionSettings) {
    ctx_settings_service::save_settings(
        state.global_store(),
        &Settings {
            execution: Some(execution),
            ..Default::default()
        },
    )
    .await
    .expect("save test execution settings");
}

pub(super) fn test_router(state: &TestDaemon) -> axum::Router {
    crate::api::router(state.handle())
}

pub(super) fn install_unreleased_host_reserve_storage_override(
    workspace_id: WorkspaceId,
) -> impl Drop {
    set_test_preflight_storage_samples_override(Arc::new(
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
    ))
}

pub(super) struct EnvVarGuard {
    key: &'static str,
    prev: Option<String>,
}

impl EnvVarGuard {
    pub(super) fn set(key: &'static str, value: &str) -> Self {
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

pub(super) async fn post_json(
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
