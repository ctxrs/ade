use std::sync::OnceLock;
use tokio::sync::Mutex as AsyncMutex;

pub(crate) fn podman_env_test_lock() -> &'static AsyncMutex<()> {
    static LOCK: OnceLock<AsyncMutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| AsyncMutex::new(()))
}
