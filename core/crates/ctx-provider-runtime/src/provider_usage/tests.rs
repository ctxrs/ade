use super::*;
use std::path::{Path, PathBuf};

struct EnvVarGuard {
    key: &'static str,
    prev: Option<String>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let prev = std::env::var(key).ok();
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, prev }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        unsafe {
            if let Some(prev) = self.prev.as_deref() {
                std::env::set_var(self.key, prev);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }
}

struct TestUsageHost {
    data_root: PathBuf,
    provider_runtime: ProviderRuntime,
    shutdown_tx: broadcast::Sender<()>,
}

impl TestUsageHost {
    fn new(data_root: PathBuf) -> Self {
        let (shutdown_tx, _) = broadcast::channel(1);
        Self {
            data_root,
            provider_runtime: ProviderRuntime::new(HashMap::new()),
            shutdown_tx,
        }
    }
}

impl ProviderUsageHost for TestUsageHost {
    fn data_root(&self) -> &Path {
        &self.data_root
    }

    fn provider_runtime(&self) -> &ProviderRuntime {
        &self.provider_runtime
    }

    fn subscribe_shutdown(&self) -> broadcast::Receiver<()> {
        self.shutdown_tx.subscribe()
    }
}

#[tokio::test]
async fn refresh_provider_usage_surfaces_agent_server_config_errors() {
    let data_root = tempfile::tempdir().expect("tempdir");
    let runtime_home = tempfile::tempdir().expect("runtime home");
    let _codex_home = EnvVarGuard::set("CTX_CODEX_HOME", &runtime_home.path().to_string_lossy());

    let config_path = data_root
        .path()
        .join("providers")
        .join("agent-servers")
        .join("agent_servers.json");
    std::fs::create_dir_all(config_path.parent().expect("config parent")).expect("mkdir");
    std::fs::write(&config_path, "{ not valid json").expect("write invalid config");

    let host = TestUsageHost::new(data_root.path().to_path_buf());
    let err = refresh_provider_usage(&host)
        .await
        .expect_err("invalid managed config should fail usage refresh");
    assert!(err.to_string().contains("loading agent server config"));
}

#[tokio::test]
async fn refresh_provider_usage_replaces_stale_cache_with_error_snapshot_on_config_error() {
    let data_root = tempfile::tempdir().expect("tempdir");
    let runtime_home = tempfile::tempdir().expect("runtime home");
    let _codex_home = EnvVarGuard::set("CTX_CODEX_HOME", &runtime_home.path().to_string_lossy());

    let config_path = data_root
        .path()
        .join("providers")
        .join("agent-servers")
        .join("agent_servers.json");
    std::fs::create_dir_all(config_path.parent().expect("config parent")).expect("mkdir");
    std::fs::write(&config_path, "{ not valid json").expect("write invalid config");

    let host = TestUsageHost::new(data_root.path().to_path_buf());
    host.provider_runtime
        .with_provider_usage_cache(|cache| {
            cache.insert(
                "codex".to_string(),
                ProviderUsageSnapshot {
                    provider_id: "codex".to_string(),
                    source: "oauth".to_string(),
                    fetched_at: Utc::now(),
                    payload: Some(serde_json::json!({"cached": true})),
                    error: None,
                },
            );
        })
        .await;

    let err = refresh_provider_usage(&host)
        .await
        .expect_err("invalid managed config should fail usage refresh");
    assert!(err.to_string().contains("loading agent server config"));

    let snapshot = host
        .provider_runtime
        .with_provider_usage_cache(|cache| cache.get("codex").cloned())
        .await
        .expect("usage cache entry should be replaced with an error snapshot");
    assert_eq!(snapshot.source, "error");
    assert!(snapshot.payload.is_none());
    assert!(
        snapshot
            .error
            .as_deref()
            .is_some_and(|value| value.contains("loading agent server config")),
        "expected managed config error snapshot: {snapshot:?}"
    );
}
