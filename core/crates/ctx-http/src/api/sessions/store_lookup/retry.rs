use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use ctx_core::ids::SessionId;

use crate::daemon::{AppState, StoreLookup};

const STORE_OPEN_RETRY_LIMIT: usize = 3;
const STORE_OPEN_RETRY_BASE_MS: u64 = 40;

pub(super) async fn store_for_existing_session_status_with_retry(
    state: &Arc<AppState>,
    session_id: SessionId,
) -> Result<ctx_store::Store, StatusCode> {
    let mut attempt = 0usize;
    loop {
        match state.lookup_session_store(session_id).await {
            StoreLookup::Found(store) => return Ok(store),
            StoreLookup::Missing | StoreLookup::Deleting => {
                return Err(StatusCode::NOT_FOUND);
            }
            StoreLookup::Unavailable(err) => {
                if is_transient_store_open_error(&err) && attempt < STORE_OPEN_RETRY_LIMIT {
                    attempt += 1;
                    let backoff_ms = STORE_OPEN_RETRY_BASE_MS.saturating_mul(attempt as u64);
                    tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                    continue;
                }
                tracing::warn!(session_id = %session_id.0, "session store lookup failed: {err:#}");
                return Err(StatusCode::INTERNAL_SERVER_ERROR);
            }
        }
    }
}

fn is_transient_store_open_error(err: &anyhow::Error) -> bool {
    let msg = err.to_string().to_lowercase();
    msg.contains("database is locked")
        || msg.contains("sqlite_busy")
        || msg.contains("database is busy")
}
