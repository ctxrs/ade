use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use futures::{SinkExt, StreamExt};
use serde_json::json;
use tokio::process::Command;
use tower::ServiceExt;

use ctx_execution_runtime::{ExecutionLaunchSnapshot, ExecutionLaunchState, ExecutionSetupJobKind};
use ctx_providers::adapters::ProviderStatus;
use ctx_providers::fake::FakeProviderAdapter;
use ctx_store::StoreManager;

use crate::api;
use crate::daemon::AppState;
use crate::storage_guard::{StorageGuardLevel, StorageGuardPathStatus, StorageGuardStatus};

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
    std::fs::write(root.join("file.txt"), "hello\n").unwrap();
    run_git(root, &["add", "."]).await;
    run_git(root, &["commit", "-m", "init"]).await;
    dir
}

async fn create_workspace_via_api(
    app: &axum::Router,
    root_path: &str,
) -> ctx_core::models::Workspace {
    let req = Request::builder()
        .method("POST")
        .uri("/api/workspaces")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "root_path": root_path,
                "name": "ws"
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

fn write_failing_sandbox_cli_shim(dir: &Path) -> std::path::PathBuf {
    let path = dir.join(if cfg!(windows) {
        "sandbox-cli-fail-fast.cmd"
    } else {
        "sandbox-cli-fail-fast.sh"
    });
    let script = if cfg!(windows) {
        "@echo off\r\n>&2 echo sandbox CLI unavailable\r\nexit /b 125\r\n"
    } else {
        "#!/bin/sh\necho 'sandbox CLI unavailable' >&2\nexit 125\n"
    };
    std::fs::write(&path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
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
        if let Some(v) = self.prev.take() {
            std::env::set_var(self.key, v);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

fn sandbox_cli_env_test_lock() -> &'static tokio::sync::Mutex<()> {
    crate::test_support::sandbox_cli_env_test_lock()
}

fn home_env_test_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

mod auth_boundaries;
mod cors;
mod daemon_smoke;
mod execution_launch;
mod health_diagnostics;
mod log_path_boundaries;
mod mobile_profile_routes;
mod mobile_secure_routes;
mod provider_routes;
mod session_artifacts;
mod session_head_large_http;
mod telemetry_export_boundaries;
mod update_boundaries;
mod web_session_routes;
mod workspace_active_routes;
