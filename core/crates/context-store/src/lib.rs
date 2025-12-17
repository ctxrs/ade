pub mod store;

pub use store::Store;

#[cfg(test)]
mod tests {
    use super::Store;
    use context_core::ids::WorkspaceId;

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
}
