use super::*;
use ctx_daemon::daemon::sessions::title_generation::{
    generate_title_for_prompt, TitleGenerationSource,
};
use ctx_managed_installs::title_generation_local;

#[tokio::test]
async fn schedule_title_generation_falls_back_without_config() {
    let (_data_dir, daemon, session) = setup_state().await;
    let prompt = "make the title this: hello world";
    let spawned = daemon
        .handle()
        .sessions()
        .schedule_session_title_generation(session.clone(), prompt.to_string(), false)
        .await;

    assert!(!spawned);

    let store = daemon.store_for_session(session.id).await.unwrap();
    let updated = store.get_session(session.id).await.unwrap().unwrap();
    let expected = title_generation::fallback_title_from_prompt(prompt);
    assert_eq!(updated.title, expected);
}

#[tokio::test]
async fn generate_title_falls_back_when_local_runtime_missing() {
    let data_dir = tempfile::tempdir().unwrap();
    let model_path = title_generation_local::model_path(data_dir.path());
    if let Some(parent) = model_path.parent() {
        tokio::fs::create_dir_all(parent).await.unwrap();
    }
    tokio::fs::write(&model_path, b"stub").await.unwrap();

    let cfg = user_settings::TitleGenerationSettings {
        mode: user_settings::TitleGenerationMode::Local,
        local: user_settings::TitleGenerationLocalSettings {
            model_id: title_generation_local::LOCAL_MODEL_ID.to_string(),
            use_json: true,
        },
        ..Default::default()
    };

    let prompt = "make the title this: hello world";
    let outcome = generate_title_for_prompt(Some(&cfg), prompt, data_dir.path())
        .await
        .unwrap();

    assert!(matches!(outcome.source, TitleGenerationSource::Fallback));
    assert_eq!(
        outcome.title,
        title_generation::fallback_title_from_prompt(prompt)
    );
}
