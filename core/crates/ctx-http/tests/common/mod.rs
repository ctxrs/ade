#![allow(dead_code)]

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use axum::body::{to_bytes, Body};
use axum::http::{Method, Request, StatusCode};
use ctx_core::models::{Session, Task, Workspace};
use ctx_http::api;
use ctx_http::daemon::AppState;
use ctx_managed_installs::{
    load_agent_server_config, save_agent_server_config, AgentServerCommand, AgentServerConfigFile,
    ManagedInstallMetadata,
};
use ctx_provider_install::install_state::InstallTarget;
use ctx_providers::adapters::ProviderAdapter;
use ctx_providers::fake::FakeProviderAdapter;
use ctx_store::StoreManager;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::process::Command;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinHandle;
use tower::ServiceExt;

pub mod crp_fixture_runtime;
pub mod openai_responses_stub;
pub mod updates_failure_safety;

const JJ_MIN_VERSION: (u64, u64, u64) = (0, 25, 0);

fn copied_test_binary_dir() -> &'static tempfile::TempDir {
    static DIR: OnceLock<tempfile::TempDir> = OnceLock::new();
    DIR.get_or_init(|| tempfile::tempdir().unwrap())
}

fn vcs_command_gate() -> &'static Semaphore {
    static GATE: OnceLock<Semaphore> = OnceLock::new();
    // Keep helper subprocess fan-out below local ulimit pressure during
    // concurrent integration test startup.
    GATE.get_or_init(|| Semaphore::new(2))
}

fn resolve_test_path(raw_path: &Path, kind: &str) -> PathBuf {
    let candidate = raw_path.to_path_buf();
    let mut searched: Vec<PathBuf> = Vec::new();

    if candidate.is_absolute() {
        if candidate.exists() {
            return std::fs::canonicalize(&candidate).unwrap_or(candidate);
        }
        searched.push(candidate);
    } else {
        searched.push(candidate.clone());
        for env_key in ["RUNFILES_DIR", "TEST_SRCDIR"] {
            let Some(base) = std::env::var_os(env_key) else {
                continue;
            };
            let base = PathBuf::from(base);
            searched.push(base.join(raw_path));
            if let Some(workspace) = std::env::var_os("TEST_WORKSPACE") {
                searched.push(base.join(workspace).join(raw_path));
            }
            searched.push(base.join("_main").join(raw_path));
        }
    }

    for path in &searched {
        if path.exists() {
            return std::fs::canonicalize(path).unwrap_or_else(|_| path.clone());
        }
    }

    panic!(
        "failed to resolve {kind} path {raw_path:?}; checked {searched:?}; RUNFILES_DIR={:?}; TEST_SRCDIR={:?}; TEST_WORKSPACE={:?}",
        std::env::var_os("RUNFILES_DIR"),
        std::env::var_os("TEST_SRCDIR"),
        std::env::var_os("TEST_WORKSPACE"),
    );
}

fn maybe_copy_test_binary(resolved: &Path) -> PathBuf {
    if !cfg!(target_os = "macos") {
        return resolved.to_path_buf();
    }
    let resolved_str = resolved.to_string_lossy();
    if !resolved_str.contains("/bazel-out/") && !resolved_str.contains("/bazel-bin/") {
        return resolved.to_path_buf();
    }

    let Some(file_name) = resolved.file_name().and_then(|name| name.to_str()) else {
        return resolved.to_path_buf();
    };
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    resolved.hash(&mut hasher);
    let suffix = hasher.finish();
    let copied = copied_test_binary_dir()
        .path()
        .join(format!("{suffix:016x}-{file_name}"));
    if copied.exists() {
        return copied;
    }

    std::fs::copy(resolved, &copied).unwrap_or_else(|err| {
        panic!("failed to copy test binary {resolved:?} to {copied:?}: {err}")
    });
    let permissions = std::fs::metadata(resolved)
        .unwrap_or_else(|err| panic!("failed to stat test binary {resolved:?}: {err}"))
        .permissions();
    std::fs::set_permissions(&copied, permissions).unwrap_or_else(|err| {
        panic!("failed to set copied test binary permissions {copied:?}: {err}")
    });
    copied
}

pub fn resolve_cargo_bin_exe(raw_path: &str) -> PathBuf {
    maybe_copy_test_binary(&resolve_test_path(Path::new(raw_path), "test binary"))
}

pub fn resolve_manifest_dir() -> PathBuf {
    resolve_test_path(Path::new(env!("CARGO_MANIFEST_DIR")), "manifest dir")
}

fn parse_jj_version(output: &str) -> Option<(u64, u64, u64)> {
    for token in output.split_whitespace() {
        let token = token.trim_start_matches('v');
        let mut version = String::new();
        let mut saw_digit = false;
        for ch in token.chars() {
            if ch.is_ascii_digit() {
                saw_digit = true;
                version.push(ch);
                continue;
            }
            if ch == '.' && saw_digit {
                version.push(ch);
                continue;
            }
            break;
        }
        if version.is_empty() {
            continue;
        }
        let parts = version.split('.').collect::<Vec<_>>();
        if parts.len() < 2 {
            continue;
        }
        let major = parts[0].parse().ok()?;
        let minor = parts[1].parse().ok()?;
        let patch = parts.get(2).and_then(|part| part.parse().ok()).unwrap_or(0);
        return Some((major, minor, patch));
    }
    None
}

pub async fn init_git_repo(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    run_git(root, &["init"]).await;
    run_git(root, &["config", "user.email", "test@example.com"]).await;
    run_git(root, &["config", "user.name", "Test"]).await;
    for (path, contents) in files {
        let p = root.join(path);
        if let Some(parent) = p.parent() {
            tokio::fs::create_dir_all(parent).await.unwrap();
        }
        tokio::fs::write(p, *contents).await.unwrap();
    }
    run_git(root, &["add", "."]).await;
    run_git(root, &["commit", "-m", "init"]).await;

    dir
}

pub async fn jj_available() -> bool {
    Command::new("jj")
        .arg("--version")
        .output()
        .await
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| parse_jj_version(&String::from_utf8_lossy(&output.stdout)))
        .map(|version| version >= JJ_MIN_VERSION)
        .unwrap_or(false)
}

pub async fn init_jj_repo(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_jj_repo_root(root).await;

    run_git(root, &["config", "user.email", "test@example.com"]).await;
    run_git(root, &["config", "user.name", "Test"]).await;
    for (path, contents) in files {
        let p = root.join(path);
        if let Some(parent) = p.parent() {
            tokio::fs::create_dir_all(parent).await.unwrap();
        }
        tokio::fs::write(p, *contents).await.unwrap();
    }
    run_git(root, &["add", "."]).await;
    run_git(root, &["commit", "-m", "init"]).await;
    run_git(root, &["branch", "-M", "main"]).await;
    run_jj(root, &["git", "import"]).await;

    dir
}

async fn init_jj_repo_root(root: &Path) {
    let candidates: &[&[&str]] = &[
        &["git", "init"],
        &["git", "init", "--colocate"],
        &["init", "--git"],
        &["init", "--git-repo", "."],
    ];
    let mut last_err = None;
    for args in candidates {
        match Command::new("jj")
            .current_dir(root)
            .args(*args)
            .output()
            .await
        {
            Ok(output) if output.status.success() => {
                assert!(root.join(".jj").exists());
                return;
            }
            Ok(output) => {
                last_err = Some(String::from_utf8_lossy(&output.stderr).to_string());
            }
            Err(err) => {
                last_err = Some(err.to_string());
            }
        }
    }
    panic!(
        "jj init failed: {}",
        last_err.unwrap_or_else(|| "unknown error".to_string())
    );
}

pub async fn setup_store(data_root: &Path) -> StoreManager {
    StoreManager::open(data_root).await.unwrap()
}

pub async fn seed_managed_codex_cli_host_runtime(data_root: &Path, command_abs_path: &Path) {
    assert!(
        command_abs_path.is_absolute(),
        "codex-cli runtime path must be absolute"
    );
    assert!(
        command_abs_path.exists(),
        "codex-cli runtime path must exist"
    );

    let command = std::fs::canonicalize(command_abs_path)
        .unwrap_or_else(|_| command_abs_path.to_path_buf())
        .to_string_lossy()
        .to_string();
    let meta = ManagedInstallMetadata {
        package: Some("codex-cli".to_string()),
        version: Some("fixture".to_string()),
        artifact_fingerprint: None,
        archive_sha256: None,
        target: Some(InstallTarget::Host),
        install_dir_rel: None,
        bin_dir_rel: None,
        last_success_at: None,
        last_error: None,
    };

    let mut cfg = load_agent_server_config(data_root)
        .await
        .unwrap_or_else(|_| AgentServerConfigFile::default());
    cfg.managed_installs
        .insert("codex-cli".to_string(), meta.clone());
    cfg.managed_provider_targets.insert(
        "codex-cli".to_string(),
        HashMap::from([(
            InstallTarget::Host.as_str().to_string(),
            AgentServerCommand {
                command,
                args: Vec::new(),
                dependencies: Vec::new(),
                managed: Some(meta),
            },
        )]),
    );
    save_agent_server_config(data_root, &cfg)
        .await
        .expect("save codex-cli managed runtime");
}

pub fn fake_providers() -> HashMap<String, Arc<dyn ProviderAdapter>> {
    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));
    providers
}

pub fn build_state(
    data_root: impl Into<std::path::PathBuf>,
    stores: StoreManager,
    providers: HashMap<String, Arc<dyn ProviderAdapter>>,
    base_url: impl Into<String>,
) -> Arc<AppState> {
    Arc::new(AppState::new(
        data_root.into(),
        stores,
        providers,
        base_url.into(),
        None,
    ))
}

pub async fn spawn_http_server(app: axum::Router) -> TestServer {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    TestServer {
        base_url: format!("http://{addr}"),
        client: reqwest::Client::new(),
        handle,
        _resource_permit: None,
    }
}

pub struct TestServer {
    pub base_url: String,
    pub client: reqwest::Client,
    handle: JoinHandle<()>,
    _resource_permit: Option<OwnedSemaphorePermit>,
}

impl TestServer {
    pub fn with_resource_permit(mut self, permit: OwnedSemaphorePermit) -> Self {
        self._resource_permit = Some(permit);
        self
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

pub fn router(state: Arc<AppState>) -> axum::Router {
    api::router(state)
}

pub async fn oneshot_json<T: DeserializeOwned>(
    app: &axum::Router,
    req: Request<Body>,
) -> (StatusCode, T) {
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let parsed = serde_json::from_slice(&body).unwrap_or_else(|err| {
        panic!(
            "failed to parse JSON response (status {}): {}\nbody: {}",
            status,
            err,
            String::from_utf8_lossy(&body)
        )
    });
    (status, parsed)
}

pub async fn oneshot_bytes(app: &axum::Router, req: Request<Body>) -> (StatusCode, Vec<u8>) {
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    (status, body.to_vec())
}

pub async fn json_request<T: DeserializeOwned>(
    app: &axum::Router,
    method: Method,
    uri: impl Into<String>,
    body: Option<Value>,
) -> (StatusCode, T) {
    let req = Request::builder()
        .method(method)
        .uri(uri.into())
        .header("content-type", "application/json")
        .body(Body::from(body.unwrap_or(Value::Null).to_string()))
        .unwrap();
    oneshot_json(app, req).await
}

pub async fn create_workspace(app: &axum::Router, root_path: &Path, name: &str) -> Workspace {
    let (status, ws) = json_request(
        app,
        Method::POST,
        "/api/workspaces",
        Some(serde_json::json!({"root_path": root_path.to_string_lossy(), "name": name})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    ws
}

pub async fn create_task(app: &axum::Router, workspace_id: uuid::Uuid, title: &str) -> Task {
    let (status, task) = json_request(
        app,
        Method::POST,
        format!("/api/workspaces/{workspace_id}/tasks"),
        Some(serde_json::json!({ "title": title })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    task
}

pub async fn create_session(
    app: &axum::Router,
    task_id: uuid::Uuid,
    provider_id: &str,
    model_id: &str,
) -> Session {
    let (status, session) = json_request(
        app,
        Method::POST,
        format!("/api/tasks/{task_id}/sessions"),
        Some(serde_json::json!({ "provider_id": provider_id, "model_id": model_id })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    session
}

pub fn fixed_uuid(seed: u128) -> uuid::Uuid {
    uuid::Uuid::from_u128(seed)
}

pub fn fixed_utc(offset_seconds: i64) -> chrono::DateTime<chrono::Utc> {
    let base = chrono::DateTime::from_timestamp(1735689600, 0).unwrap();
    base + chrono::Duration::seconds(offset_seconds)
}

pub async fn run_git(root: &Path, args: &[&str]) {
    let _permit = vcs_command_gate().acquire().await.unwrap();
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

pub async fn run_git_output(root: &Path, args: &[&str]) -> String {
    let _permit = vcs_command_gate().acquire().await.unwrap();
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
    String::from_utf8_lossy(&output.stdout).to_string()
}

pub async fn run_jj_output(root: &Path, args: &[&str]) -> String {
    let output = Command::new("jj")
        .arg("-R")
        .arg(root)
        .args(args)
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "jj {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).to_string()
}

pub async fn run_jj(root: &Path, args: &[&str]) {
    let _permit = vcs_command_gate().acquire().await.unwrap();
    let output = Command::new("jj")
        .arg("-R")
        .arg(root)
        .args(args)
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "jj {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(test)]
mod tests {
    use super::maybe_copy_test_binary;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn maybe_copy_test_binary_only_rehomes_bazel_paths_on_macos() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir
            .path()
            .join("bazel-out/darwin-fastbuild/bin/mock-binary");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, b"#!/bin/sh\nexit 0\n").unwrap();
        let mut permissions = std::fs::metadata(&source).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&source, permissions).unwrap();

        let resolved = maybe_copy_test_binary(&source);
        if cfg!(target_os = "macos") {
            assert_ne!(resolved, source);
            assert_eq!(std::fs::read(&resolved).unwrap(), b"#!/bin/sh\nexit 0\n");
            assert_eq!(
                std::fs::metadata(&resolved).unwrap().permissions().mode() & 0o111,
                0o111
            );
        } else {
            assert_eq!(resolved, source);
        }
    }
}
