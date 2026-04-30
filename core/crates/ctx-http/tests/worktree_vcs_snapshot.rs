mod common;

use std::collections::HashSet;
use std::path::Path;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use axum::http::{Method, StatusCode};
use ctx_core::models::{DiffUnavailableReason, WorktreeVcsFreshness};
use ctx_http::daemon::{AppRuntimeFlags, AppState};
use ctx_http::git_status::{
    emit_worktree_vcs_snapshot_for_worktree, refresh_worktree_vcs_summary,
    request_worktree_vcs_refresh, run_git_status_watcher,
};
use serde_json::Value;
use std::sync::Arc;
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

async fn git_stdout(root: &Path, args: &[&str]) -> String {
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
    String::from_utf8_lossy(&output.stdout).trim().to_string()
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

fn build_vcs_disabled_state(data_dir: &Path, stores: ctx_store::StoreManager) -> Arc<AppState> {
    Arc::new(AppState::new_with_runtime_flags(
        data_dir.to_path_buf(),
        stores,
        common::fake_providers(),
        "http://127.0.0.1:0".to_string(),
        None,
        None,
        AppRuntimeFlags {
            worktree_vcs_enabled: false,
        },
    ))
}

#[tokio::test(flavor = "current_thread")]
async fn worktree_vcs_disabled_mode_suppresses_projection_work() {
    let _guard = worktree_vcs_snapshot_test_lock().lock().await;
    let repo = common::init_git_repo(&[("file.txt", "hello\n")]).await;
    let data_dir = tempfile::tempdir().unwrap();
    let stores = common::setup_store(data_dir.path()).await;
    let state = build_vcs_disabled_state(data_dir.path(), stores);
    let app = common::router(state.clone());

    let ws = common::create_workspace(&app, repo.path(), "ws").await;
    let (_task, session) =
        common::create_task_with_session(&app, ws.id.0, "vcs-disabled", "fake", "fake-model").await;
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

    assert!(!state.worktree_vcs_enabled());
    assert!(!state.is_worktree_vcs_active(worktree.id).await);
    emit_worktree_vcs_snapshot_for_worktree(&state, &worktree, true)
        .await
        .expect("disabled VCS emission should be a no-op");
    request_worktree_vcs_refresh(&state, &worktree, true, true)
        .await
        .expect("disabled VCS refresh should be a no-op");
    run_git_status_watcher(state.clone(), worktree.clone())
        .await
        .expect("disabled VCS watcher should be a no-op");

    assert!(
        state.get_worktree_vcs_snapshot(worktree.id).await.is_none(),
        "disabled VCS mode must not compute or cache worktree VCS snapshots"
    );
    let active_snapshot = state
        .workspaces
        .workspace_active_snapshot
        .active_snapshot(ws.id, 10)
        .await;
    assert!(
        active_snapshot.worktree_vcs_snapshots.is_empty(),
        "disabled VCS mode must not publish worktree VCS snapshots"
    );
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
    let (_task, session) =
        common::create_task_with_session(&app, ws.id.0, "snapshot", "fake", "fake-model").await;
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
            if snapshot.summary.file_count.unwrap_or(0) > 0 {
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
    assert_eq!(
        snapshot.touched_files_state,
        ctx_core::models::WorktreeVcsTouchedFilesState::NotLoaded
    );

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
    assert_eq!(
        published.touched_files_state,
        ctx_core::models::WorktreeVcsTouchedFilesState::NotLoaded
    );
}

#[tokio::test(flavor = "current_thread")]
async fn worktree_vcs_snapshot_populates_jj_head_commit_metadata() {
    if !common::jj_available().await {
        eprintln!(
            "skipping worktree_vcs_snapshot_populates_jj_head_commit_metadata: jj not installed or too old"
        );
        return;
    }

    let _guard = worktree_vcs_snapshot_test_lock().lock().await;
    let repo = common::init_jj_repo(&[("file.txt", "hello\n")]).await;
    let data_dir = tempfile::tempdir().unwrap();
    let stores = common::setup_store(data_dir.path()).await;
    let state = common::build_state(
        data_dir.path(),
        stores,
        common::fake_providers(),
        "http://127.0.0.1:0",
    );
    let app = common::router(state.clone());

    let ws = common::create_workspace(&app, repo.path(), "jj-ws").await;
    let (_task, session) =
        common::create_task_with_session(&app, ws.id.0, "jj-vcs", "fake", "fake-model").await;
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
        .expect("snapshot emission should succeed for jj worktree");

    let expected_head = common::run_jj_output(
        repo.path(),
        &["log", "-r", "@", "-T", "commit_id ++ \"\\n\""],
    )
    .await
    .trim()
    .to_string();
    let snapshot = state
        .get_worktree_vcs_snapshot(worktree.id)
        .await
        .expect("snapshot should be present");
    assert_eq!(snapshot.head_commit_sha, expected_head);
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
    let workspace_store = state
        .store_for_workspace(ws.id)
        .await
        .expect("workspace store should open");
    let primary_branch = ctx_workspace_config::load_primary_branch(&workspace_store)
        .await
        .expect("loading primary branch should succeed")
        .expect("workspace primary branch should be configured");
    let (_task, session) =
        common::create_task_with_session(&app, ws.id.0, "snapshot-recovery", "fake", "fake-model")
            .await;
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
    let primary_ref = format!("refs/heads/{primary_branch}");
    run_git(worktree_root, &["symbolic-ref", "HEAD", primary_ref.as_str()]).await;
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
            panic!(
                "timed out waiting for recovered vcs snapshot to become available: {recovered:?}"
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn worktree_vcs_snapshot_noop_emit_preserves_freshness() {
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
    let (_task, session) =
        common::create_task_with_session(&app, ws.id.0, "noop-fresh", "fake", "fake-model").await;
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

    refresh_worktree_vcs_summary(state.clone(), worktree.clone())
        .await
        .expect("seed fresh snapshot");

    let seeded = state
        .get_worktree_vcs_snapshot(worktree.id)
        .await
        .expect("seeded snapshot should exist");
    assert_eq!(seeded.freshness, WorktreeVcsFreshness::Fresh);

    emit_worktree_vcs_snapshot_for_worktree(&state, &worktree, false)
        .await
        .expect("noop emit should succeed");

    let cached = state
        .get_worktree_vcs_snapshot(worktree.id)
        .await
        .expect("cached snapshot should remain present");
    assert_eq!(
        cached.freshness,
        WorktreeVcsFreshness::Fresh,
        "noop emit should preserve cached fresh snapshot"
    );

    let published = state
        .workspaces
        .workspace_active_snapshot
        .active_snapshot(ws.id, 10)
        .await
        .worktree_vcs_snapshots
        .into_iter()
        .find(|candidate| candidate.worktree_id == worktree.id)
        .expect("active snapshot should include worktree snapshot");
    assert_eq!(
        published.freshness,
        WorktreeVcsFreshness::Fresh,
        "noop emit should preserve published fresh snapshot"
    );
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
    let (_task, session) =
        common::create_task_with_session(&app, ws.id.0, "snapshot-eviction", "fake", "fake-model")
            .await;
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

#[tokio::test(flavor = "current_thread")]
async fn worktree_vcs_snapshot_watcher_recomputes_when_target_branch_ref_moves() {
    let _guard = worktree_vcs_snapshot_test_lock().lock().await;
    let repo = common::init_git_repo(&[("file.txt", "hello\n")]).await;
    run_git(repo.path(), &["branch", "merge-target"]).await;

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
    let (_status, _resp): (StatusCode, Value) = common::json_request(
        &app,
        Method::POST,
        format!("/api/workspaces/{}/primary_branch", ws.id.0),
        Some(serde_json::json!({ "primary_branch": "merge-target" })),
    )
    .await;

    let (_task, session) =
        common::create_task_with_session(&app, ws.id.0, "watcher-ref-move", "fake", "fake-model")
            .await;
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

    let watcher = tokio::spawn(run_git_status_watcher(state.clone(), worktree.clone()));

    let worktree_root = Path::new(&worktree.root_path);
    tokio::fs::write(worktree_root.join("file.txt"), "hello\nphase1\n")
        .await
        .expect("write changed file");
    emit_worktree_vcs_snapshot_for_worktree(&state, &worktree, true)
        .await
        .expect("initial vcs snapshot emission should succeed");

    let start = Instant::now();
    loop {
        let snapshot = state
            .get_worktree_vcs_snapshot(worktree.id)
            .await
            .expect("expected worktree vcs snapshot");
        if snapshot.summary.file_count.unwrap_or(0) > 0 {
            break;
        }
        if start.elapsed() > Duration::from_secs(30) {
            watcher.abort();
            panic!("timed out waiting for non-zero vcs summary");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    run_git(worktree_root, &["add", "file.txt"]).await;
    run_git(worktree_root, &["commit", "-m", "phase1"]).await;
    let output = Command::new("git")
        .arg("-C")
        .arg(worktree_root)
        .args(["rev-parse", "HEAD"])
        .output()
        .await
        .expect("read worktree head sha");
    assert!(output.status.success(), "rev-parse HEAD should succeed");
    let phase1_sha = String::from_utf8_lossy(&output.stdout).trim().to_string();
    run_git(repo.path(), &["branch", "-f", "merge-target", &phase1_sha]).await;

    let start = Instant::now();
    loop {
        let snapshot = state
            .get_worktree_vcs_snapshot(worktree.id)
            .await
            .expect("expected refreshed worktree vcs snapshot");
        if snapshot.summary.file_count.unwrap_or(-1) == 0 {
            watcher.abort();
            return;
        }
        if start.elapsed() > Duration::from_secs(30) {
            watcher.abort();
            panic!("timed out waiting for merge-target ref move to clear vcs summary");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn worktree_vcs_snapshot_preserves_head_when_configured_target_branch_disappears() {
    let _guard = worktree_vcs_snapshot_test_lock().lock().await;
    let repo = common::init_git_repo(&[("file.txt", "hello\n")]).await;
    run_git(repo.path(), &["branch", "merge-target"]).await;

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
    let workspace_store = state
        .store_for_workspace(ws.id)
        .await
        .expect("workspace store should open");
    ctx_workspace_config::update_primary_branch(&workspace_store, "merge-target")
        .await
        .expect("updating primary branch should succeed");

    let (_task, session) = common::create_task_with_session(
        &app,
        ws.id.0,
        "missing-target-head",
        "fake",
        "fake-model",
    )
    .await;
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
    tokio::fs::write(worktree_root.join("file.txt"), "hello\nadvanced\n")
        .await
        .expect("write changed file");
    run_git(worktree_root, &["add", "file.txt"]).await;
    run_git(worktree_root, &["commit", "-m", "advance head"]).await;
    let expected_head = git_stdout(worktree_root, &["rev-parse", "HEAD"]).await;
    assert_ne!(expected_head, worktree.base_commit_sha);

    run_git(repo.path(), &["branch", "-D", "merge-target"]).await;

    emit_worktree_vcs_snapshot_for_worktree(&state, &worktree, true)
        .await
        .expect("snapshot emission should succeed when target branch disappears");

    let snapshot = state
        .get_worktree_vcs_snapshot(worktree.id)
        .await
        .expect("snapshot should be present");
    assert!(!snapshot.available);
    assert_eq!(
        snapshot.unavailable_reason,
        Some(DiffUnavailableReason::NoTargetBranch)
    );
    assert_eq!(snapshot.base_commit_sha, worktree.base_commit_sha);
    assert_eq!(snapshot.head_commit_sha, expected_head);
    assert_eq!(snapshot.target_branch.as_deref(), Some("merge-target"));
    assert_eq!(snapshot.target_branch_commit_sha, None);
}
