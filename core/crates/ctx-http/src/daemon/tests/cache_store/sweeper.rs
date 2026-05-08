use super::super::*;

#[tokio::test]
async fn sweeper_eviction_keeps_active_entries() {
    let temp = tempdir().unwrap();
    let stores = StoreManager::open(temp.path()).await.unwrap();
    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));
    let state = Arc::new(AppState::new(
        temp.path().to_path_buf(),
        stores.clone(),
        providers,
        "http://localhost".to_string(),
        None,
    ));

    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            temp.path().to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();
    let store = state.store_for_workspace(workspace.id).await.unwrap();
    let worktree = store
        .create_worktree(
            workspace.id,
            temp.path().to_string_lossy().to_string(),
            "deadbeef".to_string(),
            None,
        )
        .await
        .unwrap();
    let task = store
        .create_task(workspace.id, "task".to_string(), None)
        .await
        .unwrap();
    let session = store
        .create_session(
            task.id,
            workspace.id,
            worktree.id,
            ctx_core::models::ExecutionEnvironment::Host,
            "fake".to_string(),
            "model".to_string(),
            "implementer".to_string(),
            None,
            None,
            None,
        )
        .await
        .unwrap();

    {
        let mut cache = state.sessions.session_head_cache.lock().await;
        cache.insert(
            session.id,
            ctx_session_service::runtime::TimedEntry::new(HashMap::new()),
        );
    }
    let _ = state.sessions.get_broadcaster(session.id).await;
    let _ = state
        .sessions
        .subscribe_session_event_head(session.id)
        .await;
    let _ = state.ensure_scheduler(session.clone()).await;

    let now = Instant::now();
    {
        let mut cache = state.sessions.session_head_cache.lock().await;
        if let Some(entry) = cache.get_mut(&session.id) {
            entry.last_access = now - Duration::from_secs(3600);
        }
    }
    {
        let mut map = state.sessions.broadcasters.lock().await;
        if let Some(entry) = map.get_mut(&session.id) {
            entry.last_access = now;
        }
    }
    {
        let mut map = state.sessions.schedulers.lock().await;
        if let Some(entry) = map.get_mut(&session.id) {
            entry.last_access = now;
        }
    }

    let config = CacheSweepConfig {
        session_ttl: Duration::from_secs(60),
        workspace_ttl: Duration::from_secs(365 * 24 * 60 * 60),
        interval: Duration::from_secs(1),
    };
    let stats = state.sweep_idle_caches(now, config).await;
    assert_eq!(stats.session_head_evicted, 1);
    assert!(state
        .sessions
        .session_head_cache
        .lock()
        .await
        .get(&session.id)
        .is_none());
    assert!(state
        .sessions
        .broadcasters
        .lock()
        .await
        .get(&session.id)
        .is_some());
    assert!(state
        .sessions
        .schedulers
        .lock()
        .await
        .get(&session.id)
        .is_some());
}

#[tokio::test]
async fn sweeper_keeps_merge_queue_running_workspaces_resident() {
    let temp = tempdir().unwrap();
    let stores = StoreManager::open(temp.path()).await.unwrap();
    let state = Arc::new(AppState::new(
        temp.path().to_path_buf(),
        stores.clone(),
        HashMap::new(),
        "http://localhost".to_string(),
        None,
    ));

    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            temp.path().to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();
    let _ = state.store_for_workspace(workspace.id).await.unwrap();
    assert_eq!(stores.stats().await.workspace_store_count, 1);

    assert!(
        state
            .transport
            .merge_queue
            .begin_workspace_drain(workspace.id)
            .await
    );

    let config = CacheSweepConfig {
        session_ttl: Duration::from_secs(0),
        workspace_ttl: Duration::from_secs(0),
        interval: Duration::from_secs(30),
    };
    let _ = state.sweep_idle_caches(Instant::now(), config).await;
    assert_eq!(stores.stats().await.workspace_store_count, 1);

    let _ = state
        .transport
        .merge_queue
        .finish_workspace_drain(workspace.id)
        .await;

    let _ = state.sweep_idle_caches(Instant::now(), config).await;
    assert_eq!(stores.stats().await.workspace_store_count, 0);
}

#[tokio::test]
async fn opening_workspace_does_not_evict_active_workspace_store() {
    let temp = tempdir().unwrap();
    let stores = StoreManager::open_with_config(
        temp.path(),
        StoreManagerConfig {
            max_cached_workspaces: 2,
            ..StoreManagerConfig::default()
        },
    )
    .await
    .unwrap();
    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));
    let state = Arc::new(AppState::new(
        temp.path().to_path_buf(),
        stores.clone(),
        providers,
        "http://localhost".to_string(),
        None,
    ));

    let workspace_a = state
        .global_store()
        .create_workspace(
            "ws-a".to_string(),
            temp.path().join("ws-a").to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();
    let workspace_b = state
        .global_store()
        .create_workspace(
            "ws-b".to_string(),
            temp.path().join("ws-b").to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();
    let workspace_c = state
        .global_store()
        .create_workspace(
            "ws-c".to_string(),
            temp.path().join("ws-c").to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();

    let store_a = state.store_for_workspace(workspace_a.id).await.unwrap();
    let worktree_a = store_a
        .create_worktree(
            workspace_a.id,
            temp.path().join("wt-a").to_string_lossy().to_string(),
            "base-a".to_string(),
            None,
        )
        .await
        .unwrap();
    let task_a = store_a
        .create_task(workspace_a.id, "task-a".to_string(), None)
        .await
        .unwrap();
    let session_a = store_a
        .create_session(
            task_a.id,
            workspace_a.id,
            worktree_a.id,
            ctx_core::models::ExecutionEnvironment::Host,
            "fake".to_string(),
            "model".to_string(),
            "implementer".to_string(),
            None,
            None,
            None,
        )
        .await
        .unwrap();
    let _ = state.ensure_scheduler(session_a).await;

    let _ = state.store_for_workspace(workspace_b.id).await.unwrap();
    assert_eq!(stores.stats().await.workspace_store_count, 2);

    let _ = state.store_for_workspace(workspace_c.id).await.unwrap();
    assert_eq!(stores.stats().await.workspace_store_count, 2);

    let workspace_a_cached = state
        .core
        .stores
        .workspace_access(workspace_a.id)
        .await
        .unwrap();
    assert!(
        matches!(workspace_a_cached.kind, WorkspaceStoreAccessKind::Cached),
        "active workspace store should stay cached under the cap"
    );
    let workspace_b_reopened = state
        .core
        .stores
        .workspace_access(workspace_b.id)
        .await
        .unwrap();
    assert!(
        matches!(
            workspace_b_reopened.kind,
            WorkspaceStoreAccessKind::ColdOpen | WorkspaceStoreAccessKind::Reactivated
        ),
        "inactive workspace store should be the one evicted under the cap"
    );
}
