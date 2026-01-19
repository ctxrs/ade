use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::json;
use tokio::process::Command;

use ctx_core::models::{Task, Workspace};
use ctx_fs::worktrees::managed_worktree_path;
use ctx_http::api;
use ctx_http::daemon::AppState;
use ctx_providers::fake::FakeProviderAdapter;
use ctx_store::StoreManager;

async fn setup_git_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    run_git(root, &["init"]).await;
    run_git(root, &["config", "user.email", "test@example.com"]).await;
    run_git(root, &["config", "user.name", "Test"]).await;
    tokio::fs::write(root.join("file.txt"), "hello\n")
        .await
        .unwrap();
    run_git(root, &["add", "."]).await;
    run_git(root, &["commit", "-m", "init"]).await;
    dir
}

async fn run_git(root: &Path, args: &[&str]) -> String {
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

async fn git_worktree_list(root: &Path) -> String {
    run_git(root, &["worktree", "list", "--porcelain"]).await
}

async fn branch_exists(root: &Path, branch: &str) -> bool {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ])
        .output()
        .await
        .unwrap();
    output.status.success()
}

#[tokio::test]
async fn archive_and_unarchive_recreates_managed_worktrees() {
    let repo = setup_git_repo().await;
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();

    let mut providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
        HashMap::new();
    providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));

    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:0".to_string(),
        None,
    ));
    state.start_workspace_active_snapshot_listener();
    let app = api::router(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base = format!("http://{}", addr);
    let client = reqwest::Client::new();

    let ws: Workspace = client
        .post(format!("{base}/api/workspaces"))
        .json(&json!({"root_path": repo.path(), "name": "ws"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let task: Task = client
        .post(format!("{base}/api/workspaces/{}/tasks", ws.id.0))
        .json(&json!({"title":"archive me"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    client
        .post(format!("{base}/api/tasks/{}/sessions", task.id.0))
        .json(&json!({"provider_id":"fake","model_id":"fake-model","env_target":"worktree"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("{base}/api/tasks/{}/sessions", task.id.0))
        .json(&json!({"provider_id":"fake","model_id":"fake-model","env_target":"worktree"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("{base}/api/tasks/{}/sessions", task.id.0))
        .json(&json!({"provider_id":"fake","model_id":"fake-model","env_target":"local"}))
        .send()
        .await
        .unwrap();

    let store = state.store_for_task(task.id).await.unwrap();
    let sessions = store.list_sessions_for_task(task.id).await.unwrap();
    let mut managed = Vec::new();
    let mut local_roots = Vec::new();
    for session in sessions {
        let worktree = store
            .get_worktree(session.worktree_id)
            .await
            .unwrap()
            .unwrap();
        let root = PathBuf::from(&worktree.root_path);
        let expected = managed_worktree_path(data_dir.path(), ws.id, worktree.id);
        if root == expected {
            managed.push(worktree);
        } else {
            local_roots.push(root);
        }
    }
    assert!(managed.len() >= 2);

    let managed_roots: Vec<PathBuf> = managed
        .iter()
        .map(|worktree| PathBuf::from(&worktree.root_path))
        .collect();
    let managed_branches: Vec<String> = managed
        .iter()
        .map(|worktree| worktree.git_branch.clone().unwrap())
        .collect();

    tokio::fs::write(managed_roots[0].join("dirty.txt"), "dirty")
        .await
        .unwrap();

    let list_before = git_worktree_list(repo.path()).await;
    for root in &managed_roots {
        let root_str = root.to_string_lossy();
        assert!(list_before.contains(root_str.as_ref()));
    }
    for branch in &managed_branches {
        assert!(branch_exists(repo.path(), branch).await);
    }

    let resp = client
        .post(format!("{base}/api/tasks/{}/archive", task.id.0))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());

    for root in &managed_roots {
        assert!(tokio::fs::metadata(root).await.is_err());
    }
    let list_archived = git_worktree_list(repo.path()).await;
    for root in &managed_roots {
        let root_str = root.to_string_lossy();
        assert!(!list_archived.contains(root_str.as_ref()));
    }
    for local_root in local_roots {
        assert!(tokio::fs::metadata(local_root).await.is_ok());
    }

    for branch in &managed_branches {
        assert!(!branch_exists(repo.path(), branch).await);
    }

    let resp = client
        .post(format!("{base}/api/tasks/{}/unarchive", task.id.0))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());

    let list_after = git_worktree_list(repo.path()).await;
    for root in &managed_roots {
        assert!(tokio::fs::metadata(root).await.is_ok());
        let root_str = root.to_string_lossy();
        assert!(list_after.contains(root_str.as_ref()));
    }
    for branch in &managed_branches {
        assert!(branch_exists(repo.path(), branch).await);
    }

    server.abort();
}
