#![allow(dead_code)]

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Method, Request, StatusCode};
use ctx_core::models::{Session, Task, Workspace};
use ctx_http::api;
use ctx_http::daemon::AppState;
use ctx_providers::adapters::ProviderAdapter;
use ctx_providers::fake::FakeProviderAdapter;
use ctx_store::StoreManager;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::process::Command;
use tokio::task::JoinHandle;
use tower::ServiceExt;

const JJ_MIN_VERSION: (u64, u64, u64) = (0, 25, 0);

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
    }
}

pub struct TestServer {
    pub base_url: String,
    pub client: reqwest::Client,
    handle: JoinHandle<()>,
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
