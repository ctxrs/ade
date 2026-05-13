use super::fixtures::{fixture_with_adapter, insert_options_cache, insert_verify_cache};
use super::*;

#[tokio::test]
async fn restart_provider_for_auth_change_invalidates_only_matching_provider_probe_caches() {
    let adapter = Arc::new(RestartTrackingAdapter::default());
    let fixture = fixture_with_adapter(adapter.clone() as Arc<dyn ProviderAdapter>).await;
    let state = Arc::clone(&fixture.state);

    insert_options_cache(
        &state,
        "ws-a/host/codex",
        serde_json::json!({ "provider_id": "codex", "probe_ok": false }),
    )
    .await;
    insert_options_cache(
        &state,
        "ws-b/container/claude-crp",
        serde_json::json!({ "provider_id": "claude-crp", "probe_ok": true }),
    )
    .await;
    insert_verify_cache(
        &state,
        "ws-a/host/codex",
        serde_json::json!({ "status": "error" }),
    )
    .await;
    insert_verify_cache(
        &state,
        "ws-b/container/claude-crp",
        serde_json::json!({ "status": "ok" }),
    )
    .await;

    restart_provider_for_auth_change(&state, "codex", "test auth updated")
        .await
        .expect("restart should succeed");

    let (codex_options_cached, claude_options_cached) = state
        .providers
        .with_provider_options_cache(|cache| {
            (
                cache.contains_key("ws-a/host/codex"),
                cache.contains_key("ws-b/container/claude-crp"),
            )
        })
        .await;
    assert!(!codex_options_cached);
    assert!(claude_options_cached);

    let (codex_verify_cached, claude_verify_cached) = state
        .providers
        .with_provider_verify_cache(|cache| {
            (
                cache.contains_key("ws-a/host/codex"),
                cache.contains_key("ws-b/container/claude-crp"),
            )
        })
        .await;
    assert!(!codex_verify_cached);
    assert!(claude_verify_cached);

    assert_eq!(adapter.restart_calls.load(Ordering::SeqCst), 1);
}
