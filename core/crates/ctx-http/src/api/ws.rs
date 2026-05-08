use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use futures::{Sink, SinkExt, StreamExt};
use serde_json::Value;
use tokio::sync::{Mutex, Notify};
use tokio::task::JoinSet;

use ctx_core::ids::*;
use ctx_core::models::*;
use ctx_workspace_active_snapshot::{is_transient_session_delta, SessionReplayCursor};

use crate::daemon::AppState;
use ctx_transport_runtime::terminals::{TerminalClientMessage, TerminalServerMessage};

use super::SecureEnvelope;

mod async_util;
mod common;
mod dictation_livekit;
mod queue;
mod replay;
mod secure_mobile;
mod terminal;
mod web_session;
mod workspace_active;
mod workspace_stream;
mod workspace_vcs;

use common::{
    accept_session_delta, accept_session_head, bump_latest_snapshot_rev, event_snapshot_rev,
    release_workspace_stream_session_pins, sync_workspace_stream_session_pins,
    ResolvedWorkspaceActiveSessionReplay, ResolvedWorkspaceActiveSubscriptions, SessionCursor,
    StreamSendControl, WorkspaceActiveSubscriptionState, HEAD_BATCH_FLUSH_INTERVAL,
    HEAD_BATCH_SESSION_LIMIT, WORKSPACE_STREAM_QUEUE_LIMIT, WORKSPACE_STREAM_QUEUE_MAX_AGE,
};
use queue::{
    filter_partial_delta_for_active_tasks, is_foreground_session, is_priority_control_event,
    log_head_batch_push_error, log_summary_batch_push_error, push_stream_message,
    should_stream_head_delta, take_next_workspace_stream_item, workspace_stream_is_idle,
    HeadBatchBuffer, NextWorkspaceStreamItem, StreamQueue, SummaryBatchBuffer,
    HEAD_BATCH_TOTAL_LIMIT,
};
use replay::{
    queue_reset_required, queue_snapshot_payload, replay_session_events,
    resolve_workspace_active_snapshot_subscriptions, with_stream_rev, ReplayOutcome,
};

pub(super) use secure_mobile::mobile_secure_workspace_stream_ws;
pub(super) use terminal::terminal_stream_ws;
#[cfg(test)]
use terminal::{
    queue_terminal_ws_message, queue_terminal_ws_tail_resync_if_requested, TerminalWsQueueOutcome,
};
pub(super) use web_session::web_session_signal;
pub(super) use workspace_active::workspace_active_snapshot_stream_ws;
pub(super) use workspace_vcs::workspace_vcs_stream_ws;

pub(super) async fn dictation_livekit_stream_ws(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| dictation_livekit::dictation_livekit_stream(socket, state))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[path = "ws_queue_tests.rs"]
    mod ws_queue_tests;
}
