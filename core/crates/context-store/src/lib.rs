pub mod store;

pub use store::Store;

#[cfg(test)]
mod tests {
    use super::Store;
    use chrono::Utc;
    use std::sync::Arc;
    use std::time::Duration;

    use context_core::ids::{MessageId, RunId, TurnId, WorkspaceId, WorktreeId};
    use context_core::models::{Message, MessageDelivery, MessageRole, SessionEventType, Worktree};
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
                .create_session(
                    &track,
                    "fake".into(),
                    "fake".into(),
                    "implementer".into(),
                    None,
                )
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

    #[tokio::test]
    async fn workspace_index_page_counts_and_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("db.sqlite");
        let store = Store::open(&db_path).await.unwrap();
        let ws = store
            .create_workspace("ws".into(), "/tmp/ws".into())
            .await
            .unwrap();

        let task_active = store
            .create_task(ws.id, "active".into(), None)
            .await
            .unwrap();
        let worktree = Worktree {
            id: WorktreeId::new(),
            workspace_id: ws.id,
            root_path: "/tmp/ws".into(),
            base_commit_sha: "abc123".into(),
            git_branch: None,
            created_at: Utc::now(),
        };
        store.insert_worktree(worktree.clone()).await.unwrap();
        let track = store
            .create_track(task_active.id, ws.id, worktree.id, "default".into())
            .await
            .unwrap();
        store
            .create_session(
                &track,
                "fake".into(),
                "fake-model".into(),
                "implementer".into(),
                None,
            )
            .await
            .unwrap();

        let task_archived = store
            .create_task(ws.id, "archived".into(), None)
            .await
            .unwrap();
        store.archive_task(task_archived.id).await.unwrap();

        let (page, cursor) = store
            .list_workspace_index_page(ws.id, None, 50, false)
            .await
            .unwrap();
        assert_eq!(page.len(), 1);
        assert!(cursor.is_none());
        let summary = &page[0];
        assert_eq!(summary.task.id, task_active.id);
        assert_eq!(summary.tracks.len(), 1);
        assert_eq!(summary.tracks[0].sessions.len(), 1);
        assert_eq!(summary.provider_ids, vec!["fake".to_string()]);

        let (active_count, archived_count) = store.workspace_task_counts(ws.id).await.unwrap();
        assert_eq!(active_count, 1);
        assert_eq!(archived_count, 1);

        let (page_all, _) = store
            .list_workspace_index_page(ws.id, None, 50, true)
            .await
            .unwrap();
        assert_eq!(page_all.len(), 2);
        assert!(page_all
            .iter()
            .any(|s| s.task.id == task_archived.id && s.task.archived_at.is_some()));

        let summary = store
            .get_workspace_task_summary(task_active.id)
            .await
            .unwrap()
            .expect("summary exists");
        assert_eq!(summary.task.id, task_active.id);
        assert_eq!(summary.tracks.len(), 1);
        assert_eq!(summary.tracks[0].sessions.len(), 1);
    }

    #[tokio::test]
    async fn workspace_index_cursor_supports_pagination() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("db.sqlite");
        let store = Store::open(&db_path).await.unwrap();
        let ws = store
            .create_workspace("ws".into(), "/tmp/ws".into())
            .await
            .unwrap();

        for i in 0..3 {
            let title = format!("task-{i}");
            store.create_task(ws.id, title.into(), None).await.unwrap();
        }

        let (page1, cursor1) = store
            .list_workspace_index_page(ws.id, None, 2, false)
            .await
            .unwrap();
        assert_eq!(page1.len(), 2);
        assert!(cursor1.is_some());

        if let Some(cursor) = cursor1 {
            let (page2, cursor2) = store
                .list_workspace_index_page(ws.id, Some(cursor), 2, false)
                .await
                .unwrap();
            assert!(page2.len() <= 2);
            assert!(cursor2.is_none());
        } else {
            panic!("expected cursor for second page");
        }
    }
}
