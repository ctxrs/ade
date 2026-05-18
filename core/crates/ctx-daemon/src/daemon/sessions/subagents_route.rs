use ctx_core::ids::{SessionId, TurnId};
use ctx_core::models::{SessionSummary, SubagentInvocation};
use ctx_observability::logs;
use serde::{Deserialize, Serialize};

use crate::daemon::sessions::route_contract::parse_session_route_id;
use crate::daemon::sessions::subagents::{self, SubagentError, SubagentErrorKind};
use crate::daemon::{ScopedMcpSessionAccessError, SessionRouteParams, SessionsHandle};

#[derive(Debug, Clone, Copy, Default)]
pub struct McpSessionRouteContext {
    mcp_auth: Option<ctx_mcp_auth::McpAuthContext>,
}

impl McpSessionRouteContext {
    pub fn new(mcp_auth: Option<ctx_mcp_auth::McpAuthContext>) -> Self {
        Self { mcp_auth }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SessionSubagentInvocationsRouteQuery {
    #[serde(default)]
    turn_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SpawnAgentRouteRequest {
    #[serde(default)]
    tool_call_id: Option<String>,
    #[serde(default)]
    worktree: Option<String>,
    task_label: String,
    prompt: String,
    #[serde(default)]
    harness: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    reasoning_effort: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SendInputRouteRequest {
    agent_id: String,
    message: String,
    #[serde(default)]
    interrupt: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ArchiveAgentRouteRequest {
    agent_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GetAgentRouteRequest {
    agent_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InterruptAgentRouteRequest {
    agent_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WaitAgentRouteRequest {
    #[serde(default)]
    agent_id: Option<String>,
    #[serde(default)]
    agent_ids: Option<Vec<String>>,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    until: Option<String>,
    #[serde(default)]
    since_seq: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(transparent)]
pub struct SessionSubagentsRouteResponse(Vec<SessionSummary>);

#[derive(Debug, Serialize)]
#[serde(transparent)]
pub struct SessionSubagentInvocationsRouteResponse(Vec<SubagentInvocation>);

#[derive(Debug, Serialize)]
#[serde(transparent)]
pub struct SessionSubagentInvocationRouteResponse(SubagentInvocation);

#[derive(Debug, Serialize)]
#[serde(transparent)]
pub struct SpawnAgentRouteResponse(subagents::SpawnAgentResp);

#[derive(Debug, Serialize)]
#[serde(transparent)]
pub struct SendInputRouteResponse(subagents::SendInputResp);

#[derive(Debug, Serialize)]
#[serde(transparent)]
pub struct ArchiveAgentRouteResponse(subagents::ArchiveAgentResp);

#[derive(Debug, Serialize)]
#[serde(transparent)]
pub struct ListAgentsRouteResponse(Vec<subagents::AgentSummary>);

#[derive(Debug, Serialize)]
#[serde(transparent)]
pub struct GetAgentRouteResponse(subagents::GetAgentResp);

#[derive(Debug, Serialize)]
#[serde(transparent)]
pub struct InterruptAgentRouteResponse(subagents::InterruptAgentResp);

#[derive(Debug, Serialize)]
#[serde(transparent)]
pub struct WaitAgentRouteResponse(subagents::WaitAgentResp);

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum SessionSubagentRouteErrorKind {
    BadRequest,
    Unauthorized,
    Forbidden,
    NotFound,
    InsufficientStorage,
    Internal,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SessionSubagentRouteError {
    kind: SessionSubagentRouteErrorKind,
    message: String,
}

impl SessionSubagentRouteError {
    fn new(kind: SessionSubagentRouteErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    fn bad_request(message: impl Into<String>) -> Self {
        Self::new(SessionSubagentRouteErrorKind::BadRequest, message)
    }

    fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(SessionSubagentRouteErrorKind::Unauthorized, message)
    }

    fn forbidden(message: impl Into<String>) -> Self {
        Self::new(SessionSubagentRouteErrorKind::Forbidden, message)
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self::new(SessionSubagentRouteErrorKind::NotFound, message)
    }

    fn insufficient_storage(message: impl Into<String>) -> Self {
        Self::new(SessionSubagentRouteErrorKind::InsufficientStorage, message)
    }

    fn internal(message: impl Into<String>) -> Self {
        Self::new(SessionSubagentRouteErrorKind::Internal, message)
    }

    pub fn kind(&self) -> SessionSubagentRouteErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl SessionsHandle {
    pub async fn list_session_subagents_for_route(
        &self,
        params: SessionRouteParams,
    ) -> Result<SessionSubagentsRouteResponse, SessionSubagentRouteError> {
        let session_id = parse_subagent_route_id(params)?;
        let subagents = self
            .list_session_subagents_for_request(session_id)
            .await
            .map_err(|_| SessionSubagentRouteError::internal("internal server error"))?
            .ok_or_else(|| SessionSubagentRouteError::not_found("session not found"))?;
        Ok(SessionSubagentsRouteResponse(subagents))
    }

    pub async fn list_session_subagent_invocations_for_route(
        &self,
        params: SessionRouteParams,
        query: SessionSubagentInvocationsRouteQuery,
    ) -> Result<SessionSubagentInvocationsRouteResponse, SessionSubagentRouteError> {
        let session_id = parse_subagent_route_id(params)?;
        let turn_id = parse_optional_turn_id(query.turn_id)?;
        let invocations = self
            .list_session_subagent_invocations_for_request(session_id, turn_id)
            .await
            .map_err(|_| SessionSubagentRouteError::internal("internal server error"))?
            .ok_or_else(|| SessionSubagentRouteError::not_found("session not found"))?;
        Ok(SessionSubagentInvocationsRouteResponse(invocations))
    }

    pub async fn get_session_subagent_invocation_for_route(
        &self,
        params: SessionRouteParams,
        invocation_id: String,
    ) -> Result<SessionSubagentInvocationRouteResponse, SessionSubagentRouteError> {
        let session_id = parse_subagent_route_id(params)?;
        let invocation = self
            .get_session_subagent_invocation_for_request(session_id, &invocation_id)
            .await
            .map_err(|_| SessionSubagentRouteError::internal("internal server error"))?
            .ok_or_else(|| SessionSubagentRouteError::not_found("session not found"))?;
        Ok(SessionSubagentInvocationRouteResponse(invocation))
    }

    pub async fn spawn_agent_for_mcp_route(
        &self,
        params: SessionRouteParams,
        context: McpSessionRouteContext,
        request: SpawnAgentRouteRequest,
    ) -> Result<SpawnAgentRouteResponse, SessionSubagentRouteError> {
        let parent_id = self
            .resolve_mcp_subagent_parent_session_id(params, context)
            .await?;
        self.spawn_agent(parent_id, request.into_low_level())
            .await
            .map(SpawnAgentRouteResponse)
            .map_err(subagent_route_error)
    }

    pub async fn send_input_for_mcp_route(
        &self,
        params: SessionRouteParams,
        context: McpSessionRouteContext,
        request: SendInputRouteRequest,
    ) -> Result<SendInputRouteResponse, SessionSubagentRouteError> {
        let parent_id = self
            .resolve_mcp_subagent_parent_session_id(params, context)
            .await?;
        self.send_input(parent_id, request.into_low_level())
            .await
            .map(SendInputRouteResponse)
            .map_err(subagent_route_error)
    }

    pub async fn archive_agent_for_mcp_route(
        &self,
        params: SessionRouteParams,
        context: McpSessionRouteContext,
        request: ArchiveAgentRouteRequest,
    ) -> Result<ArchiveAgentRouteResponse, SessionSubagentRouteError> {
        let parent_id = self
            .resolve_mcp_subagent_parent_session_id(params, context)
            .await?;
        self.archive_agent(parent_id, request.into_low_level())
            .await
            .map(ArchiveAgentRouteResponse)
            .map_err(subagent_route_error)
    }

    pub async fn list_agents_for_mcp_route(
        &self,
        params: SessionRouteParams,
        context: McpSessionRouteContext,
    ) -> Result<ListAgentsRouteResponse, SessionSubagentRouteError> {
        let parent_id = self
            .resolve_mcp_subagent_parent_session_id(params, context)
            .await?;
        self.list_agents(parent_id)
            .await
            .map(ListAgentsRouteResponse)
            .map_err(subagent_route_error)
    }

    pub async fn get_agent_for_mcp_route(
        &self,
        params: SessionRouteParams,
        context: McpSessionRouteContext,
        request: GetAgentRouteRequest,
    ) -> Result<GetAgentRouteResponse, SessionSubagentRouteError> {
        let parent_id = self
            .resolve_mcp_subagent_parent_session_id(params, context)
            .await?;
        self.get_agent(parent_id, request.into_low_level())
            .await
            .map(GetAgentRouteResponse)
            .map_err(subagent_route_error)
    }

    pub async fn interrupt_agent_for_mcp_route(
        &self,
        params: SessionRouteParams,
        context: McpSessionRouteContext,
        request: InterruptAgentRouteRequest,
    ) -> Result<InterruptAgentRouteResponse, SessionSubagentRouteError> {
        let parent_id = self
            .resolve_mcp_subagent_parent_session_id(params, context)
            .await?;
        self.interrupt_agent(parent_id, request.into_low_level())
            .await
            .map(InterruptAgentRouteResponse)
            .map_err(subagent_route_error)
    }

    pub async fn wait_agent_for_mcp_route(
        &self,
        params: SessionRouteParams,
        context: McpSessionRouteContext,
        request: WaitAgentRouteRequest,
    ) -> Result<WaitAgentRouteResponse, SessionSubagentRouteError> {
        let parent_id = self
            .resolve_mcp_subagent_parent_session_id(params, context)
            .await?;
        self.wait_agent(parent_id, request.into_low_level())
            .await
            .map(WaitAgentRouteResponse)
            .map_err(subagent_route_error)
    }

    async fn resolve_mcp_subagent_parent_session_id(
        &self,
        params: SessionRouteParams,
        context: McpSessionRouteContext,
    ) -> Result<SessionId, SessionSubagentRouteError> {
        let session_id = parse_subagent_route_id(params)?;
        if let Some(mcp_auth) = context.mcp_auth {
            self.require_scoped_mcp_session_context(mcp_auth, session_id)
                .await
                .map_err(scoped_mcp_session_route_error)?;
        }
        Ok(session_id)
    }
}

impl SpawnAgentRouteRequest {
    fn into_low_level(self) -> subagents::SpawnAgentReq {
        subagents::SpawnAgentReq {
            tool_call_id: self.tool_call_id,
            worktree: self.worktree,
            task_label: self.task_label,
            prompt: self.prompt,
            harness: self.harness,
            model: self.model,
            reasoning_effort: self.reasoning_effort,
        }
    }
}

impl SendInputRouteRequest {
    fn into_low_level(self) -> subagents::SendInputReq {
        subagents::SendInputReq {
            agent_id: self.agent_id,
            message: self.message,
            interrupt: self.interrupt,
        }
    }
}

impl ArchiveAgentRouteRequest {
    fn into_low_level(self) -> subagents::ArchiveAgentReq {
        subagents::ArchiveAgentReq {
            agent_id: self.agent_id,
        }
    }
}

impl GetAgentRouteRequest {
    fn into_low_level(self) -> subagents::GetAgentReq {
        subagents::GetAgentReq {
            agent_id: self.agent_id,
        }
    }
}

impl InterruptAgentRouteRequest {
    fn into_low_level(self) -> subagents::InterruptAgentReq {
        subagents::InterruptAgentReq {
            agent_id: self.agent_id,
        }
    }
}

impl WaitAgentRouteRequest {
    fn into_low_level(self) -> subagents::WaitAgentReq {
        subagents::WaitAgentReq {
            agent_id: self.agent_id,
            agent_ids: self.agent_ids,
            timeout_ms: self.timeout_ms,
            mode: self.mode,
            until: self.until,
            since_seq: self.since_seq,
        }
    }
}

fn parse_subagent_route_id(
    params: SessionRouteParams,
) -> Result<SessionId, SessionSubagentRouteError> {
    parse_session_route_id(params.session_id())
        .map_err(|_| SessionSubagentRouteError::bad_request("invalid session id"))
}

fn parse_optional_turn_id(
    raw: Option<String>,
) -> Result<Option<TurnId>, SessionSubagentRouteError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    uuid::Uuid::parse_str(trimmed)
        .map(TurnId)
        .map(Some)
        .map_err(|_| SessionSubagentRouteError::bad_request("invalid turn id"))
}

fn scoped_mcp_session_route_error(error: ScopedMcpSessionAccessError) -> SessionSubagentRouteError {
    match error {
        ScopedMcpSessionAccessError::Unauthorized(message) => {
            SessionSubagentRouteError::unauthorized(message)
        }
        ScopedMcpSessionAccessError::SessionNotFound => {
            SessionSubagentRouteError::not_found("session not found")
        }
        ScopedMcpSessionAccessError::StoreUnavailable(error) => {
            SessionSubagentRouteError::internal(logs::redact_sensitive(&error.to_string()))
        }
    }
}

fn subagent_route_error(error: SubagentError) -> SessionSubagentRouteError {
    match error.kind() {
        SubagentErrorKind::BadRequest => SessionSubagentRouteError::bad_request(error.message()),
        SubagentErrorKind::NotFound => SessionSubagentRouteError::not_found(error.message()),
        SubagentErrorKind::Forbidden => SessionSubagentRouteError::forbidden(error.message()),
        SubagentErrorKind::InsufficientStorage => {
            SessionSubagentRouteError::insufficient_storage(error.message())
        }
        SubagentErrorKind::Internal => SessionSubagentRouteError::internal(error.message()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;
    use serde_json::json;

    #[test]
    fn listing_query_preserves_turn_id_contract() {
        let query: SessionSubagentInvocationsRouteQuery =
            serde_json::from_value(json!({ "turn_id": "  ", "ignored": true })).unwrap();
        assert_eq!(parse_optional_turn_id(query.turn_id).unwrap(), None);

        let turn_id = TurnId::new();
        let query: SessionSubagentInvocationsRouteQuery = serde_json::from_value(json!({
            "turn_id": turn_id.0.to_string()
        }))
        .unwrap();
        assert_eq!(
            parse_optional_turn_id(query.turn_id).unwrap(),
            Some(turn_id)
        );

        let query: SessionSubagentInvocationsRouteQuery =
            serde_json::from_value(json!({ "turn_id": "not-a-turn" })).unwrap();
        let error = parse_optional_turn_id(query.turn_id).unwrap_err();
        assert_eq!(error.kind(), SessionSubagentRouteErrorKind::BadRequest);
        assert_eq!(error.message(), "invalid turn id");
    }

    #[test]
    fn invalid_session_id_uses_existing_route_message() {
        let error = parse_subagent_route_id(SessionRouteParams::new("not-a-session")).unwrap_err();
        assert_eq!(error.kind(), SessionSubagentRouteErrorKind::BadRequest);
        assert_eq!(error.message(), "invalid session id");
    }

    #[test]
    fn mcp_route_requests_preserve_current_serde_shape() {
        let spawn: SpawnAgentRouteRequest = serde_json::from_value(json!({
            "tool_call_id": "tool",
            "worktree": "new",
            "task_label": "child",
            "prompt": "do work",
            "harness": "codex",
            "model": "gpt",
            "reasoning_effort": "high",
            "ignored": true
        }))
        .unwrap();
        let spawn = spawn.into_low_level();
        assert_eq!(spawn.tool_call_id.as_deref(), Some("tool"));
        assert_eq!(spawn.worktree.as_deref(), Some("new"));
        assert_eq!(spawn.task_label, "child");
        assert_eq!(spawn.prompt, "do work");
        assert_eq!(spawn.harness.as_deref(), Some("codex"));
        assert_eq!(spawn.model.as_deref(), Some("gpt"));
        assert_eq!(spawn.reasoning_effort.as_deref(), Some("high"));

        let send: SendInputRouteRequest = serde_json::from_value(json!({
            "agent_id": "agent",
            "message": "hello",
            "interrupt": true,
            "ignored": true
        }))
        .unwrap();
        let send = send.into_low_level();
        assert_eq!(send.agent_id, "agent");
        assert_eq!(send.message, "hello");
        assert_eq!(send.interrupt, Some(true));

        let wait: WaitAgentRouteRequest = serde_json::from_value(json!({
            "agent_id": "agent",
            "agent_ids": ["a", "b"],
            "timeout_ms": 1,
            "mode": "all",
            "until": "update",
            "since_seq": 2,
            "ignored": true
        }))
        .unwrap();
        let wait = wait.into_low_level();
        assert_eq!(wait.agent_id.as_deref(), Some("agent"));
        assert_eq!(wait.agent_ids, Some(vec!["a".to_string(), "b".to_string()]));
        assert_eq!(wait.timeout_ms, Some(1));
        assert_eq!(wait.mode.as_deref(), Some("all"));
        assert_eq!(wait.until.as_deref(), Some("update"));
        assert_eq!(wait.since_seq, Some(2));
    }

    #[test]
    fn route_error_mapping_preserves_status_categories_and_messages() {
        for (kind, expected_kind) in [
            (
                SubagentErrorKind::BadRequest,
                SessionSubagentRouteErrorKind::BadRequest,
            ),
            (
                SubagentErrorKind::NotFound,
                SessionSubagentRouteErrorKind::NotFound,
            ),
            (
                SubagentErrorKind::Forbidden,
                SessionSubagentRouteErrorKind::Forbidden,
            ),
            (
                SubagentErrorKind::InsufficientStorage,
                SessionSubagentRouteErrorKind::InsufficientStorage,
            ),
            (
                SubagentErrorKind::Internal,
                SessionSubagentRouteErrorKind::Internal,
            ),
        ] {
            let error = subagent_route_error(SubagentError::new_for_test(kind, "message"));
            assert_eq!(error.kind(), expected_kind);
            assert_eq!(error.message(), "message");
        }
    }

    #[test]
    fn scoped_mcp_errors_preserve_messages_and_redaction() {
        let unauthorized = scoped_mcp_session_route_error(
            ScopedMcpSessionAccessError::Unauthorized("scoped message"),
        );
        assert_eq!(
            unauthorized.kind(),
            SessionSubagentRouteErrorKind::Unauthorized
        );
        assert_eq!(unauthorized.message(), "scoped message");

        let missing = scoped_mcp_session_route_error(ScopedMcpSessionAccessError::SessionNotFound);
        assert_eq!(missing.kind(), SessionSubagentRouteErrorKind::NotFound);
        assert_eq!(missing.message(), "session not found");

        let raw_message = "CTX_MCP_TOKEN=secret-token-123";
        let internal = scoped_mcp_session_route_error(
            ScopedMcpSessionAccessError::StoreUnavailable(anyhow!(raw_message)),
        );
        assert_eq!(internal.kind(), SessionSubagentRouteErrorKind::Internal);
        assert_eq!(internal.message(), logs::redact_sensitive(raw_message));
        assert!(!internal.message().contains("secret-token-123"));
    }
}
