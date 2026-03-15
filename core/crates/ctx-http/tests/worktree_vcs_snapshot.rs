mod common;

use std::collections::HashSet;
use std::path::Path;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use ctx_core::models::DiffUnavailableReason;
use ctx_http::git_status::emit_worktree_vcs_snapshot_for_worktree;
use tokio::process::Command;

async fn run_git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .await
        .expect("run git command");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn remove_git_marker(root: &Path) {
    let git_path = root.join(".git");
    let Ok(metadata) = tokio::fs::metadata(&git_path).await else {
        return;
    };
    if metadata.is_dir() {
        tokio::fs::remove_dir_all(&git_path)
            .await
            .expect("remove .git directory");
    } else {
        tokio::fs::remove_file(&git_path)
            .await
            .expect("remove .git file");
    }
}

async fn poison_worktree_git_marker(worktree_root: &Path) {
    remove_git_marker(worktree_root).await;
    tokio::fs::write(
        worktree_root.join(".git"),
        "gitdir: /definitely/missing/ctx-test-gitdir\n",
    )
    .await
    .expect("write poisoned .git marker");
}

fn worktree_vcs_snapshot_test_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

#[tokio::test(flavor = "current_thread")]
async fn worktree_vcs_snapshot_clears_stale_counts_when_repo_becomes_unavailable() {
    let _guard = worktree_vcs_snapshot_test_lock().lock().await;
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
    let _ = repo;
    poison_worktree_git_marker(worktree_root).await;

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

    let active_snapshot = state
        .workspaces
        .workspace_active_snapshot
        .active_snapshot(ws.id, 10)
        .await;
    let published = active_snapshot
        .worktree_vcs_snapshots
        .into_iter()
        .find(|candidate| candidate.worktree_id == worktree.id)
        .expect("workspace active snapshot should include worktree vcs snapshot");
    assert!(!published.available);
    assert_eq!(
        published.unavailable_reason,
        Some(DiffUnavailableReason::NoRepo)
    );
    assert_eq!(published.summary.file_count, None);
    assert!(published.touched_files.items.is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn worktree_vcs_snapshot_recovers_when_repo_is_reinitialized() {
    let _guard = worktree_vcs_snapshot_test_lock().lock().await;
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
    let task = common::create_task(&app, ws.id.0, "snapshot-recovery").await;
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

    let worktree_root = Path::new(&worktree.root_path);
    let _ = repo;
    poison_worktree_git_marker(worktree_root).await;

    emit_worktree_vcs_snapshot_for_worktree(&state, &worktree, true)
        .await
        .expect("no-repo snapshot emission should succeed");
    let unavailable = state
        .get_worktree_vcs_snapshot(worktree.id)
        .await
        .expect("unavailable snapshot should be present");
    assert!(!unavailable.available);
    assert_eq!(
        unavailable.unavailable_reason,
        Some(DiffUnavailableReason::NoRepo)
    );

    remove_git_marker(worktree_root).await;
    run_git(worktree_root, &["init"]).await;
    run_git(worktree_root, &["config", "user.email", "test@example.com"]).await;
    run_git(worktree_root, &["config", "user.name", "Test"]).await;
    run_git(worktree_root, &["add", "."]).await;
    run_git(worktree_root, &["commit", "-m", "restore"]).await;

    emit_worktree_vcs_snapshot_for_worktree(&state, &worktree, true)
        .await
        .expect("recovered repo snapshot emission should succeed");

    let start = Instant::now();
    loop {
        let recovered = state
            .get_worktree_vcs_snapshot(worktree.id)
            .await
            .expect("recovered snapshot should be present");
        if recovered.available {
            assert_eq!(recovered.unavailable_reason, None);

            let active_snapshot = state
                .workspaces
                .workspace_active_snapshot
                .active_snapshot(ws.id, 10)
                .await;
            let published = active_snapshot
                .worktree_vcs_snapshots
                .into_iter()
                .find(|candidate| candidate.worktree_id == worktree.id)
                .expect("workspace active snapshot should include recovered vcs snapshot");
            assert!(published.available);
            assert_eq!(published.unavailable_reason, None);
            break;
        }

        if start.elapsed() > Duration::from_secs(5) {
            panic!("timed out waiting for recovered vcs snapshot to become available");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn worktree_vcs_snapshot_does_not_repopulate_cache_after_activity_eviction() {
    let _guard = worktree_vcs_snapshot_test_lock().lock().await;
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
    let task = common::create_task(&app, ws.id.0, "snapshot-eviction").await;
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

    emit_worktree_vcs_snapshot_for_worktree(&state, &worktree, true)
        .await
        .expect("initial snapshot emission should succeed");
    assert!(
        state.get_worktree_vcs_snapshot(worktree.id).await.is_some(),
        "expected initial active snapshot to populate cache",
    );

    state
        .workspaces
        .update_worktree_vcs_activity(&next_active, &HashSet::new())
        .await;
    assert!(
        state.get_worktree_vcs_snapshot(worktree.id).await.is_none(),
        "expected activity eviction to clear worktree vcs cache",
    );

    emit_worktree_vcs_snapshot_for_worktree(&state, &worktree, true)
        .await
        .expect("inactive snapshot emission should not fail");
    assert!(
        state.get_worktree_vcs_snapshot(worktree.id).await.is_none(),
        "inactive emit should not recreate worktree vcs cache",
    );

    let active_snapshot = state
        .workspaces
        .workspace_active_snapshot
        .active_snapshot(ws.id, 10)
        .await;
    assert!(
        active_snapshot
            .worktree_vcs_snapshots
            .into_iter()
            .all(|candidate| candidate.worktree_id != worktree.id),
        "inactive emit should not repopulate workspace active snapshot",
    );
}
