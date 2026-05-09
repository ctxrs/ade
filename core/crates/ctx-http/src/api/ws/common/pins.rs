use std::collections::HashSet;
use std::sync::Arc;

use ctx_core::ids::SessionId;

use crate::daemon::AppState;

pub(in crate::api::ws) async fn sync_workspace_stream_session_pins<I, J>(
    state: &Arc<AppState>,
    current: I,
    next: J,
) where
    I: IntoIterator<Item = SessionId>,
    J: IntoIterator<Item = SessionId>,
{
    let current = current.into_iter().collect::<HashSet<_>>();
    let next = next.into_iter().collect::<HashSet<_>>();
    for session_id in next.difference(&current) {
        state.attach_session(*session_id).await;
    }
    for session_id in current.difference(&next) {
        state.detach_session(*session_id).await;
    }
}

pub(in crate::api::ws) async fn release_workspace_stream_session_pins<I>(
    state: &Arc<AppState>,
    current: I,
) where
    I: IntoIterator<Item = SessionId>,
{
    for session_id in current {
        state.detach_session(session_id).await;
    }
}
