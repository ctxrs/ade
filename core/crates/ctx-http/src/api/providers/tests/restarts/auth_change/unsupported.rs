use super::fixtures::{fixture_with_adapter, insert_options_cache};
use super::*;

#[tokio::test]
async fn restart_provider_for_auth_change_skips_adapters_without_drain_restart() {
    let fixture =
        fixture_with_adapter(Arc::new(UnsupportedRestartAdapter) as Arc<dyn ProviderAdapter>).await;
    let state = Arc::clone(&fixture.state);

    insert_options_cache(
        &state,
        "ws-a/host/codex",
        serde_json::json!({ "provider_id": "codex", "probe_ok": false }),
    )
    .await;

    restart_provider_for_auth_change(&state, "codex", "test auth updated")
        .await
        .expect("unsupported restart should be skipped");

    let options_cached = state
        .test_with_provider_options_cache(|cache| cache.contains_key("ws-a/host/codex"))
        .await;
    assert!(!options_cached);
}
