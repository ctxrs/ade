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

const MERGE_QUEUE_CONFLICT_MESSAGE: &str = concat!(
    "Your merge queue submission produces conflicts with the current head. ",
    "Please rebase your changes, carefully considering the intent of your changes and the intent of the upstream changes. ",
    "If in doubt about how to resolve conflicts, please ask for help."
);

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

async fn git_success(root: &Path, args: &[&str]) -> bool {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .await
        .unwrap();
    output.status.success()
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
async fn merge_queue_accepts_unrebased_changes() {
    let repo = common::init_git_repo(&[
        ("note.txt", "base\n"),
        (".gitignore", ".ctx/merge-queue\n.ctx/config.toml\n"),
    ])
    .await;
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

    let workspace = common::create_workspace(&app, repo.path(), "mq-unrebased").await;

    let worktree_root = tempfile::tempdir().unwrap();
    let feature_path = worktree_root.path().join("feature");
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

    append_file(&feature_path.join("note.txt"), "feature\n").await;
    common::run_git(&feature_path, &["add", "note.txt"]).await;
    common::run_git(&feature_path, &["commit", "-m", "feature"]).await;

    append_file(&repo.path().join("target.txt"), "target\n").await;
    common::run_git(repo.path(), &["add", "target.txt"]).await;
    common::run_git(repo.path(), &["commit", "-m", "target"]).await;

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
    state
        .global_store()
        .upsert_workspace_worktree_index(worktree.id, workspace.id)
        .await
        .unwrap();

    let (status, entry): (StatusCode, MergeQueueEntry) = common::json_request(
        &app,
        Method::POST,
        "/api/merge-queue/entries",
        Some(json!({
            "worktree_id": worktree.id.0.to_string(),
            "message": "mq-unrebased",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let entry = wait_for_entry(&state, entry.id).await;
    assert_eq!(entry.status, MergeQueueEntryStatus::Passed);
    let merge_queue_repo = repo.path().join(".ctx/merge-queue/repo");
    let mq_head = git_output(&merge_queue_repo, &["rev-parse", &target_branch]).await;
    assert_eq!(mq_head, entry.result_commit_sha.clone().unwrap());
}

#[tokio::test]
async fn merge_queue_conflict_message_and_cleanup() {
    let repo = common::init_git_repo(&[
        ("note.txt", "base\n"),
        (".gitignore", ".ctx/merge-queue\n.ctx/config.toml\n"),
    ])
    .await;
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

    let workspace = common::create_workspace(&app, repo.path(), "mq-conflict").await;

    let worktree_root = tempfile::tempdir().unwrap();
    let feature_path = worktree_root.path().join("feature");
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

    tokio::fs::write(feature_path.join("note.txt"), "feature\n")
        .await
        .unwrap();
    common::run_git(&feature_path, &["add", "note.txt"]).await;
    common::run_git(&feature_path, &["commit", "-m", "feature"]).await;

    tokio::fs::write(repo.path().join("note.txt"), "target\n")
        .await
        .unwrap();
    common::run_git(repo.path(), &["add", "note.txt"]).await;
    common::run_git(repo.path(), &["commit", "-m", "target"]).await;

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
    state
        .global_store()
        .upsert_workspace_worktree_index(worktree.id, workspace.id)
        .await
        .unwrap();

    let (status, _body): (StatusCode, serde_json::Value) = common::json_request(
        &app,
        Method::POST,
        "/api/merge-queue/entries",
        Some(json!({
            "worktree_id": worktree.id.0.to_string(),
            "message": "mq-conflict",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let entries = store
        .list_merge_queue_entries(workspace.id, Some(1))
        .await
        .unwrap();
    let entry = entries
        .into_iter()
        .next()
        .expect("expected merge queue entry");
    let entry = wait_for_entry(&state, entry.id).await;
    assert_eq!(entry.status, MergeQueueEntryStatus::Conflict);
    assert_eq!(
        entry.error_message.as_deref(),
        Some(MERGE_QUEUE_CONFLICT_MESSAGE)
    );

    let worktree_path = repo
        .path()
        .join(".ctx/merge-queue/worktrees")
        .join(workspace.id.0.to_string())
        .join(entry.id.0.to_string());
    assert!(!worktree_path.exists());

    let merge_queue_repo = repo.path().join(".ctx/merge-queue/repo");
    let branch_ref = format!("refs/heads/ctx-merge-queue/{}", entry.id.0);
    let branch_exists = git_success(
        &merge_queue_repo,
        &["show-ref", "--verify", "--quiet", &branch_ref],
    )
    .await;
    assert!(!branch_exists);
}

#[tokio::test]
async fn merge_queue_isolation_and_canonical_sync() {
    let repo = common::init_git_repo(&[
        ("note.txt", "base\n"),
        (".gitignore", ".ctx/merge-queue\n.ctx/config.toml\n"),
    ])
    .await;
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

    let worktree_root = tempfile::tempdir().unwrap();
    let feature1_path = worktree_root.path().join("feature-1");
    common::run_git(
        repo.path(),
        &[
            "worktree",
            "add",
            feature1_path.to_str().unwrap(),
            "-b",
            "feature",
        ],
    )
    .await;

    append_file(&feature1_path.join("note.txt"), "mq-1\n").await;
    common::run_git(&feature1_path, &["add", "note.txt"]).await;
    common::run_git(&feature1_path, &["commit", "-m", "mq-1"]).await;

    let base_head = git_output(repo.path(), &["rev-parse", "HEAD"]).await;
    let feature_head = git_output(&feature1_path, &["rev-parse", "HEAD"]).await;
    let store = state.store_for_workspace(workspace.id).await.unwrap();
    let worktree1 = store
        .create_worktree(
            workspace.id,
            feature1_path.to_string_lossy().to_string(),
            feature_head,
            Some("feature".to_string()),
        )
        .await
        .unwrap();
    state
        .global_store()
        .upsert_workspace_worktree_index(worktree1.id, workspace.id)
        .await
        .unwrap();

    let (status, entry): (StatusCode, MergeQueueEntry) = common::json_request(
        &app,
        Method::POST,
        "/api/merge-queue/entries",
        Some(json!({
            "worktree_id": worktree1.id.0.to_string(),
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
    let mq_head = git_output(&merge_queue_repo, &["rev-parse", &target_branch]).await;
    assert_ne!(mq_head, base_head);
    common::run_git(
        repo.path(),
        &[
            "fetch",
            merge_queue_repo.to_str().unwrap(),
            &format!("refs/heads/{target_branch}"),
        ],
    )
    .await;
    common::run_git(repo.path(), &["reset", "--hard", "FETCH_HEAD"]).await;
    let canonical_synced = git_output(repo.path(), &["rev-parse", "HEAD"]).await;
    assert_eq!(canonical_synced, mq_head);

    let feature2_path = worktree_root.path().join("feature-2");
    common::run_git(
        repo.path(),
        &[
            "worktree",
            "add",
            feature2_path.to_str().unwrap(),
            "-b",
            "feature-2",
        ],
    )
    .await;

    write_merge_queue_config(repo.path(), &target_branch, "clean_only").await;
    append_file(&feature2_path.join("note.txt"), "mq-2\n").await;
    common::run_git(&feature2_path, &["add", "note.txt"]).await;
    common::run_git(&feature2_path, &["commit", "-m", "mq-2"]).await;

    let feature2_head = git_output(&feature2_path, &["rev-parse", "HEAD"]).await;
    let worktree2 = store
        .create_worktree(
            workspace.id,
            feature2_path.to_string_lossy().to_string(),
            feature2_head,
            Some("feature-2".to_string()),
        )
        .await
        .unwrap();
    state
        .global_store()
        .upsert_workspace_worktree_index(worktree2.id, workspace.id)
        .await
        .unwrap();

    let (status, entry): (StatusCode, MergeQueueEntry) = common::json_request(
        &app,
        Method::POST,
        "/api/merge-queue/entries",
        Some(json!({
            "worktree_id": worktree2.id.0.to_string(),
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

    let feature3_path = worktree_root.path().join("feature-3");
    common::run_git(
        repo.path(),
        &[
            "worktree",
            "add",
            feature3_path.to_str().unwrap(),
            "-b",
            "feature-3",
        ],
    )
    .await;
    append_file(&feature3_path.join("note.txt"), "mq-3\n").await;
    common::run_git(&feature3_path, &["add", "note.txt"]).await;
    common::run_git(&feature3_path, &["commit", "-m", "mq-3"]).await;
    let feature3_head = git_output(&feature3_path, &["rev-parse", "HEAD"]).await;
    let worktree3 = store
        .create_worktree(
            workspace.id,
            feature3_path.to_string_lossy().to_string(),
            feature3_head,
            Some("feature-3".to_string()),
        )
        .await
        .unwrap();
    state
        .global_store()
        .upsert_workspace_worktree_index(worktree3.id, workspace.id)
        .await
        .unwrap();

    append_file(&repo.path().join("note.txt"), "dirty\n").await;

    let (status, entry): (StatusCode, MergeQueueEntry) = common::json_request(
        &app,
        Method::POST,
        "/api/merge-queue/entries",
        Some(json!({
            "worktree_id": worktree3.id.0.to_string(),
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
