use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;
use std::{env, fs};

use axum::Router;
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use tokio::process::Command;

use ctx_providers::fake::FakeProviderAdapter;
use ctx_store::StoreManager;

use ctx_http::api;
use ctx_http::daemon::AppState;

struct EnvVarGuard {
    key: &'static str,
    prev: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let prev = env::var_os(key);
        env::set_var(key, value);
        Self { key, prev }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(prev) = self.prev.as_ref() {
            env::set_var(self.key, prev);
        } else {
            env::remove_var(self.key);
        }
    }
}

#[derive(Debug, Deserialize)]
struct BufferOpenResp {
    buffer_id: String,
    version: u64,
    text: String,
}

#[derive(Debug, Deserialize)]
struct SessionGitStatusResponse {
    #[serde(default)]
    entries: Vec<SessionGitStatusEntry>,
}

#[derive(Debug, Deserialize)]
struct SessionGitStatusEntry {
    path: String,
}

#[derive(Debug, Deserialize)]
struct WorkspaceAttachmentResp {
    name: String,
    status: String,
}

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
    fs::write(root.join("file.txt"), "hello\n").unwrap();
    run_git(root, &["add", "."]).await;
    run_git(root, &["commit", "-m", "init"]).await;
    dir
}

fn should_run() -> bool {
    if !cfg!(target_os = "linux") {
        return false;
    }
    matches!(
        env::var("CTX_E2E_SANDBOX").ok().as_deref(),
        Some("1") | Some("true") | Some("yes")
    )
}

fn sandbox_cli_binary_for_tests() -> Option<std::path::PathBuf> {
    if let Some(raw) = env::var_os("CTX_HARNESS_SANDBOX_CLI_PATH") {
        let path = std::path::PathBuf::from(raw);
        if path.exists() {
            return Some(path);
        }
    }
    which::which("nerdctl").ok()
}

fn sandbox_cli_env_for_data_root(data_root: &Path) -> Vec<(String, String)> {
    let sandbox_root = data_root.join("sandbox");
    let xdg_root = sandbox_root.join("xdg");
    vec![
        (
            "XDG_CONFIG_HOME".to_string(),
            xdg_root.join("config").to_string_lossy().to_string(),
        ),
        (
            "XDG_DATA_HOME".to_string(),
            xdg_root.join("data").to_string_lossy().to_string(),
        ),
        (
            "XDG_RUNTIME_DIR".to_string(),
            sandbox_root.join("run").to_string_lossy().to_string(),
        ),
        (
            "HOME".to_string(),
            sandbox_root.join("home").to_string_lossy().to_string(),
        ),
        (
            "TMPDIR".to_string(),
            sandbox_root.join("tmp").to_string_lossy().to_string(),
        ),
        (
            "TMP".to_string(),
            sandbox_root.join("tmp").to_string_lossy().to_string(),
        ),
        (
            "TEMP".to_string(),
            sandbox_root.join("tmp").to_string_lossy().to_string(),
        ),
    ]
}

async fn sandbox_volume_exists(data_root: &Path, name: &str) -> bool {
    let Some(sandbox_cli) = sandbox_cli_binary_for_tests() else {
        return false;
    };
    let mut cmd = Command::new(sandbox_cli);
    cmd.arg("volume").arg("inspect").arg(name);
    for (k, v) in sandbox_cli_env_for_data_root(data_root) {
        cmd.env(k, v);
    }
    match cmd.output().await {
        Ok(out) => out.status.success(),
        Err(_) => false,
    }
}

#[tokio::test]
async fn disk_isolated_smoke_sandbox_volume_buffers_terminal() {
    if !should_run() {
        return;
    }
    if env::var("CTX_BUNDLE_DIR").ok().is_none() {
        // The daemon requires a bundled harness image tar for the default image when pulls are
        // disabled (release behavior). Running this smoke without a bundle isn't meaningful.
        return;
    }

    let Some(sandbox_cli) = sandbox_cli_binary_for_tests() else {
        return;
    };
    let _sandbox_cli_path =
        EnvVarGuard::set("CTX_HARNESS_SANDBOX_CLI_PATH", sandbox_cli.as_os_str());

    // Ensure the sandbox CLI is present before we spend time bootstrapping.
    if Command::new(&sandbox_cli)
        .arg("version")
        .output()
        .await
        .map(|o| !o.status.success())
        .unwrap_or(true)
    {
        return;
    }

    let git_repo = setup_git_repo().await;
    let host_root = git_repo.path().to_path_buf();
    let host_file_path = host_root.join("file.txt");
    let host_text_before = fs::read_to_string(&host_file_path).unwrap();

    let ref_repo = setup_git_repo().await;
    fs::write(ref_repo.path().join("ref.txt"), "refdata\n").unwrap();
    run_git(ref_repo.path(), &["add", "."]).await;
    run_git(ref_repo.path(), &["commit", "-m", "ref"]).await;

    let home = tempfile::tempdir().unwrap();
    let _home_guard = EnvVarGuard::set("HOME", home.path());

    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();

    let mut providers: HashMap<
        String,
        std::sync::Arc<dyn ctx_providers::adapters::ProviderAdapter>,
    > = HashMap::new();
    providers.insert(
        "fake".into(),
        std::sync::Arc::new(FakeProviderAdapter::new()),
    );

    let state = std::sync::Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:4399".to_string(),
        None,
    ));
    let app: Router = api::router(state.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base = format!("http://{addr}");
    let client = reqwest::Client::new();

    // Create workspace.
    let ws: ctx_core::models::Workspace = client
        .post(format!("{base}/api/workspaces"))
        .json(&json!({
            "root_path": host_root.to_string_lossy(),
            "name": "ws"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    // Configure disk-isolated container execution for the workspace.
    let _cfg: serde_json::Value = client
        .post(format!(
            "{base}/api/workspaces/{}/execution_config",
            ws.id.0
        ))
        .json(&json!({
            "environment": "sandbox",
            "network_mode": "all",
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    // Ensure harness container exists early (also exercises mount verification).
    let _container_status: serde_json::Value = client
        .post(format!(
            "{base}/api/workspaces/{}/harness_container/ensure",
            ws.id.0
        ))
        .json(&json!({}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    // Create task (creates default worktree + session).
    let task: ctx_core::models::Task = client
        .post(format!("{base}/api/workspaces/{}/tasks", ws.id.0))
        .json(&json!({"title":"t1"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let session_id = task
        .primary_session_id
        .expect("default session not created");
    let worktree_id = task
        .primary_worktree_id
        .expect("default worktree not created");

    // Verify worktree is container-rooted.
    let worktree: ctx_core::models::Worktree = client
        .get(format!("{base}/api/worktrees/{}", worktree_id.0))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        worktree.root_path.starts_with("/ctx/ws/worktrees/"),
        "expected container worktree root, got {}",
        worktree.root_path
    );

    // Buffer open + update should edit container FS only.
    let opened: BufferOpenResp = client
        .post(format!("{base}/api/buffers/open"))
        .json(&json!({
            "session_id": session_id.0,
            "path": "file.txt"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(opened.text, "hello\n");

    let new_text = "hello from container\n";
    let _ = client
        .post(format!("{base}/api/buffers/update"))
        .json(&json!({
            "buffer_id": opened.buffer_id,
            "version": opened.version,
            "text": new_text,
            "persist": true
        }))
        .send()
        .await
        .unwrap();

    let reopened: BufferOpenResp = client
        .post(format!("{base}/api/buffers/open"))
        .json(&json!({
            "session_id": session_id.0,
            "path": "file.txt"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(reopened.text, new_text);

    // Host FS remains unchanged.
    let host_text_after = fs::read_to_string(&host_file_path).unwrap();
    assert_eq!(host_text_after, host_text_before);

    // Attachments: daemon fetches via host git and imports into the disk-isolated volume.
    let _attachments: serde_json::Value = client
        .post(format!("{base}/api/workspaces/{}/attachments", ws.id.0))
        .json(&json!({
            "kind": "reference_repo",
            "name": "ref1",
            "source": ref_repo.path().to_string_lossy(),
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let attachment_ready = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let listed: Vec<WorkspaceAttachmentResp> = client
                .get(format!("{base}/api/workspaces/{}/attachments", ws.id.0))
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            if listed
                .iter()
                .any(|a| a.name == "ref1" && a.status == "ready")
            {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap_or(false);
    assert!(attachment_ready, "attachment did not become ready");

    let attachment_buf: BufferOpenResp = client
        .post(format!("{base}/api/buffers/open"))
        .json(&json!({
            "session_id": session_id.0,
            "path": ".ctx/attachments/refs/ref1/ref.txt"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(attachment_buf.text, "refdata\n");
    assert!(!host_root
        .join(".ctx/attachments/refs/ref1/ref.txt")
        .exists());

    // Terminal should run inside the container worktree.
    let term: ctx_core::models::TerminalSession = client
        .post(format!("{base}/api/workspaces/{}/terminals", ws.id.0))
        .json(&json!({
            "worktree_id": worktree_id.0,
            "shell": "/bin/bash",
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let ws_url = format!("ws://{}/api/terminals/{}/stream", addr, term.id.0);
    let (mut ws_stream, _) = tokio_tungstenite::connect_async(ws_url).await.unwrap();
    ws_stream
        .send(tokio_tungstenite::tungstenite::Message::Text(
            "pwd\n".into(),
        ))
        .await
        .unwrap();

    let expected_prefix = "/ctx/ws/worktrees/";
    let saw_pwd = tokio::time::timeout(Duration::from_secs(15), async {
        while let Some(Ok(frame)) = ws_stream.next().await {
            if let tokio_tungstenite::tungstenite::Message::Binary(bytes) = frame {
                let txt = String::from_utf8_lossy(&bytes);
                if txt.contains(expected_prefix) {
                    return true;
                }
            }
        }
        false
    })
    .await
    .unwrap_or(false);
    assert!(saw_pwd, "terminal output did not include {expected_prefix}");

    // Terminal write should affect container FS, and git status should reflect it.
    ws_stream
        .send(tokio_tungstenite::tungstenite::Message::Text(
            "printf 'term wrote\\n' > term_write.txt\ncat term_write.txt\n".into(),
        ))
        .await
        .unwrap();

    let saw_term_write = tokio::time::timeout(Duration::from_secs(15), async {
        while let Some(Ok(frame)) = ws_stream.next().await {
            if let tokio_tungstenite::tungstenite::Message::Binary(bytes) = frame {
                let txt = String::from_utf8_lossy(&bytes);
                if txt.contains("term wrote") {
                    return true;
                }
            }
        }
        false
    })
    .await
    .unwrap_or(false);
    assert!(
        saw_term_write,
        "terminal output did not include expected write"
    );

    let term_written: BufferOpenResp = client
        .post(format!("{base}/api/buffers/open"))
        .json(&json!({
            "session_id": session_id.0,
            "path": "term_write.txt"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(term_written.text, "term wrote\n");

    let status: SessionGitStatusResponse = client
        .get(format!("{base}/api/sessions/{}/git/status", session_id.0))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        status.entries.iter().any(|e| e.path == "term_write.txt"),
        "expected git status to include term_write.txt"
    );

    assert!(!host_root.join("term_write.txt").exists());

    // Deleting the workspace should clean up the disk-isolated volume.
    let vol_name = format!("ctx-ws-{}", ws.id.0);
    assert!(sandbox_volume_exists(data_dir.path(), &vol_name).await);
    let _ = client
        .delete(format!("{base}/api/workspaces/{}", ws.id.0))
        .send()
        .await
        .unwrap();
    // Give the async cleanup a brief moment.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!sandbox_volume_exists(data_dir.path(), &vol_name).await);
}
