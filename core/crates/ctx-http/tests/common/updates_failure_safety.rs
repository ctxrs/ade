use std::sync::Mutex;

use serde_json::{json, Value};

use crate::common;

static ENV_LOCK: Mutex<()> = Mutex::new(());

pub fn lock_env() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner())
}

pub struct EnvGuard {
    key: &'static str,
    previous: Option<String>,
}

impl EnvGuard {
    pub fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        if let Some(value) = self.previous.take() {
            std::env::set_var(self.key, value);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

pub fn current_platform_key() -> Option<&'static str> {
    ctx_http::updates::platform_key()
}

pub fn release_manifest_for(platform: &str, appimage_url_path: &str, sha256: &str) -> Value {
    json!({
      "channel": "stable",
      "latest_version": "9.9.9",
      "published_at": "2026-02-19T00:00:00Z",
      "platforms": {
        platform: {
          "appimage": {
            "url_path": appimage_url_path,
            "sha256": sha256
          },
          "daemon": {
            "url_path": "/download/stable/9.9.9/ctx-daemon",
            "sha256": "deadbeef"
          }
        }
      }
    })
}

pub async fn test_app_router(data_root: &std::path::Path) -> axum::Router {
    let stores = common::setup_store(data_root).await;
    let state = common::build_state(
        data_root.to_path_buf(),
        stores,
        common::fake_providers(),
        "http://127.0.0.1:0",
    );
    common::router(state)
}
