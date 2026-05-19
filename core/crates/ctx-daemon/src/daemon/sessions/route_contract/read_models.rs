use std::time::Instant;

use ctx_core::ids::{SessionId, TurnId};
use ctx_core::models::{
    SessionEventsPage, SessionHeadSnapshot, SessionHistoryPage, SessionSnapshot, SessionState,
    SessionTurnTool,
};
use serde::{Deserialize, Serialize};

use crate::daemon::SessionsHandle;

use super::common::{parse_session_route_id, SessionRouteParams, SessionTurnToolsRouteParams};

const SESSION_HEAD_CACHE: &str = "session_head";
pub(super) const SESSION_EVENTS_DEFAULT_LIMIT: u32 = 200;
pub(super) const SESSION_EVENTS_MAX_LIMIT: u32 = 1000;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SessionSnapshotRouteQuery {
    pub limit: Option<u32>,
    pub include_events: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SessionHeadRouteQuery {
    pub limit: Option<u32>,
    pub include_events: Option<String>,
    pub min_event_seq: Option<i64>,
}

#[derive(Debug, Clone, Copy, Deserialize, Default)]
pub struct SessionHistoryRouteQuery {
    pub before_seq: Option<i64>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SessionEventsRouteQuery {
    pub after_seq: Option<i64>,
    pub limit: Option<u32>,
    pub tail: Option<u32>,
    pub include_transient: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(transparent)]
pub struct SessionSnapshotRouteResponse(SessionSnapshot);

#[derive(Debug, Clone, Serialize)]
#[serde(transparent)]
pub struct SessionHeadRouteResponse(SessionHeadSnapshot);

#[derive(Debug, Clone, Serialize)]
#[serde(transparent)]
pub struct SessionHistoryRouteResponse(SessionHistoryPage);

#[derive(Debug, Clone, Serialize)]
#[serde(transparent)]
pub struct SessionEventsRouteResponse(SessionEventsPage);

#[derive(Debug, Clone, Serialize)]
#[serde(transparent)]
pub struct SessionStateRouteResponse(SessionState);

#[derive(Debug, Clone, Serialize)]
#[serde(transparent)]
pub struct SessionTurnToolsRouteResponse(Vec<SessionTurnTool>);

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum SessionReadModelRouteErrorKind {
    BadRequest,
    NotFound,
    Conflict,
    Internal,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SessionReadModelRouteError {
    kind: SessionReadModelRouteErrorKind,
    message: String,
}

impl SessionReadModelRouteError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            kind: SessionReadModelRouteErrorKind::BadRequest,
            message: message.into(),
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            kind: SessionReadModelRouteErrorKind::NotFound,
            message: message.into(),
        }
    }

    fn conflict(message: impl Into<String>) -> Self {
        Self {
            kind: SessionReadModelRouteErrorKind::Conflict,
            message: message.into(),
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self {
            kind: SessionReadModelRouteErrorKind::Internal,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> SessionReadModelRouteErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl SessionsHandle {
    pub async fn load_session_snapshot_for_route(
        &self,
        params: SessionRouteParams,
        query: SessionSnapshotRouteQuery,
    ) -> Result<SessionSnapshotRouteResponse, SessionReadModelRouteError> {
        let session_id = parse_session_id(params.session_id())?;
        let limit = query.limit.unwrap_or(60);
        let include_events = parse_boolish_flag(query.include_events.as_deref(), "include_events")?;
        self.load_session_snapshot(session_id, limit, include_events)
            .await
            .map_err(|error| {
                tracing::warn!(session_id = %session_id.0, "session snapshot load failed: {error:#}");
                SessionReadModelRouteError::internal("failed to load session snapshot")
            })?
            .map(Into::into)
            .ok_or_else(|| SessionReadModelRouteError::not_found("session not found"))
    }

    pub async fn session_head_for_route(
        &self,
        params: SessionRouteParams,
        query: SessionHeadRouteQuery,
    ) -> Result<SessionHeadRouteResponse, SessionReadModelRouteError> {
        let session_id = parse_session_id(params.session_id())?;
        let limit = query.limit.unwrap_or(60);
        let include_events = parse_boolish_flag(query.include_events.as_deref(), "include_events")?;
        let min_event_seq = query.min_event_seq;

        let started_at = Instant::now();
        if matches!(min_event_seq, Some(value) if value < 0) {
            return Err(SessionReadModelRouteError::bad_request(
                "min_event_seq must be non-negative",
            ));
        }

        let workspace_id = match self.workspace_id_for_session(session_id).await {
            Ok(Some(workspace_id)) => workspace_id,
            Ok(None) => return Err(SessionReadModelRouteError::not_found("session not found")),
            Err(error) => {
                tracing::warn!(session_id = %session_id.0, "session workspace lookup failed: {error:#}");
                return Err(SessionReadModelRouteError::internal(
                    "failed to load session head",
                ));
            }
        };
        if self.is_workspace_deleting(workspace_id).await {
            return Err(SessionReadModelRouteError::not_found("session not found"));
        }

        if let Some(head) = self
            .cached_session_head_for_request(session_id, include_events, limit, min_event_seq)
            .await
        {
            self.record_session_head_recovery_metrics(
                "active_snapshot_cache",
                "ok",
                started_at.elapsed(),
                limit,
                include_events,
                Some(&head),
            );
            return Ok(head.into());
        }

        self.emit_cache_miss(SESSION_HEAD_CACHE).await;
        match self
            .load_session_head_snapshot_from_store(session_id, limit, include_events)
            .await
        {
            Ok(Some(head)) => {
                if matches!(min_event_seq, Some(min_event_seq) if head.last_event_seq < min_event_seq)
                {
                    self.emit_cache_rehydrate(SESSION_HEAD_CACHE, false).await;
                    self.record_session_head_recovery_metrics(
                        "store_rebuild",
                        "stale",
                        started_at.elapsed(),
                        limit,
                        include_events,
                        Some(&head),
                    );
                    return Err(SessionReadModelRouteError::conflict(
                        "session head is stale",
                    ));
                }

                self.emit_cache_rehydrate(SESSION_HEAD_CACHE, true).await;
                self.record_session_head_recovery_metrics(
                    "store_rebuild",
                    "ok",
                    started_at.elapsed(),
                    limit,
                    include_events,
                    Some(&head),
                );
                self.update_session_head_cache(head.clone(), include_events)
                    .await;
                Ok(head.into())
            }
            Ok(None) => {
                self.emit_cache_rehydrate(SESSION_HEAD_CACHE, false).await;
                self.record_session_head_recovery_metrics(
                    "store_rebuild",
                    "missing",
                    started_at.elapsed(),
                    limit,
                    include_events,
                    None,
                );
                Err(SessionReadModelRouteError::not_found("session not found"))
            }
            Err(error) => {
                self.emit_cache_rehydrate(SESSION_HEAD_CACHE, false).await;
                self.record_session_head_recovery_metrics(
                    "store_rebuild",
                    "error",
                    started_at.elapsed(),
                    limit,
                    include_events,
                    None,
                );
                tracing::warn!(session_id = %session_id.0, "session head load failed: {error:#}");
                Err(SessionReadModelRouteError::internal(
                    "failed to load session head",
                ))
            }
        }
    }

    pub async fn load_session_history_page_for_route(
        &self,
        params: SessionRouteParams,
        query: SessionHistoryRouteQuery,
    ) -> Result<SessionHistoryRouteResponse, SessionReadModelRouteError> {
        let session_id = parse_session_id(params.session_id())?;
        let limit = query.limit.unwrap_or(60);
        self.load_session_history_page(session_id, query.before_seq, limit)
            .await
            .map_err(|error| {
                tracing::warn!(session_id = %session_id.0, "session history load failed: {error:#}");
                SessionReadModelRouteError::internal("failed to load session history")
            })?
            .map(Into::into)
            .ok_or_else(|| SessionReadModelRouteError::not_found("session not found"))
    }

    pub async fn list_session_events_page_for_route(
        &self,
        params: SessionRouteParams,
        query: SessionEventsRouteQuery,
    ) -> Result<SessionEventsRouteResponse, SessionReadModelRouteError> {
        let session_id = parse_session_id(params.session_id())?;
        let limit = query
            .limit
            .unwrap_or(SESSION_EVENTS_DEFAULT_LIMIT)
            .clamp(1, SESSION_EVENTS_MAX_LIMIT);
        let include_transient =
            parse_boolish_flag(query.include_transient.as_deref(), "include_transient")?;
        self.list_session_events_page(
            session_id,
            query.after_seq,
            limit,
            query.tail,
            include_transient,
        )
        .await
        .map_err(|error| {
            tracing::warn!(session_id = %session_id.0, "session events load failed: {error:#}");
            SessionReadModelRouteError::internal("failed to load session events")
        })?
        .map(Into::into)
        .ok_or_else(|| SessionReadModelRouteError::not_found("session not found"))
    }

    pub async fn load_session_state_for_route(
        &self,
        params: SessionRouteParams,
    ) -> Result<SessionStateRouteResponse, SessionReadModelRouteError> {
        let session_id = parse_session_id(params.session_id())?;
        self.load_session_state(session_id)
            .await
            .map_err(|error| {
                tracing::warn!(session_id = %session_id.0, "session state load failed: {error:#}");
                SessionReadModelRouteError::internal("failed to load session state")
            })?
            .map(Into::into)
            .ok_or_else(|| SessionReadModelRouteError::not_found("session not found"))
    }

    pub async fn list_session_turn_tools_for_route(
        &self,
        params: SessionTurnToolsRouteParams,
    ) -> Result<SessionTurnToolsRouteResponse, SessionReadModelRouteError> {
        let session_id = parse_session_id(params.session_id())?;
        let turn_id = parse_turn_id(params.turn_id())?;
        self.list_session_turn_tools_for_request(session_id, turn_id)
            .await
            .map_err(|error| {
                tracing::warn!(session_id = %session_id.0, turn_id = %turn_id.0, "session turn tools load failed: {error:#}");
                SessionReadModelRouteError::internal("failed to load session turn tools")
            })?
            .map(Into::into)
            .ok_or_else(|| SessionReadModelRouteError::not_found("session not found"))
    }
}

impl From<SessionSnapshot> for SessionSnapshotRouteResponse {
    fn from(snapshot: SessionSnapshot) -> Self {
        Self(snapshot)
    }
}

impl From<SessionHeadSnapshot> for SessionHeadRouteResponse {
    fn from(head: SessionHeadSnapshot) -> Self {
        Self(head)
    }
}

impl From<SessionHistoryPage> for SessionHistoryRouteResponse {
    fn from(page: SessionHistoryPage) -> Self {
        Self(page)
    }
}

impl From<SessionEventsPage> for SessionEventsRouteResponse {
    fn from(page: SessionEventsPage) -> Self {
        Self(page)
    }
}

impl From<SessionState> for SessionStateRouteResponse {
    fn from(state: SessionState) -> Self {
        Self(state)
    }
}

impl From<Vec<SessionTurnTool>> for SessionTurnToolsRouteResponse {
    fn from(tools: Vec<SessionTurnTool>) -> Self {
        Self(tools)
    }
}

pub(super) fn parse_session_id(value: &str) -> Result<SessionId, SessionReadModelRouteError> {
    parse_session_route_id(value)
        .map_err(|_| SessionReadModelRouteError::bad_request("invalid session id"))
}

pub(super) fn parse_turn_id(value: &str) -> Result<TurnId, SessionReadModelRouteError> {
    uuid::Uuid::parse_str(value)
        .map(TurnId)
        .map_err(|_| SessionReadModelRouteError::bad_request("invalid turn id"))
}

pub(super) fn parse_boolish_flag(
    raw: Option<&str>,
    label: &str,
) -> Result<bool, SessionReadModelRouteError> {
    match raw {
        Some(value) => ctx_core::boolish::parse_boolish(value).ok_or_else(|| {
            SessionReadModelRouteError::bad_request(format!(
                "{label} must be one of: 1/true/yes/on or 0/false/no/off"
            ))
        }),
        None => Ok(false),
    }
}
