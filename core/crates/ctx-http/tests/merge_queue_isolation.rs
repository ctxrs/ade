use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::http::{Method, StatusCode};
use ctx_core::ids::MergeQueueEntryId;
use ctx_core::models::{MergeQueueEntry, MergeQueueEntryStatus};
use ctx_fs::git::git_status_porcelain;
use ctx_http::daemon::AppState;
use ctx_http::merge_queue;
use serde_json::json;
use tokio::process::Command;

mod common;

async fn write_merge_queue_config(
    root: &Path,
    target_branch: &str,
    canonical_sync: &str,
) -> PathBuf {
    let ctx_dir = root.join(".ctx");
    tokio::fs::create_dir_all(&ctx_dir).await.unwrap();
    let config_path = ctx_dir.join("config.toml");
    let config = format!(
        "[merge_queue]\n\
enabled = true\n\
target_branch = \"{target_branch}\"\n\
verify_commands = [\"true\"]\n\
push_on_success = false\n\
canonical_sync = \"{canonical_sync}\"\n"
    );
    tokio::fs::write(&config_path, config).await.unwrap();
    config_path
}

async fn append_file(path: &Path, text: &str) {
    let mut contents = tokio::fs::read_to_string(path).await.unwrap_or_default();
    contents.push_str(text);
    tokio::fs::write(path, contents).await.unwrap();
}

async fn git_output(root: &Path, args: &[&str]) -> String {
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
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

async fn wait_for_entry(state: &Arc<AppState>, entry_id: MergeQueueEntryId) -> MergeQueueEntry {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let entry = merge_queue::get_merge_queue_entry(state, entry_id)
            .await
            .unwrap();
        match entry.status {
            MergeQueueEntryStatus::Queued | MergeQueueEntryStatus::Running => {
                if Instant::now() > deadline {
                    panic!("merge queue entry timed out: {:?}", entry.status);
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            _ => return entry,
        }
    }
}

#[tokio::test]
async fn merge_queue_isolation_and_canonical_sync() {
    let repo = common::init_git_repo(&[("note.txt", "base\n")]).await;
    let target_branch = git_output(repo.path(), &["rev-parse", "--abbrev-ref", "HEAD"]).await;
    write_merge_queue_config(repo.path(), &target_branch, "never").await;

    let data_dir = tempfile::tempdir().unwrap();
    let stores = common::setup_store(data_dir.path()).await;
    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        common::fake_providers(),
        "http://127.0.0.1:0",
    );
    let app = common::router(state.clone());
    merge_queue::spawn_merge_queue_runner(state.clone());

    let workspace = common::create_workspace(&app, repo.path(), "mq-test").await;

    let feature_path = repo.path().join("feature");
    common::run_git(
        repo.path(),
        &[
            "worktree",
            "add",
            feature_path.to_str().unwrap(),
            "-b",
            "feature",
        ],
    )
    .await;

    append_file(&feature_path.join("note.txt"), "mq-1\n").await;
    common::run_git(&feature_path, &["add", "note.txt"]).await;
    common::run_git(&feature_path, &["commit", "-m", "mq-1"]).await;

    let base_head = git_output(repo.path(), &["rev-parse", "HEAD"]).await;
    let feature_head = git_output(&feature_path, &["rev-parse", "HEAD"]).await;
    let store = state.store_for_workspace(workspace.id).await.unwrap();
    let worktree = store
        .create_worktree(
            workspace.id,
            feature_path.to_string_lossy().to_string(),
            feature_head,
            Some("feature".to_string()),
        )
        .await
        .unwrap();

    let (status, entry): (StatusCode, MergeQueueEntry) = common::json_request(
        &app,
        Method::POST,
        "/api/merge-queue/entries",
        Some(json!({
            "worktree_id": worktree.id.0.to_string(),
            "message": "mq-1",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let entry = wait_for_entry(&state, entry.id).await;
    assert_eq!(entry.status, MergeQueueEntryStatus::Passed);

    let canonical_head = git_output(repo.path(), &["rev-parse", "HEAD"]).await;
    assert_eq!(canonical_head, base_head);

    let merge_queue_repo = repo.path().join(".ctx/merge-queue/repo");
    let mq_head = git_output(&merge_queue_repo, &["rev-parse", "HEAD"]).await;
    assert_ne!(mq_head, base_head);

    write_merge_queue_config(repo.path(), &target_branch, "clean_only").await;
    append_file(&feature_path.join("note.txt"), "mq-2\n").await;
    common::run_git(&feature_path, &["add", "note.txt"]).await;
    common::run_git(&feature_path, &["commit", "-m", "mq-2"]).await;

    let (status, entry): (StatusCode, MergeQueueEntry) = common::json_request(
        &app,
        Method::POST,
        "/api/merge-queue/entries",
        Some(json!({
            "worktree_id": worktree.id.0.to_string(),
            "message": "mq-2",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let entry = wait_for_entry(&state, entry.id).await;
    assert_eq!(entry.status, MergeQueueEntryStatus::Passed);
    let canonical_head = git_output(repo.path(), &["rev-parse", "HEAD"]).await;
    let expected = entry.result_commit_sha.clone().unwrap();
    assert_eq!(canonical_head, expected);

    append_file(&repo.path().join("note.txt"), "dirty\n").await;
    append_file(&feature_path.join("note.txt"), "mq-3\n").await;
    common::run_git(&feature_path, &["add", "note.txt"]).await;
    common::run_git(&feature_path, &["commit", "-m", "mq-3"]).await;

    let (status, entry): (StatusCode, MergeQueueEntry) = common::json_request(
        &app,
        Method::POST,
        "/api/merge-queue/entries",
        Some(json!({
            "worktree_id": worktree.id.0.to_string(),
            "message": "mq-3",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let entry = wait_for_entry(&state, entry.id).await;
    assert_eq!(entry.status, MergeQueueEntryStatus::Passed);

    let canonical_dirty = git_status_porcelain(repo.path()).await.unwrap();
    assert!(!canonical_dirty.is_empty());

    let canonical_after = git_output(repo.path(), &["rev-parse", "HEAD"]).await;
    assert_eq!(canonical_after, expected);

    let run = store
        .get_latest_merge_queue_run(entry.id)
        .await
        .unwrap()
        .unwrap();
    let log_path = run.log_path.unwrap();
    let log_contents = tokio::fs::read_to_string(log_path).await.unwrap();
    assert!(log_contents.contains("canonical sync skipped"));
}
