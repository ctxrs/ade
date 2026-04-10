use super::*;
use chrono::TimeDelta;
use ctx_providers::adapters::ProviderAdapter;
use ctx_store::StoreManager;
use ctx_workspace_config::{update_merge_queue_config, MergeQueueConfigUpdate};

async fn setup_state() -> (tempfile::TempDir, Arc<AppState>) {
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:0".to_string(),
        None,
    ));
    (data_dir, state)
}

async fn create_workspace(
    state: &Arc<AppState>,
    data_dir: &tempfile::TempDir,
    name: &str,
) -> Workspace {
    let root = data_dir.path().join(name);
    tokio::fs::create_dir_all(&root).await.unwrap();
    state
        .global_store()
        .create_workspace(
            name.to_string(),
            root.to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap()
}

fn queued_entry(workspace_id: WorkspaceId, name: &str) -> MergeQueueEntry {
    let now = Utc::now();
    MergeQueueEntry {
        id: MergeQueueEntryId::new(),
        workspace_id,
        worktree_id: None,
        session_id: None,
        target_branch: "main".to_string(),
        message: Some(name.to_string()),
        patch_source: MergeQueuePatchSource::Generated,
        base_commit_sha: Some(format!("{name}-base")),
        head_commit_sha: Some(format!("{name}-head")),
        patch_path: format!("/tmp/{name}.patch"),
        patch_size: 1,
        status: MergeQueueEntryStatus::Queued,
        result_commit_sha: None,
        error_message: None,
        created_at: now,
        updated_at: now,
    }
}

async fn wait_for_entry_status<F>(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    entry_id: MergeQueueEntryId,
    predicate: F,
    timeout: Duration,
) -> MergeQueueEntry
where
    F: Fn(MergeQueueEntryStatus) -> bool,
{
    tokio::time::timeout(timeout, async {
        loop {
            let store = state.core.stores.workspace(workspace_id).await.unwrap();
            let entry = store
                .get_merge_queue_entry(entry_id)
                .await
                .unwrap()
                .unwrap();
            if predicate(entry.status.clone()) {
                break entry;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("timed out waiting for merge queue entry status")
}

#[tokio::test]
async fn list_queued_entries_and_entry_lookup_are_workspace_scoped() {
    let (_data_dir, state) = setup_state().await;
    let workspace_a = state
        .global_store()
        .create_workspace("a".to_string(), "/tmp/a".to_string(), VcsKind::Git)
        .await
        .unwrap();
    let workspace_b = state
        .global_store()
        .create_workspace("b".to_string(), "/tmp/b".to_string(), VcsKind::Git)
        .await
        .unwrap();
    let store_a = state.store_for_workspace(workspace_a.id).await.unwrap();
    let store_b = state.store_for_workspace(workspace_b.id).await.unwrap();

    let base_time = Utc::now();
    let queued_a = MergeQueueEntry {
        id: MergeQueueEntryId::new(),
        workspace_id: workspace_a.id,
        worktree_id: None,
        session_id: None,
        target_branch: "main".to_string(),
        message: Some("queued-a".to_string()),
        patch_source: MergeQueuePatchSource::Generated,
        base_commit_sha: Some("base-a".to_string()),
        head_commit_sha: Some("head-a".to_string()),
        patch_path: "/tmp/queued-a.patch".to_string(),
        patch_size: 10,
        status: MergeQueueEntryStatus::Queued,
        result_commit_sha: None,
        error_message: None,
        created_at: base_time,
        updated_at: base_time,
    };
    let passed_b = MergeQueueEntry {
        id: MergeQueueEntryId::new(),
        workspace_id: workspace_b.id,
        worktree_id: None,
        session_id: None,
        target_branch: "main".to_string(),
        message: Some("passed-b".to_string()),
        patch_source: MergeQueuePatchSource::Generated,
        base_commit_sha: Some("base-b".to_string()),
        head_commit_sha: Some("head-b".to_string()),
        patch_path: "/tmp/passed-b.patch".to_string(),
        patch_size: 11,
        status: MergeQueueEntryStatus::Passed,
        result_commit_sha: Some("result-b".to_string()),
        error_message: None,
        created_at: base_time + TimeDelta::milliseconds(10),
        updated_at: base_time + TimeDelta::milliseconds(10),
    };
    let queued_b = MergeQueueEntry {
        id: MergeQueueEntryId::new(),
        workspace_id: workspace_b.id,
        worktree_id: None,
        session_id: None,
        target_branch: "main".to_string(),
        message: Some("queued-b".to_string()),
        patch_source: MergeQueuePatchSource::Generated,
        base_commit_sha: Some("base-c".to_string()),
        head_commit_sha: Some("head-c".to_string()),
        patch_path: "/tmp/queued-b.patch".to_string(),
        patch_size: 12,
        status: MergeQueueEntryStatus::Queued,
        result_commit_sha: None,
        error_message: None,
        created_at: base_time + TimeDelta::milliseconds(20),
        updated_at: base_time + TimeDelta::milliseconds(20),
    };

    store_a.create_merge_queue_entry(&queued_a).await.unwrap();
    store_b.create_merge_queue_entry(&passed_b).await.unwrap();
    store_b.create_merge_queue_entry(&queued_b).await.unwrap();

    let looked_up = get_workspace_merge_queue_entry(state.as_ref(), workspace_b.id, queued_b.id)
        .await
        .unwrap();
    assert_eq!(looked_up.id.0, queued_b.id.0);
    assert_eq!(looked_up.workspace_id.0, workspace_b.id.0);

    let queued_a_entries = list_queued_entries_for_workspace(state.as_ref(), workspace_a.id)
        .await
        .unwrap();
    assert_eq!(queued_a_entries.len(), 1);
    assert_eq!(queued_a_entries[0].id.0, queued_a.id.0);

    let queued_b_entries = list_queued_entries_for_workspace(state.as_ref(), workspace_b.id)
        .await
        .unwrap();
    assert_eq!(queued_b_entries.len(), 1);
    assert_eq!(queued_b_entries[0].id.0, queued_b.id.0);
}

#[tokio::test]
async fn workspace_activation_only_schedules_the_opened_workspace() {
    let (data_dir, state) = setup_state().await;
    let workspace_a = create_workspace(&state, &data_dir, "a").await;
    let workspace_b = create_workspace(&state, &data_dir, "b").await;

    let _ = state.core.stores.workspace(workspace_a.id).await.unwrap();
    let _ = state.core.stores.workspace(workspace_b.id).await.unwrap();
    state.core.stores.evict_workspace(workspace_a.id).await;
    state.core.stores.evict_workspace(workspace_b.id).await;
    assert_eq!(state.core.stores.stats().await.workspace_store_count, 0);

    spawn_merge_queue_runner(state.clone());
    activate_workspace_merge_queue(&state, workspace_a.id).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let stats = state.core.stores.stats().await;
    assert_eq!(stats.workspace_store_count, 1);
    assert!(state.core.stores.workspace(workspace_a.id).await.is_ok());
    assert_eq!(state.core.stores.stats().await.workspace_store_count, 1);
}

#[tokio::test]
async fn disabled_workspace_with_queued_rows_are_cancelled_after_activation() {
    let (data_dir, state) = setup_state().await;
    let workspace = create_workspace(&state, &data_dir, "disabled").await;
    let store = state.core.stores.workspace(workspace.id).await.unwrap();
    let entry = queued_entry(workspace.id, "disabled-queued");
    store.create_merge_queue_entry(&entry).await.unwrap();
    drop(store);
    state.core.stores.evict_workspace(workspace.id).await;

    spawn_merge_queue_runner(state.clone());
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(state.core.stores.stats().await.workspace_store_count, 0);

    activate_workspace_merge_queue(&state, workspace.id).await;
    let stored = wait_for_entry_status(
        &state,
        workspace.id,
        entry.id,
        |status| status == MergeQueueEntryStatus::Cancelled,
        Duration::from_secs(2),
    )
    .await;
    assert_eq!(stored.status, MergeQueueEntryStatus::Cancelled);
    assert_eq!(
        stored.error_message.as_deref(),
        Some("merge queue disabled while entry was queued")
    );
    assert!(!state
        .transport
        .merge_queue_state
        .lock()
        .await
        .running
        .contains(&workspace.id));
}

#[tokio::test]
async fn cancel_for_disabled_workspace_noops_if_queue_was_reenabled() {
    let (data_dir, state) = setup_state().await;
    let workspace = create_workspace(&state, &data_dir, "reenabled").await;
    let store = state.core.stores.workspace(workspace.id).await.unwrap();
    let entry = queued_entry(workspace.id, "queued-before-reenable");
    store.create_merge_queue_entry(&entry).await.unwrap();

    update_merge_queue_config(
        &store,
        MergeQueueConfigUpdate {
            enabled: true,
            target_branch: Some("main".to_string()),
            verify_commands: Vec::new(),
            push_on_success: None,
            push_remote: None,
            push_branch: None,
            canonical_sync: None,
        },
    )
    .await
    .unwrap();

    cancel_queued_entries_for_disabled_workspace(&state, &store, workspace.id)
        .await
        .unwrap();

    let stored = store
        .get_merge_queue_entry(entry.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, MergeQueueEntryStatus::Queued);
}

#[tokio::test]
async fn enabled_workspace_queued_rows_resume_only_after_open() {
    let (data_dir, state) = setup_state().await;
    let workspace = create_workspace(&state, &data_dir, "enabled").await;
    let store = state.core.stores.workspace(workspace.id).await.unwrap();
    update_merge_queue_config(
        &store,
        MergeQueueConfigUpdate {
            enabled: true,
            target_branch: Some("main".to_string()),
            verify_commands: Vec::new(),
            push_on_success: None,
            push_remote: None,
            push_branch: None,
            canonical_sync: None,
        },
    )
    .await
    .unwrap();
    let entry = queued_entry(workspace.id, "enabled-queued");
    store.create_merge_queue_entry(&entry).await.unwrap();
    drop(store);
    state.core.stores.evict_workspace(workspace.id).await;

    spawn_merge_queue_runner(state.clone());
    tokio::time::sleep(Duration::from_millis(100)).await;

    let cold_store = state.core.stores.workspace(workspace.id).await.unwrap();
    let queued = cold_store
        .get_merge_queue_entry(entry.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(queued.status, MergeQueueEntryStatus::Queued);
    drop(cold_store);
    state.core.stores.evict_workspace(workspace.id).await;
    assert_eq!(state.core.stores.stats().await.workspace_store_count, 0);

    activate_workspace_merge_queue(&state, workspace.id).await;
    let resumed = wait_for_entry_status(
        &state,
        workspace.id,
        entry.id,
        |status| status != MergeQueueEntryStatus::Queued,
        Duration::from_secs(2),
    )
    .await;
    assert_ne!(resumed.status, MergeQueueEntryStatus::Queued);
}

#[tokio::test]
async fn enabled_workspace_queued_rows_resume_when_reopened_from_draining_store() {
    let (data_dir, state) = setup_state().await;
    let workspace = create_workspace(&state, &data_dir, "draining").await;
    let store = state.core.stores.workspace(workspace.id).await.unwrap();
    update_merge_queue_config(
        &store,
        MergeQueueConfigUpdate {
            enabled: true,
            target_branch: Some("main".to_string()),
            verify_commands: Vec::new(),
            push_on_success: None,
            push_remote: None,
            push_branch: None,
            canonical_sync: None,
        },
    )
    .await
    .unwrap();
    let entry = queued_entry(workspace.id, "draining-queued");
    store.create_merge_queue_entry(&entry).await.unwrap();

    spawn_merge_queue_runner(state.clone());
    state.core.stores.evict_workspace(workspace.id).await;
    assert_eq!(state.core.stores.stats().await.workspace_store_count, 0);

    let reopened = state.store_for_workspace(workspace.id).await.unwrap();
    activate_workspace_merge_queue(&state, workspace.id).await;
    let resumed = wait_for_entry_status(
        &state,
        workspace.id,
        entry.id,
        |status| status != MergeQueueEntryStatus::Queued,
        Duration::from_secs(2),
    )
    .await;
    assert_ne!(resumed.status, MergeQueueEntryStatus::Queued);

    drop(reopened);
    drop(store);
}

#[tokio::test]
async fn enabling_queue_reschedules_existing_queued_workspace() {
    let (data_dir, state) = setup_state().await;
    let workspace = create_workspace(&state, &data_dir, "reenable").await;
    let store = state.core.stores.workspace(workspace.id).await.unwrap();
    let entry = queued_entry(workspace.id, "reenable-queued");
    store.create_merge_queue_entry(&entry).await.unwrap();

    spawn_merge_queue_runner(state.clone());

    update_merge_queue_config(
        &store,
        MergeQueueConfigUpdate {
            enabled: true,
            target_branch: Some("main".to_string()),
            verify_commands: Vec::new(),
            push_on_success: None,
            push_remote: None,
            push_branch: None,
            canonical_sync: None,
        },
    )
    .await
    .unwrap();
    assert!(
        schedule_workspace_if_enabled_and_queued(&state, workspace.id)
            .await
            .unwrap()
    );

    let resumed = wait_for_entry_status(
        &state,
        workspace.id,
        entry.id,
        |status| status != MergeQueueEntryStatus::Queued,
        Duration::from_secs(2),
    )
    .await;
    assert_ne!(resumed.status, MergeQueueEntryStatus::Queued);
}

#[tokio::test]
async fn pending_wakeup_restarts_disabled_drain_after_reenable() {
    let (data_dir, state) = setup_state().await;
    let workspace = create_workspace(&state, &data_dir, "reenable-race").await;
    let store = state.core.stores.workspace(workspace.id).await.unwrap();
    let entry = queued_entry(workspace.id, "reenable-race-entry");
    store.create_merge_queue_entry(&entry).await.unwrap();
    spawn_merge_queue_runner(state.clone());

    assert!(
        begin_workspace_drain(state.as_ref(), workspace.id).await,
        "test should start with a simulated disabled drain already running"
    );

    update_merge_queue_config(
        &store,
        MergeQueueConfigUpdate {
            enabled: true,
            target_branch: Some("main".to_string()),
            verify_commands: Vec::new(),
            push_on_success: None,
            push_remote: None,
            push_branch: None,
            canonical_sync: None,
        },
    )
    .await
    .unwrap();

    schedule_workspace_drain(&state, workspace.id).await;
    assert!(
        state
            .transport
            .merge_queue_state
            .lock()
            .await
            .pending
            .contains(&workspace.id),
        "wakeups that land during an in-flight drain should be preserved"
    );

    if finish_workspace_drain(state.as_ref(), workspace.id).await {
        let _ = state.transport.merge_queue_schedule_tx.send(workspace.id);
    }

    let resumed = wait_for_entry_status(
        &state,
        workspace.id,
        entry.id,
        |status| status != MergeQueueEntryStatus::Queued,
        Duration::from_secs(2),
    )
    .await;
    assert_ne!(resumed.status, MergeQueueEntryStatus::Queued);
}

#[tokio::test]
async fn workspace_drain_ownership_allows_only_one_runner() {
    let (_data_dir, state) = setup_state().await;
    let workspace_id = WorkspaceId::new();

    assert!(begin_workspace_drain(state.as_ref(), workspace_id).await);
    assert!(!begin_workspace_drain(state.as_ref(), workspace_id).await);
    finish_workspace_drain(state.as_ref(), workspace_id).await;
    assert!(begin_workspace_drain(state.as_ref(), workspace_id).await);
}

#[tokio::test]
async fn disabled_drain_stays_dormant_without_reopening_workspace() {
    let (data_dir, state) = setup_state().await;
    let workspace = create_workspace(&state, &data_dir, "disabled-recheck").await;
    let store = state.core.stores.workspace(workspace.id).await.unwrap();
    update_merge_queue_config(
        &store,
        MergeQueueConfigUpdate {
            enabled: true,
            target_branch: Some("main".to_string()),
            verify_commands: Vec::new(),
            push_on_success: None,
            push_remote: None,
            push_branch: None,
            canonical_sync: None,
        },
    )
    .await
    .unwrap();
    let entry = queued_entry(workspace.id, "disabled-recheck-entry");
    store.create_merge_queue_entry(&entry).await.unwrap();
    drop(store);
    state.core.stores.evict_workspace(workspace.id).await;

    assert!(
        !reschedule_workspace_after_drain(&state, workspace.id, WorkspaceDrainStop::Disabled).await
    );
    assert_eq!(state.core.stores.stats().await.workspace_store_count, 0);
}
