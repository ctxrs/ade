use super::fixtures::{fixture_with_adapter, insert_options_cache};
use super::*;

#[tokio::test]
async fn restart_provider_for_auth_change_skips_adapters_without_drain_restart() {
    let fixture =
        fixture_with_adapter(Arc::new(UnsupportedRestartAdapter) as Arc<dyn ProviderAdapter>).await;
    let daemon = &fixture.daemon;

    insert_options_cache(
        daemon,
        "ws-a/host/codex",
        serde_json::json!({ "provider_id": "codex", "probe_ok": false }),
    )
    .await;

    daemon
        .handle()
        .providers()
        .restart_provider_for_auth_change("codex", "test auth updated")
        .await
        .expect("unsupported restart should be skipped");

    let options_cached = daemon
        .test_with_provider_options_cache(|cache| cache.contains_key("ws-a/host/codex"))
        .await;
    assert!(!options_cached);
}
