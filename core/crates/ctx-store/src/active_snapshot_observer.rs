use std::sync::{Arc, Mutex, OnceLock, Weak};

use async_trait::async_trait;

use ctx_core::ids::SessionId;
use ctx_core::models::SessionHeadSnapshot;

#[async_trait]
pub trait ActiveSnapshotObserver: Send + Sync {
    async fn on_active_head_snapshot(&self, head: SessionHeadSnapshot);
    async fn on_active_head_removed(&self, session_id: SessionId);
}

static ACTIVE_SNAPSHOT_OBSERVERS: OnceLock<Mutex<Vec<Weak<dyn ActiveSnapshotObserver>>>> =
    OnceLock::new();

fn observers() -> &'static Mutex<Vec<Weak<dyn ActiveSnapshotObserver>>> {
    ACTIVE_SNAPSHOT_OBSERVERS.get_or_init(|| Mutex::new(Vec::new()))
}

pub fn register_active_snapshot_observer(observer: Arc<dyn ActiveSnapshotObserver>) {
    let mut guard = observers()
        .lock()
        .expect("active snapshot observers mutex poisoned");
    guard.push(Arc::downgrade(&observer));
}
