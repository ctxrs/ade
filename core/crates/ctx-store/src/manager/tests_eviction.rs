use super::*;

use chrono::Utc;

use ctx_core::ids::TurnId;
use ctx_core::models::{
    ExecutionEnvironment, SessionEventType, SessionTurn, SessionTurnStatus, VcsKind,
};

#[tokio::test]
async fn evicted_workspace_clone_remains_usable_until_last_handle_drops() {
    let temp = tempfile::tempdir().unwrap();
    let manager = StoreManager::open_with_config(
        temp.path(),
        StoreManagerConfig {
            max_cached_workspaces: 1,
            ..StoreManagerConfig::default()
        },
    )
    .await
    .unwrap();
    let workspace_a = manager
        .global()
        .create_workspace(
            "a".to_string(),
            temp.path().join("a").to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();
    let workspace_b = manager
        .global()
        .create_workspace(
            "b".to_string(),
            temp.path().join("b").to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();

    let store_a = manager.workspace(workspace_a.id).await.unwrap();
    let worktree_a = store_a
        .create_worktree(
            workspace_a.id,
            temp.path().join("a").to_string_lossy().to_string(),
            "base".to_string(),
            None,
        )
        .await
        .unwrap();
    let task_a = store_a
        .create_task(workspace_a.id, "task".to_string(), None)
        .await
        .unwrap();
    let session_a = store_a
        .create_session(
            task_a.id,
            workspace_a.id,
            worktree_a.id,
            ExecutionEnvironment::Host,
            "fake".to_string(),
            "fake".to_string(),
            "implementer".to_string(),
            None,
            None,
            None,
        )
        .await
        .unwrap();
    store_a
        .set_task_primary_session(task_a.id, session_a.id, worktree_a.id)
        .await
        .unwrap();
    let turn_id = TurnId::new();
    let now = Utc::now();
    store_a
        .insert_session_turn(SessionTurn {
            turn_id,
            session_id: session_a.id,
            run_id: None,
            user_message_id: None,
            status: SessionTurnStatus::Running,
            start_seq: Some(1),
            end_seq: None,
            started_at: now,
            updated_at: now,
            assistant_partial: None,
            thought_partial: None,
            metrics_json: None,
            tool_total: 0,
            tool_pending: 0,
            tool_running: 0,
            tool_completed: 0,
            tool_failed: 0,
        })
        .await
        .unwrap();
    let _ = store_a
        .append_session_event(
            session_a.id,
            None,
            Some(turn_id),
            SessionEventType::Notice,
            serde_json::json!({"msg":"before evict"}),
        )
        .await
        .unwrap();
    let _ = manager.workspace(workspace_b.id).await.unwrap();
    let evicted = manager
        .evict_workspaces_to_cap(&HashSet::from([workspace_b.id]))
        .await;
    assert_eq!(evicted, 1);
    assert_eq!(manager.stats().await.workspace_store_count, 1);

    let reopened_before_drop = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        manager.workspace_access(workspace_a.id),
    )
    .await
    .expect("reopen should reuse the draining store, not deadlock")
    .unwrap();
    assert!(reopened_before_drop.opened_now);
    assert!(reopened_before_drop
        .store
        .get_task(task_a.id)
        .await
        .unwrap()
        .is_some());
    drop(reopened_before_drop);

    let task = store_a
        .create_task(workspace_a.id, "still-live".to_string(), None)
        .await
        .unwrap();
    assert_eq!(task.workspace_id, workspace_a.id);

    drop(store_a);

    assert!(
        manager
            .workspace_access(workspace_a.id)
            .await
            .unwrap()
            .opened_now
    );
}

#[tokio::test]
async fn deleted_workspace_is_not_rehydrated_from_pending_close_store() {
    let temp = tempfile::tempdir().unwrap();
    let manager = StoreManager::open_with_config(
        temp.path(),
        StoreManagerConfig {
            max_cached_workspaces: 1,
            ..StoreManagerConfig::default()
        },
    )
    .await
    .unwrap();
    let workspace = manager
        .global()
        .create_workspace(
            "a".to_string(),
            temp.path().join("a").to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();
    let store = manager.workspace(workspace.id).await.unwrap();

    manager.evict_workspace(workspace.id).await;
    manager
        .global()
        .delete_workspace(workspace.id)
        .await
        .unwrap();

    let manager_for_reopen = manager.clone();
    let reopen =
        tokio::spawn(async move { manager_for_reopen.workspace_access(workspace.id).await });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(
        !reopen.is_finished(),
        "reopen should wait for the draining store to finish closing"
    );

    drop(store);

    let err = match reopen.await.unwrap() {
        Ok(_) => panic!("deleted workspace should not reopen from a draining store"),
        Err(err) => err.to_string(),
    };
    assert!(
        err.contains("not found"),
        "expected missing workspace error after delete, got: {err}"
    );
}
