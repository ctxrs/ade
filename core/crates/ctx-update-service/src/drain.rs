use std::time::SystemTime;

use anyhow::Result;
use tokio::sync::Mutex;

#[derive(Clone, Debug, serde::Serialize)]
pub struct UpdateDrainState {
    pub reason: String,
    pub owner: String,
    pub acquired_at_ms: u64,
}

#[derive(Debug, Default)]
pub struct UpdateDrainCoordinator {
    state: Mutex<Option<UpdateDrainState>>,
}

impl UpdateDrainCoordinator {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn snapshot(&self) -> Option<UpdateDrainState> {
        self.state.lock().await.clone()
    }

    pub async fn acquire(
        &self,
        reason: impl Into<String>,
        owner: impl Into<String>,
    ) -> Option<UpdateDrainState> {
        let mut guard = self.state.lock().await;
        if guard.is_some() {
            return None;
        }
        let state = UpdateDrainState {
            reason: reason.into(),
            owner: owner.into(),
            acquired_at_ms: current_time_ms(),
        };
        *guard = Some(state.clone());
        Some(state)
    }

    pub async fn release(&self) -> bool {
        self.state.lock().await.take().is_some()
    }

    pub async fn reject_if_draining(&self) -> Result<()> {
        if let Some(drain) = self.snapshot().await {
            anyhow::bail!(
                "daemon maintenance is in progress; retry after it completes (reason={}, owner={})",
                drain.reason,
                drain.owner
            );
        }
        Ok(())
    }
}

fn current_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::UpdateDrainCoordinator;

    #[tokio::test]
    async fn update_drain_acquire_reject_and_release_round_trip() {
        let drain = UpdateDrainCoordinator::new();

        let acquired = drain
            .acquire("test_update", "unit_test")
            .await
            .expect("first acquire");
        assert_eq!(acquired.reason, "test_update");
        assert!(drain.acquire("second", "unit_test").await.is_none());

        let error = drain
            .reject_if_draining()
            .await
            .expect_err("drain should reject");
        assert!(error.to_string().contains("test_update"));

        assert!(drain.release().await);
        assert!(drain.reject_if_draining().await.is_ok());
        assert!(!drain.release().await);
    }
}
