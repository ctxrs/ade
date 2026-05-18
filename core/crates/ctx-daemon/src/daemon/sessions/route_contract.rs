use std::time::Instant;

use ctx_core::ids::{SessionId, TurnId};
use ctx_core::models::{
    SessionEventsPage, SessionHeadSnapshot, SessionHistoryPage, SessionSnapshot, SessionState,
    SessionTurnTool,
};
use serde::{Deserialize, Serialize};

use crate::daemon::SessionsHandle;

const SESSION_HEAD_CACHE: &str = "session_head";
const SESSION_EVENTS_DEFAULT_LIMIT: u32 = 200;
const SESSION_EVENTS_MAX_LIMIT: u32 = 1000;

#[derive(Debug, Clone)]
pub struct SessionRouteParams {
    session_id: String,
}

impl SessionRouteParams {
    pub fn new(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SessionTurnToolsRouteParams {
    session_id: String,
    turn_id: String,
}

impl SessionTurnToolsRouteParams {
    pub fn new(session_id: impl Into<String>, turn_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            turn_id: turn_id.into(),
        }
    }
}

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
        let session_id = parse_session_id(&params.session_id)?;
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
        let session_id = parse_session_id(&params.session_id)?;
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
        let session_id = parse_session_id(&params.session_id)?;
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
        let session_id = parse_session_id(&params.session_id)?;
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
        let session_id = parse_session_id(&params.session_id)?;
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
        let session_id = parse_session_id(&params.session_id)?;
        let turn_id = parse_turn_id(&params.turn_id)?;
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

fn parse_session_id(value: &str) -> Result<SessionId, SessionReadModelRouteError> {
    uuid::Uuid::parse_str(value)
        .map(SessionId)
        .map_err(|_| SessionReadModelRouteError::bad_request("invalid session id"))
}

fn parse_turn_id(value: &str) -> Result<TurnId, SessionReadModelRouteError> {
    uuid::Uuid::parse_str(value)
        .map(TurnId)
        .map_err(|_| SessionReadModelRouteError::bad_request("invalid turn id"))
}

fn parse_boolish_flag(raw: Option<&str>, label: &str) -> Result<bool, SessionReadModelRouteError> {
    match raw {
        Some(value) => ctx_core::boolish::parse_boolish(value).ok_or_else(|| {
            SessionReadModelRouteError::bad_request(format!(
                "{label} must be one of: 1/true/yes/on or 0/false/no/off"
            ))
        }),
        None => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use ctx_core::ids::{
        ArtifactId, MessageId, RunId, SessionEventId, TaskId, WorkspaceId, WorktreeId,
    };
    use ctx_core::models::{
        Artifact, Message, MessageDelivery, MessageRole, SessionActivityState, SessionEvent,
        SessionEventType, SessionGitStatusSummary, SessionMetadata, SessionSnapshotSummary,
        SessionStatus, SessionTurn, SessionTurnStatus, SessionTurnTool,
    };
    use serde_json::json;

    fn now(minute: u32) -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 17, 10, minute, 0).unwrap()
    }

    fn session_metadata() -> SessionMetadata {
        SessionMetadata {
            id: SessionId::new(),
            task_id: TaskId::new(),
            workspace_id: WorkspaceId::new(),
            worktree_id: WorktreeId::new(),
            execution_environment: ctx_core::models::ExecutionEnvironment::Host,
            parent_session_id: None,
            relationship: None,
            provider_id: "fake".to_string(),
            model_id: "fake-model".to_string(),
            reasoning_effort: Some("high".to_string()),
            title: "session".to_string(),
            agent_role: "default".to_string(),
            status: SessionStatus::Active,
            provider_session_ref: None,
            created_at: now(0),
            updated_at: now(1),
        }
    }

    fn message(session_id: SessionId, task_id: TaskId, turn_id: TurnId) -> Message {
        Message {
            id: MessageId::new(),
            session_id,
            task_id,
            run_id: Some(RunId::new()),
            turn_id: Some(turn_id),
            turn_sequence: Some(1),
            order_seq: Some(2),
            role: MessageRole::Assistant,
            content: "hello".to_string(),
            attachments: Vec::new(),
            delivery: MessageDelivery::Immediate,
            delivered_at: Some(now(2)),
            created_at: now(2),
        }
    }

    fn turn(session_id: SessionId, turn_id: TurnId) -> SessionTurn {
        SessionTurn {
            turn_id,
            session_id,
            run_id: Some(RunId::new()),
            user_message_id: Some(MessageId::new()),
            status: SessionTurnStatus::Completed,
            start_seq: Some(1),
            end_seq: Some(5),
            started_at: now(1),
            updated_at: now(2),
            assistant_partial: Some("partial".to_string()),
            thought_partial: None,
            metrics_json: Some(json!({"tokens": 1})),
            failure: None,
            tool_total: 1,
            tool_pending: 0,
            tool_running: 0,
            tool_completed: 1,
            tool_failed: 0,
        }
    }

    fn turn_tool(session_id: SessionId, turn_id: TurnId) -> SessionTurnTool {
        SessionTurnTool {
            session_id,
            tool_call_id: "tool-call".to_string(),
            turn_id,
            tool_kind: Some("shell".to_string()),
            provider_tool_name: Some("exec".to_string()),
            title: Some("Run command".to_string()),
            subtitle: Some("cargo test".to_string()),
            status: Some("completed".to_string()),
            input_json: Some(json!({"cmd": "cargo test"})),
            output_text: Some("ok".to_string()),
            order_seq: 1,
            first_event_seq: Some(2),
            input_truncated: Some(false),
            input_original_bytes: Some(10),
            output_truncated: Some(false),
            output_original_bytes: Some(2),
            created_at: now(1),
            updated_at: now(2),
        }
    }

    fn event(session_id: SessionId, transient: bool) -> SessionEvent {
        SessionEvent {
            seq: 7,
            id: SessionEventId::new(),
            session_id,
            run_id: Some(RunId::new()),
            turn_id: Some(TurnId::new()),
            event_type: SessionEventType::AssistantChunk,
            payload_json: json!({"text": "hello"}),
            transient,
            created_at: now(3),
        }
    }

    fn state() -> SessionState {
        SessionState {
            artifacts: vec![Artifact {
                id: ArtifactId::new(),
                session_id: SessionId::new(),
                task_id: TaskId::new(),
                workspace_id: WorkspaceId::new(),
                worktree_id: WorktreeId::new(),
                name: Some("log.txt".to_string()),
                absolute_path: "/tmp/log.txt".to_string(),
                mime_type: "text/plain".to_string(),
                bytes: 12,
                created_at: now(4),
                missing: Some(true),
            }],
            git_status: Some(SessionGitStatusSummary {
                summary_line: "clean".to_string(),
                branch: Some("main".to_string()),
                upstream: None,
                ahead: 0,
                behind: 0,
                detached: false,
                staged: 0,
                unstaged: 0,
                untracked: 0,
            }),
        }
    }

    fn snapshot() -> SessionSnapshot {
        let session = session_metadata();
        SessionSnapshot {
            summary: SessionSnapshotSummary {
                session: session.clone(),
                last_message_at: Some(now(2)),
                last_message_preview: Some("hello".to_string()),
                last_event_seq: Some(7),
                projection_rev: 3,
                state_rev: 4,
                activity: SessionActivityState {
                    is_working: true,
                    last_turn_status: Some(SessionTurnStatus::Running),
                },
                unread: Some(true),
            },
            head: Some(SessionHeadSnapshot {
                session,
                turns: Vec::new(),
                tool_summaries: Vec::new(),
                events: vec![event(SessionId::new(), true)],
                messages: Vec::new(),
                last_event_seq: 7,
                projection_rev: 3,
                state_rev: 4,
                activity: SessionActivityState::default(),
                has_more_turns: false,
                history_cursor: Some(1),
                has_more_history: true,
                summary_checkpoint: None,
                head_window: Default::default(),
            }),
            state: Some(state()),
        }
    }

    #[test]
    fn route_wrappers_preserve_read_model_wire_shapes() {
        let snapshot = snapshot();
        assert_eq!(
            serde_json::to_value(SessionSnapshotRouteResponse::from(snapshot.clone())).unwrap(),
            serde_json::to_value(&snapshot).unwrap()
        );

        let head = snapshot.head.clone().unwrap();
        assert_eq!(
            serde_json::to_value(SessionHeadRouteResponse::from(head.clone())).unwrap(),
            serde_json::to_value(head).unwrap()
        );

        let turn_id = TurnId::new();
        let session_id = SessionId::new();
        let task_id = TaskId::new();
        let history = SessionHistoryPage {
            session_id,
            turns: vec![turn(session_id, turn_id)],
            messages: vec![message(session_id, task_id, turn_id)],
            next_cursor: Some(1),
            has_more: true,
        };
        assert_eq!(
            serde_json::to_value(SessionHistoryRouteResponse::from(history.clone())).unwrap(),
            serde_json::to_value(history).unwrap()
        );

        let events = SessionEventsPage {
            session_id,
            events: vec![event(session_id, true)],
            next_cursor: Some(7),
            has_more: false,
        };
        let events_value =
            serde_json::to_value(SessionEventsRouteResponse::from(events.clone())).unwrap();
        assert_eq!(events_value, serde_json::to_value(events).unwrap());
        assert_eq!(events_value["events"][0]["seq"], serde_json::Value::Null);

        let state = state();
        assert_eq!(
            serde_json::to_value(SessionStateRouteResponse::from(state.clone())).unwrap(),
            serde_json::to_value(state).unwrap()
        );

        let tools = vec![turn_tool(session_id, turn_id)];
        assert_eq!(
            serde_json::to_value(SessionTurnToolsRouteResponse::from(tools.clone())).unwrap(),
            serde_json::to_value(tools).unwrap()
        );
    }

    #[test]
    fn route_params_parse_ids_and_reject_bad_values() {
        let session_id = SessionId::new();
        assert_eq!(
            parse_session_id(&session_id.0.to_string()).unwrap(),
            session_id
        );
        let error = parse_session_id("not-a-session").unwrap_err();
        assert_eq!(error.kind(), SessionReadModelRouteErrorKind::BadRequest);
        assert_eq!(error.message(), "invalid session id");

        let turn_id = TurnId::new();
        assert_eq!(parse_turn_id(&turn_id.0.to_string()).unwrap(), turn_id);
        let error = parse_turn_id("not-a-turn").unwrap_err();
        assert_eq!(error.kind(), SessionReadModelRouteErrorKind::BadRequest);
        assert_eq!(error.message(), "invalid turn id");
    }

    #[test]
    fn boolish_flags_preserve_current_values_and_errors() {
        for value in ["1", "true", "yes", "on"] {
            assert!(parse_boolish_flag(Some(value), "include_events").unwrap());
        }
        for value in ["0", "false", "no", "off"] {
            assert!(!parse_boolish_flag(Some(value), "include_events").unwrap());
        }
        assert!(!parse_boolish_flag(None, "include_events").unwrap());

        let error = parse_boolish_flag(Some("maybe"), "include_events").unwrap_err();
        assert_eq!(error.kind(), SessionReadModelRouteErrorKind::BadRequest);
        assert!(error.message().contains("include_events must be one of"));
    }

    #[test]
    fn query_defaults_preserve_non_clamped_limits_except_events() {
        let snapshot = SessionSnapshotRouteQuery::default();
        assert_eq!(snapshot.limit.unwrap_or(60), 60);
        let snapshot = SessionSnapshotRouteQuery {
            limit: Some(0),
            include_events: None,
        };
        assert_eq!(snapshot.limit.unwrap_or(60), 0);

        let history = SessionHistoryRouteQuery {
            before_seq: None,
            limit: Some(u32::MAX),
        };
        assert_eq!(history.limit.unwrap_or(60), u32::MAX);

        let events = SessionEventsRouteQuery {
            after_seq: None,
            limit: Some(u32::MAX),
            tail: Some(0),
            include_transient: None,
        };
        assert_eq!(
            events
                .limit
                .unwrap_or(SESSION_EVENTS_DEFAULT_LIMIT)
                .clamp(1, SESSION_EVENTS_MAX_LIMIT),
            SESSION_EVENTS_MAX_LIMIT
        );
    }

    #[test]
    fn negative_min_event_seq_is_bad_request() {
        let query = SessionHeadRouteQuery {
            limit: None,
            include_events: None,
            min_event_seq: Some(-1),
        };

        assert!(matches!(query.min_event_seq, Some(value) if value < 0));
    }
}
