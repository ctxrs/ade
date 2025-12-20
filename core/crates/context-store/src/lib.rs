pub mod store;

pub use store::Store;

#[cfg(test)]
mod tests {
    use super::Store;
    use std::sync::Arc;
    use std::time::Duration;

    use context_core::ids::{MessageId, RunId, TurnId, WorkspaceId};
    use context_core::models::{Message, MessageDelivery, MessageRole, SessionEventType};
    use tokio::sync::Barrier;

    #[tokio::test]
    async fn can_create_and_list_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("db.sqlite");
        let store = Store::open(&db_path).await.unwrap();

        let ws = store
            .create_workspace("test".into(), "/tmp/test".into())
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
            .create_workspace("test".into(), "/tmp/test".into())
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
        let store = Store::open(&db_path).await.unwrap();
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
        tokio::time::timeout(Duration::from_secs(10), async {
            let dir = tempfile::tempdir().unwrap();
            let db_path = dir.path().join("db.sqlite");
            let store = Store::open(&db_path).await.unwrap();

            let ws = store
                .create_workspace("test".into(), "/tmp/test".into())
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
            let track = store
                .create_track(task.id, ws.id, worktree.id, "t1".into())
                .await
                .unwrap();
            let session = store
                .create_session(&track, "fake".into(), "fake".into(), "implementer".into(), None)
                .await
                .unwrap();

            let store = store.clone();
            let session_id = session.id;
            let task_id = session.task_id;
            let track_id = session.track_id;

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
                                track_id,
                                run_id: Some(run_id),
                                turn_id: Some(turn_id),
                                turn_sequence: None,
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
}
