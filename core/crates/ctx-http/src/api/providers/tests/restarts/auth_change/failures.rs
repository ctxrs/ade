use super::fixtures::{fixture_with_adapter, insert_options_cache};
use super::*;

#[tokio::test]
async fn restart_provider_for_auth_change_returns_error_when_adapter_restart_fails() {
    let adapter = Arc::new(RestartFailingAdapter::default());
    let fixture = fixture_with_adapter(adapter.clone() as Arc<dyn ProviderAdapter>).await;
    let state = Arc::clone(&fixture.state);

    insert_options_cache(
        &state,
        "ws-a/host/codex",
        serde_json::json!({ "provider_id": "codex", "probe_ok": false }),
    )
    .await;

    let err = restart_provider_for_auth_change(&state, "codex", "test auth updated")
        .await
        .expect_err("restart failure should bubble up");
    assert!(err
        .to_string()
        .contains("provider auth updated but drain-restart failed for codex"));

    let options_cached = state
        .providers
        .with_provider_options_cache(|cache| cache.contains_key("ws-a/host/codex"))
        .await;
    assert!(!options_cached);

    assert_eq!(adapter.restart_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn set_codex_active_account_returns_error_when_restart_fails() {
    let fixture = fixture_with_adapter(Arc::new(RestartFailingAdapter::default())).await;
    let state = Arc::clone(&fixture.state);
    provider_accounts::upsert_codex_account(
        &state.core.data_root,
        provider_accounts::CodexAccountEntry {
            id: "acct".to_string(),
            label: "Account".to_string(),
            kind: provider_accounts::CODEX_CREDENTIAL_KIND_OAUTH.to_string(),
            email: Some("acct@example.com".to_string()),
            plan_type: None,
            created_at: Utc::now(),
            last_used_at: None,
            secret_ref: None,
            endpoint_profile: provider_accounts::CodexEndpointProfile::default(),
        },
    )
    .await
    .expect("seed codex account");

    let err = set_codex_active_account(
        State(Arc::clone(&state)),
        Json(CodexActiveAccountReq {
            account_id: Some("acct".to_string()),
        }),
    )
    .await
    .expect_err("restart failure should surface");
    assert_eq!(err.0, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(err
        .1
         .0
        .error
        .contains("provider auth updated but drain-restart failed"));
}
