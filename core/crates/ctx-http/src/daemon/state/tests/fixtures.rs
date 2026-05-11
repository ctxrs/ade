use super::*;

pub(super) async fn test_state(temp: &tempfile::TempDir) -> Arc<AppState> {
    Arc::new(AppState::new(
        temp.path().to_path_buf(),
        StoreManager::open(temp.path()).await.expect("open stores"),
        HashMap::new(),
        "http://127.0.0.1:4310".to_string(),
        None,
    ))
}
