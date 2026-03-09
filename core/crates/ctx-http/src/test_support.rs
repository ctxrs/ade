use std::sync::{Mutex as StdMutex, OnceLock};

pub(crate) fn podman_env_test_lock() -> &'static StdMutex<()> {
    static LOCK: OnceLock<StdMutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| StdMutex::new(()))
}
