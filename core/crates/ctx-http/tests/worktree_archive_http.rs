use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::json;
use tokio::process::Command;

use ctx_core::models::{Task, Workspace};
use ctx_daemon::test_support::TestDaemon;
use ctx_fs::worktrees::managed_worktree_path;
use ctx_http::api;
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

    let daemon = TestDaemon::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:0".to_string(),
        None,
    );
    let app = api::router(daemon.handle());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base = format!("http://{addr}");
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
        .json(&json!({
            "title": "archive me",
            "default_session": {
                "provider_id":"fake",
                "model_id":"fake-model",
                "execution_environment":"host"
            }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let primary_session_id = task
        .primary_session_id
        .expect("task creation should create a primary session");

    let first_child_resp = client
        .post(format!("{base}/api/tasks/{}/sessions", task.id.0))
        .json(&json!({
            "provider_id":"fake",
            "model_id":"fake-model",
            "execution_environment":"host",
            "parent_session_id": primary_session_id.0.to_string(),
            "relationship": "sub_agent"
        }))
        .send()
        .await
        .unwrap();
    assert!(
        first_child_resp.status().is_success(),
        "first child session creation failed: {}",
        first_child_resp.status()
    );

    let second_child_resp = client
        .post(format!("{base}/api/tasks/{}/sessions", task.id.0))
        .json(&json!({
            "provider_id":"fake",
            "model_id":"fake-model",
            "execution_environment":"host",
            "parent_session_id": primary_session_id.0.to_string(),
            "relationship": "sub_agent"
        }))
        .send()
        .await
        .unwrap();
    assert!(
        second_child_resp.status().is_success(),
        "second child session creation failed: {}",
        second_child_resp.status()
    );

    let store = daemon.store_for_task(task.id).await.unwrap();
    let sessions = store.list_sessions_for_task(task.id).await.unwrap();
    assert_eq!(sessions.len(), 3);
    let task = store
        .get_task(task.id)
        .await
        .unwrap()
        .expect("task should still exist");
    let mut worktree_ids: HashSet<_> = sessions.iter().map(|session| session.worktree_id).collect();
    if let Some(primary_worktree_id) = task.primary_worktree_id {
        worktree_ids.insert(primary_worktree_id);
    }
    let expected_managed_count = worktree_ids.len();
    let mut managed = Vec::new();
    for worktree_id in worktree_ids {
        let worktree = store.get_worktree(worktree_id).await.unwrap().unwrap();
        let root = PathBuf::from(&worktree.root_path);
        let expected = managed_worktree_path(data_dir.path(), ws.id, worktree.id);
        if root == expected {
            managed.push(worktree);
        }
    }
    assert_eq!(managed.len(), expected_managed_count);

    let mut managed_roots: Vec<PathBuf> = managed
        .iter()
        .map(|worktree| PathBuf::from(&worktree.root_path))
        .collect();
    managed_roots.sort();
    managed_roots.dedup();
    let mut managed_branches: Vec<String> = managed
        .iter()
        .map(|worktree| worktree.git_branch.clone().unwrap())
        .collect();
    managed_branches.sort();
    managed_branches.dedup();

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
