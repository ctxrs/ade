use std::path::{Path, PathBuf};

use tempfile::TempDir;
use tokio::process::Command;

use ctx_core::ids::WorktreeId;
use ctx_core::models::{VcsKind, Worktree};
use ctx_fs::git::rev_parse_head;
use ctx_http::attachments;
use ctx_http::daemon::AppState;
use ctx_store::StoreManager;

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

fn copy_dir_recursive(src: &Path, dest: &Path) {
    std::fs::create_dir_all(dest).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let ty = entry.file_type().unwrap();
        let target = dest.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &target);
        } else if ty.is_file() {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn fixture_root() -> PathBuf {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("core/fixtures/workspace-attachments-demo");
    repo_root
}

#[tokio::test]
#[ignore]
async fn attachments_demo_react_smoketest() {
    let temp = TempDir::new().unwrap();
    let ws_root = temp.path().join("workspace");
    copy_dir_recursive(&fixture_root(), &ws_root);

    run_git(&ws_root, &["init"]).await;
    run_git(&ws_root, &["config", "user.email", "test@example.com"]).await;
    run_git(&ws_root, &["config", "user.name", "Test"]).await;
    std::fs::write(ws_root.join("README.md"), "demo\n").unwrap();
    run_git(&ws_root, &["add", "."]).await;
    run_git(&ws_root, &["commit", "-m", "init"]).await;

    let data_dir = TempDir::new().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();

    let ws = stores
        .global()
        .create_workspace(
            "demo".to_string(),
            ws_root.to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();
    let store = stores.workspace(ws.id).await.unwrap();
    let base_commit_sha = rev_parse_head(&ws_root).await.unwrap();
    let worktree = Worktree {
        id: WorktreeId::new(),
        workspace_id: ws.id,
        root_path: ws_root.to_string_lossy().to_string(),
        base_commit_sha,
        vcs_kind: Some(VcsKind::Git),
        base_revision: None,
        vcs_ref: None,
        git_branch: None,
        created_at: chrono::Utc::now(),
        bootstrap_status: None,
        bootstrap_started_at: None,
        bootstrap_finished_at: None,
        bootstrap_exit_code: None,
        bootstrap_timeout_sec: None,
        bootstrap_error: None,
        bootstrap_log_path: None,
        bootstrap_log_truncated: None,
        bootstrap_config_path: None,
        bootstrap_config_key: None,
        bootstrap_command: None,
        bootstrap_script_path: None,
    };
    store.insert_worktree(worktree.clone()).await.unwrap();
    let _task = store
        .create_task(ws.id, "demo".to_string(), None)
        .await
        .unwrap();

    let providers: std::collections::HashMap<
        String,
        std::sync::Arc<dyn ctx_providers::adapters::ProviderAdapter>,
    > = std::collections::HashMap::new();
    let state = std::sync::Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:0".to_string(),
        None,
    ));

    attachments::sync_workspace_attachments(std::sync::Arc::clone(&state), &ws, false)
        .await
        .unwrap();
    let mounts =
        attachments::ensure_worktree_attachment_mounts(state.as_ref(), &ws, &worktree, true)
            .await
            .unwrap();

    assert!(!mounts.is_empty());
    assert!(ws_root.join(".ctx/attachments/refs/react").exists());
    assert!(ws_root.join(".ctx/attachments/docs/react-docs").exists());
}
