use super::*;

#[tokio::test]
async fn startup_provider_status_refresh_runs_in_background() {
    let temp = tempdir().unwrap();
    let stores = StoreManager::open(temp.path()).await.unwrap();
    let adapter = Arc::new(BlockingInspectAdapter::default());
    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert("blocking".into(), adapter.clone());
    let state = Arc::new(AppState::new(
        temp.path().to_path_buf(),
        stores,
        providers,
        "http://localhost".to_string(),
        None,
    ));

    spawn_startup_provider_status_refresh(state.clone());

    tokio::time::timeout(Duration::from_secs(1), async {
        while !adapter.inspect_started.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("startup refresh should begin inspect work");

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        state
            .providers
            .with_provider_statuses(|statuses| statuses.is_empty())
            .await,
        "provider status refresh should no longer block startup"
    );

    adapter.release_inspect.notify_waiters();

    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if state
                .providers
                .with_provider_statuses(|statuses| statuses.contains_key("blocking"))
                .await
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("background provider status refresh should complete after inspect unblocks");
}
