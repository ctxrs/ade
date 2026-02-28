use std::path::PathBuf;

use ctx_http::installer;
use ctx_http::settings::{
    TitleGenerationLocalSettings, TitleGenerationMode, TitleGenerationSettings,
};
use ctx_http::title_generation;
use ctx_http::title_generation_local;
use ctx_store::StoreManager;

mod common;

const ENABLE_ENV: &str = "CTX_E2E_TITLE_GENERATION_LOCAL";
const DATA_DIR_ENV: &str = "CTX_E2E_TITLE_GENERATION_DATA_DIR";

fn env_enabled() -> bool {
    std::env::var(ENABLE_ENV)
        .ok()
        .map(|value| value.trim().to_ascii_lowercase())
        .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "yes"))
}

fn env_data_dir() -> Option<PathBuf> {
    std::env::var(DATA_DIR_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn generate_title_local_real_runtime() {
    if !env_enabled() {
        eprintln!("skipping: set {ENABLE_ENV}=1 to run this test");
        return;
    }

    if title_generation_local::runtime_download_spec().is_none() {
        eprintln!("skipping: no local runtime available for this platform");
        return;
    }

    let mut temp_dir = None;
    let data_root = if let Some(path) = env_data_dir() {
        if let Err(err) = std::fs::create_dir_all(&path) {
            panic!(
                "failed to create {DATA_DIR_ENV} dir {}: {err}",
                path.display()
            );
        }
        path
    } else {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().to_path_buf();
        temp_dir = Some(tmp);
        path
    };

    let stores = StoreManager::open(&data_root).await.unwrap();
    let state = common::build_state(
        data_root.clone(),
        stores,
        common::fake_providers(),
        "http://127.0.0.1:0",
    );

    let (install_id, _started) = state
        .start_install("title_generation_local".to_string(), None)
        .await;
    installer::install_title_generation_local_with_progress(state.clone(), install_id)
        .await
        .unwrap();

    let status = title_generation_local::local_status(&data_root)
        .await
        .unwrap();
    assert!(status.ready, "local runtime+model not ready: {status:?}");

    let cfg = TitleGenerationSettings {
        mode: TitleGenerationMode::Local,
        local: TitleGenerationLocalSettings {
            model_id: title_generation_local::LOCAL_MODEL_ID.to_string(),
            use_json: true,
        },
        ..Default::default()
    };

    let title = title_generation::generate_title(
        &cfg,
        "Summarize this session: install local title generation with llama.cpp and Qwen.",
        &data_root,
    )
    .await
    .unwrap();

    assert!(!title.is_empty(), "expected non-empty title");
    assert!(
        title.chars().count() <= title_generation::TITLE_MAX_CHARS,
        "title too long: {title}"
    );
    assert_ne!(
        title,
        title_generation::DEFAULT_SESSION_TITLE,
        "got default session title"
    );

    drop(temp_dir);
}
