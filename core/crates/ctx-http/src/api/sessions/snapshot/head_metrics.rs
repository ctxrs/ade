use std::time::Duration;

use crate::daemon::SessionsHandle;
use ctx_core::models::SessionHeadSnapshot;

pub(super) fn record_session_head_recovery_metrics(
    state: &SessionsHandle,
    source: &'static str,
    result: &'static str,
    elapsed: Duration,
    limit: u32,
    include_events: bool,
    head: Option<&SessionHeadSnapshot>,
) {
    state.record_session_head_recovery_metrics(
        source,
        result,
        elapsed,
        limit,
        include_events,
        head,
    );
}
