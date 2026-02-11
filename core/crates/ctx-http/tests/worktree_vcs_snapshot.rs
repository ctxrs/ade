mod common;

use std::collections::HashSet;
use std::path::Path;
use std::time::{Duration, Instant};

use ctx_core::models::DiffUnavailableReason;
use ctx_http::git_status::emit_worktree_vcs_snapshot_for_worktree;

#[tokio::test]
async fn worktree_vcs_snapshot_clears_stale_counts_when_repo_becomes_unavailable() {
    let repo = common::init_git_repo(&[("file.txt", "hello\n")]).await;
    let data_dir = tempfile::tempdir().unwrap();
    let stores = common::setup_store(data_dir.path()).await;
    let state = common::build_state(
        data_dir.path(),
        stores,
        common::fake_providers(),
        "http://127.0.0.1:0",
    );
    let app = common::router(state.clone());

    let ws = common::create_workspace(&app, repo.path(), "ws").await;
    let task = common::create_task(&app, ws.id.0, "snapshot").await;
    let session = common::create_session(&app, task.id.0, "fake", "fake-model").await;
    let worktree = state
        .store_for_worktree(session.worktree_id)
        .await
        .expect("store for worktree")
        .get_worktree(session.worktree_id)
        .await
        .expect("load worktree")
        .expect("worktree should exist");
    let mut next_active = HashSet::new();
    next_active.insert(worktree.id);
    state
        .workspaces
        .update_worktree_vcs_activity(&HashSet::new(), &next_active)
        .await;
    tokio::fs::write(
        Path::new(&worktree.root_path).join("file.txt"),
        "hello\nchanged\n",
    )
    .await
    .expect("write changed file");
    emit_worktree_vcs_snapshot_for_worktree(&state, &worktree, true)
        .await
        .expect("initial snapshot emission should succeed");
    let start = Instant::now();
    loop {
        let snapshot = state.get_worktree_vcs_snapshot(worktree.id).await;
        if let Some(snapshot) = snapshot {
            if snapshot.summary.file_count.unwrap_or(0) > 0
                || snapshot.summary.line_count.unwrap_or(0) > 0
            {
                break;
            }
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!("timed out waiting for non-empty vcs summary");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let worktree_root = Path::new(&worktree.root_path);
    let git_path = worktree_root.join(".git");
    let metadata = tokio::fs::metadata(&git_path)
        .await
        .expect("worktree .git should exist");
    if metadata.is_dir() {
        tokio::fs::remove_dir_all(&git_path).await.unwrap();
    } else {
        tokio::fs::remove_file(&git_path).await.unwrap();
    }

    emit_worktree_vcs_snapshot_for_worktree(&state, &worktree, true)
        .await
        .expect("snapshot emission should not fail for no-repo");

    let snapshot = state
        .get_worktree_vcs_snapshot(worktree.id)
        .await
        .expect("snapshot should be present");
    assert!(!snapshot.available);
    assert_eq!(
        snapshot.unavailable_reason,
        Some(DiffUnavailableReason::NoRepo)
    );
    assert_eq!(snapshot.summary.file_count, None);
    assert_eq!(snapshot.summary.line_additions, None);
    assert_eq!(snapshot.summary.line_deletions, None);
    assert!(snapshot.touched_files.items.is_empty());
}
