use super::*;
use tokio::sync::oneshot;
use tokio::time::{timeout, Duration};

#[tokio::test]
async fn update_execution_config_can_leave_runtime_unspecified() {
    let temp = tempfile::tempdir().expect("tempdir");
    let db_path = temp.path().join("db.sqlite");
    let store = Store::open_sqlite(&db_path, None)
        .await
        .expect("open sqlite store");

    update_execution_config(
        &store,
        ExecutionConfigUpdate {
            environment: ExecutionEnvironment::Sandbox,
            network_mode: Some(ContainerNetworkMode::LlmOnly),
            allowlist: Some(vec![" api.openai.com ".to_string(), "".to_string()]),
            image: None,
        },
    )
    .await
    .expect("update execution config");

    let loaded = load_execution_settings_override(&store)
        .await
        .expect("load override")
        .expect("execution override");
    assert_eq!(loaded.mode, Some(ExecutionMode::Sandbox));
    assert_eq!(
        loaded.container.network_mode,
        Some(ContainerNetworkMode::LlmOnly)
    );
    assert_eq!(
        loaded.container.allowlist,
        Some(vec!["api.openai.com".to_string()])
    );

    store.close().await;
}

#[tokio::test]
async fn preferred_new_session_model_round_trips_and_clears() {
    let temp = tempfile::tempdir().expect("tempdir");
    let db_path = temp.path().join("db.sqlite");
    let store = Store::open_sqlite(&db_path, None)
        .await
        .expect("open sqlite store");

    update_preferred_new_session_model_id(&store, " codex ", Some(" gpt-5.4/xhigh ".to_string()))
        .await
        .expect("persist preferred model");
    update_preferred_new_session_model_id(&store, "claude-crp", Some(" opus/high ".to_string()))
        .await
        .expect("persist second preferred model");

    assert_eq!(
        load_preferred_new_session_model_id(&store, "codex")
            .await
            .expect("load codex pref"),
        Some("gpt-5.4/xhigh".to_string())
    );
    assert_eq!(
        load_preferred_new_session_models(&store)
            .await
            .expect("load pref map"),
        HashMap::from([
            ("claude-crp".to_string(), "opus/high".to_string()),
            ("codex".to_string(), "gpt-5.4/xhigh".to_string()),
        ])
    );

    update_preferred_new_session_model_id(&store, "codex", Some("   ".to_string()))
        .await
        .expect("clear codex pref");
    assert_eq!(
        load_preferred_new_session_model_id(&store, "codex")
            .await
            .expect("load cleared codex pref"),
        None
    );
    assert_eq!(
        load_preferred_new_session_models(&store)
            .await
            .expect("load remaining pref map"),
        HashMap::from([("claude-crp".to_string(), "opus/high".to_string())])
    );

    update_preferred_new_session_model_id(&store, "claude-crp", None)
        .await
        .expect("clear last pref");
    assert_eq!(
        load_preferred_new_session_models(&store)
            .await
            .expect("load empty pref map"),
        HashMap::new()
    );

    store.close().await;
}

#[tokio::test]
async fn preferred_new_session_model_requires_provider_id() {
    let temp = tempfile::tempdir().expect("tempdir");
    let db_path = temp.path().join("db.sqlite");
    let store = Store::open_sqlite(&db_path, None)
        .await
        .expect("open sqlite store");

    let error =
        update_preferred_new_session_model_id(&store, "   ", Some("gpt-5.4/xhigh".to_string()))
            .await
            .expect_err("blank provider id should fail");
    assert!(error.to_string().contains("provider_id is required"));

    store.close().await;
}

#[tokio::test]
async fn malformed_preferred_new_session_model_entries_are_ignored() {
    let temp = tempfile::tempdir().expect("tempdir");
    let db_path = temp.path().join("db.sqlite");
    let store = Store::open_sqlite(&db_path, None)
        .await
        .expect("open sqlite store");

    store
        .upsert_runtime_settings_document(
            WORKSPACE_SETTINGS_SCHEMA_VERSION,
            r#"{
  "new_session": {
    "preferred_model_by_provider": {
      "codex": 7,
      "claude-crp": " opus/high ",
      "empty": "   "
    }
  }
}"#,
        )
        .await
        .expect("write malformed runtime settings");

    assert_eq!(
        load_preferred_new_session_models(&store)
            .await
            .expect("load preferred model map"),
        HashMap::from([("claude-crp".to_string(), "opus/high".to_string())])
    );
    assert_eq!(
        load_preferred_new_session_model_id(&store, "codex")
            .await
            .expect("load malformed codex preference"),
        None
    );

    store.close().await;
}

#[tokio::test]
async fn concurrent_workspace_settings_updates_do_not_clobber_each_other() {
    let temp = tempfile::tempdir().expect("tempdir");
    let db_path = temp.path().join("db.sqlite");
    let store = Store::open_sqlite(&db_path, None)
        .await
        .expect("open sqlite store");

    let (loaded_tx, loaded_rx) = oneshot::channel();
    let (resume_tx, resume_rx) = oneshot::channel();
    workspace_settings_test_pause_hook()
        .lock()
        .await
        .insert("primary_branch", (loaded_tx, resume_rx));

    let first_store = store.clone();
    let first = tokio::spawn(async move { update_primary_branch(&first_store, "main").await });
    loaded_rx.await.expect("first update reached pause point");

    let second_store = store.clone();
    let second = tokio::spawn(async move {
        update_preferred_new_session_model_id(
            &second_store,
            "codex",
            Some("gpt-5.4/xhigh".to_string()),
        )
        .await
    });

    let mut second = second;
    let second_blocked = timeout(Duration::from_millis(100), &mut second)
        .await
        .is_err();
    resume_tx.send(()).expect("resume paused update");
    assert!(
        second_blocked,
        "second workspace settings write should wait for the first"
    );

    first
        .await
        .expect("join first")
        .expect("first update succeeds");
    second
        .await
        .expect("join second")
        .expect("second update succeeds");

    assert_eq!(
        load_primary_branch(&store)
            .await
            .expect("load primary branch"),
        Some("main".to_string())
    );
    assert_eq!(
        load_preferred_new_session_model_id(&store, "codex")
            .await
            .expect("load preferred model"),
        Some("gpt-5.4/xhigh".to_string())
    );

    store.close().await;
}
