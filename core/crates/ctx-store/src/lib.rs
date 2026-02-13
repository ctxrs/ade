mod active_snapshot_observer;
pub mod manager;
pub mod store;

pub use active_snapshot_observer::{register_active_snapshot_observer, ActiveSnapshotObserver};
pub use manager::{StoreManager, StoreManagerConfig, StoreManagerStats};
pub use store::{is_unique_constraint_violation, Store, StoreStats, WorktreeBootstrapResultUpdate};

#[cfg(feature = "fault_injection")]
pub mod fault_injection;

#[cfg(not(feature = "fault_injection"))]
pub mod fault_injection {
    pub fn clear_failpoints() {}
    pub fn set_failpoint(_point: &'static str, _times: u32) {}
    pub fn maybe_fail(_point: &'static str) -> anyhow::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Store;
    use std::sync::Arc;
    use std::time::Duration;

    use ctx_core::ids::{MessageId, RunId, TurnId, WorkspaceId};
    use ctx_core::models::{Message, MessageDelivery, MessageRole, SessionEventType, VcsKind};
    use tokio::sync::Barrier;

    #[tokio::test]
    async fn can_create_and_list_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("db.sqlite");
        let store = Store::open(&db_path).await.unwrap();

        let ws = store
            .create_workspace("test".into(), "/tmp/test".into(), VcsKind::Git)
            .await
            .unwrap();
        assert_eq!(ws.name, "test");

        let list = store.list_workspaces().await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id.0, ws.id.0);

        let got = store.get_workspace(ws.id).await.unwrap();
        assert!(got.is_some());

        store.delete_workspace(ws.id).await.unwrap();
        let list = store.list_workspaces().await.unwrap();
        assert!(list.is_empty());
    }

    #[tokio::test]
    async fn can_create_task() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("db.sqlite");
        let store = Store::open(&db_path).await.unwrap();
        let ws = store
            .create_workspace("test".into(), "/tmp/test".into(), VcsKind::Git)
            .await
            .unwrap();

        let task = store
            .create_task(ws.id, "do thing".into(), None)
            .await
            .unwrap();
        let tasks = store.list_tasks(ws.id).await.unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id.0, task.id.0);
        assert!(tasks[0].assistant_seen_at.is_none());

        let fetched = store.get_task(task.id).await.unwrap().unwrap();
        assert_eq!(fetched.title, "do thing");
        assert!(fetched.assistant_seen_at.is_none());

        let updated_at_before = fetched.updated_at;
        store.mark_task_read(task.id).await.unwrap();
        let fetched_after_read = store.get_task(task.id).await.unwrap().unwrap();
        assert!(fetched_after_read.assistant_seen_at.is_some());
        assert_eq!(fetched_after_read.updated_at, updated_at_before);

        drop(store);
        let store = loop {
            match Store::open(&db_path).await {
                Ok(store) => break store,
                Err(err) if err.to_string().contains("database is locked") => {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Err(err) => panic!("failed to reopen store: {err:?}"),
            }
        };
        let fetched_after_restart = store.get_task(task.id).await.unwrap().unwrap();
        assert!(fetched_after_restart.assistant_seen_at.is_some());
        assert_eq!(fetched_after_restart.updated_at, updated_at_before);

        store.mark_task_unread(task.id).await.unwrap();
        let fetched_after_unread = store.get_task(task.id).await.unwrap().unwrap();
        assert!(fetched_after_unread.assistant_seen_at.is_none());
        assert_eq!(fetched_after_unread.updated_at, updated_at_before);

        // ensure list for other workspace empty
        let other = WorkspaceId::new();
        let tasks_other = store.list_tasks(other).await.unwrap();
        assert!(tasks_other.is_empty());
    }

    #[tokio::test]
    async fn concurrent_event_and_message_writes_do_not_error() {
        tokio::time::timeout(Duration::from_secs(60), async {
            let dir = tempfile::tempdir().unwrap();
            let db_path = dir.path().join("db.sqlite");
            let store = Store::open(&db_path).await.unwrap();

            let ws = store
                .create_workspace("test".into(), "/tmp/test".into(), VcsKind::Git)
                .await
                .unwrap();
            let task = store
                .create_task(ws.id, "do thing".into(), None)
                .await
                .unwrap();
            let worktree = store
                .create_worktree(ws.id, "/tmp/test".into(), "deadbeef".into(), None)
                .await
                .unwrap();
            let session = store
                .create_session(
                    task.id,
                    ws.id,
                    worktree.id,
                    "fake".into(),
                    "fake".into(),
                    "implementer".into(),
                    None,
                    None,
                    None,
                )
                .await
                .unwrap();

            let store = store.clone();
            let session_id = session.id;
            let task_id = session.task_id;

            const WORKERS: usize = 16;
            const WRITES_PER_WORKER: usize = 20;

            let barrier = Arc::new(Barrier::new(WORKERS));
            let mut handles = Vec::with_capacity(WORKERS);
            for worker in 0..WORKERS {
                let store = store.clone();
                let barrier = barrier.clone();
                handles.push(tokio::spawn(async move {
                    barrier.wait().await;
                    for i in 0..WRITES_PER_WORKER {
                        let run_id = RunId::new();
                        let turn_id = TurnId::new();
                        store
                            .append_session_event(
                                session_id,
                                Some(run_id),
                                Some(turn_id),
                                SessionEventType::Notice,
                                serde_json::json!({ "worker": worker, "i": i }),
                            )
                            .await?;
                        store
                            .insert_message(Message {
                                id: MessageId::new(),
                                session_id,
                                task_id,
                                run_id: Some(run_id),
                                turn_id: Some(turn_id),
                                turn_sequence: None,
                                order_seq: None,
                                role: MessageRole::User,
                                content: format!("hello {worker} {i}"),
                                attachments: vec![],
                                delivery: MessageDelivery::Immediate,
                                delivered_at: None,
                                created_at: chrono::Utc::now(),
                            })
                            .await?;
                    }
                    anyhow::Result::<()>::Ok(())
                }));
            }

            for h in handles {
                h.await.unwrap().unwrap();
            }

            let events = store.list_session_events(session_id).await.unwrap();
            assert_eq!(events.len(), WORKERS * WRITES_PER_WORKER);
            let messages = store.list_messages_for_session(session_id).await.unwrap();
            assert_eq!(messages.len(), WORKERS * WRITES_PER_WORKER);
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn subagent_sessions_and_last_message_for_run() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("db.sqlite");
        let store = Store::open(&db_path).await.unwrap();

        let ws = store
            .create_workspace("test".into(), "/tmp/test".into(), VcsKind::Git)
            .await
            .unwrap();
        let task = store.create_task(ws.id, "task".into(), None).await.unwrap();
        let worktree = store
            .create_worktree(ws.id, "/tmp/test".into(), "deadbeef".into(), None)
            .await
            .unwrap();

        let parent = store
            .create_session(
                task.id,
                ws.id,
                worktree.id,
                "fake".into(),
                "fake".into(),
                "implementer".into(),
                None,
                None,
                None,
            )
            .await
            .unwrap();
        let subagent = store
            .create_session(
                task.id,
                ws.id,
                worktree.id,
                "fake".into(),
                "fake".into(),
                "subagent".into(),
                Some(parent.id),
                Some("sub_agent".into()),
                None,
            )
            .await
            .unwrap();
        let _reviewer = store
            .create_session(
                task.id,
                ws.id,
                worktree.id,
                "fake".into(),
                "fake".into(),
                "reviewer".into(),
                Some(parent.id),
                Some("reviewer".into()),
                None,
            )
            .await
            .unwrap();

        let subs = store.list_subagent_sessions(parent.id).await.unwrap();
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].id.0, subagent.id.0);

        let run_id = RunId::new();
        let turn_id = TurnId::new();
        store
            .insert_message(Message {
                id: MessageId::new(),
                session_id: subagent.id,
                task_id: subagent.task_id,
                run_id: Some(run_id),
                turn_id: Some(turn_id),
                turn_sequence: Some(1),
                order_seq: None,
                role: MessageRole::Assistant,
                content: "final response".to_string(),
                attachments: vec![],
                delivery: MessageDelivery::Immediate,
                delivered_at: None,
                created_at: chrono::Utc::now(),
            })
            .await
            .unwrap();

        let last = store
            .get_last_assistant_message_for_run(subagent.id, run_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(last.content, "final response");
    }

    #[tokio::test]
    async fn workspace_active_page_includes_primary_and_subagent_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("db.sqlite");
        let store = Store::open(&db_path).await.unwrap();
        let ws = store
            .create_workspace("ws".into(), "/tmp/ws".into(), VcsKind::Git)
            .await
            .unwrap();

        let task = store
            .create_task(ws.id, "active".into(), None)
            .await
            .unwrap();
        let worktree = store
            .create_worktree(ws.id, "/tmp/ws".into(), "abc123".into(), None)
            .await
            .unwrap();
        let primary = store
            .create_session(
                task.id,
                ws.id,
                worktree.id,
                "fake".into(),
                "fake".into(),
                "implementer".into(),
                None,
                None,
                None,
            )
            .await
            .unwrap();
        store
            .set_task_primary_session(task.id, primary.id, worktree.id)
            .await
            .unwrap();
        let subagent = store
            .create_session(
                task.id,
                ws.id,
                worktree.id,
                "fake".into(),
                "fake".into(),
                "reviewer".into(),
                Some(primary.id),
                Some("sub_agent".into()),
                None,
            )
            .await
            .unwrap();

        let (summaries, total) = store.list_workspace_active_page(ws.id, 50).await.unwrap();
        assert_eq!(total, 1);
        assert_eq!(summaries.len(), 1);
        let summary = &summaries[0];
        assert_eq!(summary.primary_session.session.id, primary.id);
        assert!(summary.primary_session_head.is_none());
        assert_eq!(summary.sessions.len(), 1);
        assert_eq!(summary.sessions[0].session.id, subagent.id);
    }

    #[cfg(feature = "fault_injection")]
    async fn setup_fault_fixture() -> (
        tempfile::TempDir,
        Store,
        ctx_core::ids::WorkspaceId,
        ctx_core::ids::TaskId,
        ctx_core::ids::WorktreeId,
        ctx_core::ids::SessionId,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("db.sqlite");
        let store = Store::open(&db_path).await.unwrap();
        let ws = store
            .create_workspace("fault".into(), "/tmp/fault".into(), VcsKind::Git)
            .await
            .unwrap();
        let task = store
            .create_task(ws.id, "fault task".into(), None)
            .await
            .unwrap();
        let worktree = store
            .create_worktree(ws.id, "/tmp/fault".into(), "deadbeef".into(), None)
            .await
            .unwrap();
        let session = store
            .create_session(
                task.id,
                ws.id,
                worktree.id,
                "fake".into(),
                "fake".into(),
                "implementer".into(),
                None,
                None,
                None,
            )
            .await
            .unwrap();
        (dir, store, ws.id, task.id, worktree.id, session.id)
    }

    #[cfg(feature = "fault_injection")]
    #[tokio::test]
    async fn fault_injection_append_session_event_fails_once_then_recovers() {
        let (_dir, store, _ws_id, _task_id, _worktree_id, session_id) = setup_fault_fixture().await;
        crate::fault_injection::clear_failpoints();
        crate::fault_injection::set_failpoint("ctx_store.append_session_event", 1);

        let first = store
            .append_session_event(
                session_id,
                None,
                None,
                SessionEventType::Notice,
                serde_json::json!({"msg":"first"}),
            )
            .await;
        assert!(first.is_err(), "expected injected failure for first append");

        let second = store
            .append_session_event(
                session_id,
                None,
                None,
                SessionEventType::Notice,
                serde_json::json!({"msg":"second"}),
            )
            .await;
        assert!(second.is_ok(), "expected recovery after one-shot failpoint");
        crate::fault_injection::clear_failpoints();
    }

    #[cfg(feature = "fault_injection")]
    #[tokio::test]
    async fn fault_injection_list_session_events_page_fails_once_then_recovers() {
        let (_dir, store, _ws_id, _task_id, _worktree_id, session_id) = setup_fault_fixture().await;
        crate::fault_injection::clear_failpoints();
        store
            .append_session_event(
                session_id,
                None,
                None,
                SessionEventType::Notice,
                serde_json::json!({"msg":"seed"}),
            )
            .await
            .unwrap();

        crate::fault_injection::set_failpoint("ctx_store.list_session_events_page_by_seq", 1);
        let first = store
            .list_session_events_page_by_seq(session_id, None, None, false)
            .await;
        assert!(first.is_err(), "expected injected failure for first list");

        let second = store
            .list_session_events_page_by_seq(session_id, None, None, false)
            .await
            .unwrap();
        assert_eq!(second.len(), 1);
        crate::fault_injection::clear_failpoints();
    }

    #[cfg(feature = "fault_injection")]
    #[tokio::test]
    async fn fault_injection_session_head_snapshot_fails_once_then_recovers() {
        let (_dir, store, _ws_id, _task_id, _worktree_id, session_id) = setup_fault_fixture().await;
        crate::fault_injection::clear_failpoints();
        store
            .append_session_event(
                session_id,
                None,
                None,
                SessionEventType::Notice,
                serde_json::json!({"msg":"seed"}),
            )
            .await
            .unwrap();

        crate::fault_injection::set_failpoint("ctx_store.get_session_head_snapshot", 1);
        let first = store.get_session_head_snapshot(session_id, 10, true).await;
        assert!(
            first.is_err(),
            "expected injected failure for session head snapshot"
        );

        let second = store
            .get_session_head_snapshot(session_id, 10, true)
            .await
            .unwrap();
        assert!(
            second.is_some(),
            "expected session head snapshot after recovery"
        );
        crate::fault_injection::clear_failpoints();
    }
}
