use ctx_core::ids::SessionId;
use ctx_mcp_auth::McpAuthContext;
use ctx_observability::logs;
use ctx_subagent_service::route_contract::{
    ArchiveAgentRouteRequest, ArchiveAgentRouteResponse, GetAgentRouteRequest,
    GetAgentRouteResponse, InterruptAgentRouteRequest, InterruptAgentRouteResponse,
    ListAgentsRouteResponse, SendInputRouteRequest, SendInputRouteResponse,
    SessionSubagentInvocationRouteResponse, SessionSubagentInvocationsRouteQuery,
    SessionSubagentInvocationsRouteResponse, SessionSubagentRouteError,
    SessionSubagentsRouteResponse, SpawnAgentRouteRequest, SpawnAgentRouteResponse,
    WaitAgentRouteRequest, WaitAgentRouteResponse,
};

use crate::daemon::sessions::route_contract::parse_session_route_id;
use crate::daemon::sessions::subagents::{SubagentError, SubagentErrorKind};
use crate::daemon::{ScopedMcpSessionAccessError, SessionRouteParams, SessionsHandle};

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
        Ok(SessionSubagentsRouteResponse::new(subagents))
    }

    pub async fn list_session_subagent_invocations_for_route(
        &self,
        params: SessionRouteParams,
        query: SessionSubagentInvocationsRouteQuery,
    ) -> Result<SessionSubagentInvocationsRouteResponse, SessionSubagentRouteError> {
        let session_id = parse_subagent_route_id(params)?;
        let turn_id = query.into_turn_id()?;
        let invocations = self
            .list_session_subagent_invocations_for_request(session_id, turn_id)
            .await
            .map_err(|_| SessionSubagentRouteError::internal("internal server error"))?
            .ok_or_else(|| SessionSubagentRouteError::not_found("session not found"))?;
        Ok(SessionSubagentInvocationsRouteResponse::new(invocations))
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
        Ok(SessionSubagentInvocationRouteResponse::new(invocation))
    }

    pub async fn spawn_agent_for_mcp_route(
        &self,
        params: SessionRouteParams,
        mcp_auth: Option<McpAuthContext>,
        request: SpawnAgentRouteRequest,
    ) -> Result<SpawnAgentRouteResponse, SessionSubagentRouteError> {
        let parent_id = self
            .resolve_mcp_subagent_parent_session_id(params, mcp_auth)
            .await?;
        self.spawn_agent(parent_id, request.into_low_level())
            .await
            .map(SpawnAgentRouteResponse::new)
            .map_err(subagent_route_error)
    }

    pub async fn send_input_for_mcp_route(
        &self,
        params: SessionRouteParams,
        mcp_auth: Option<McpAuthContext>,
        request: SendInputRouteRequest,
    ) -> Result<SendInputRouteResponse, SessionSubagentRouteError> {
        let parent_id = self
            .resolve_mcp_subagent_parent_session_id(params, mcp_auth)
            .await?;
        self.send_input(parent_id, request.into_low_level())
            .await
            .map(SendInputRouteResponse::new)
            .map_err(subagent_route_error)
    }

    pub async fn archive_agent_for_mcp_route(
        &self,
        params: SessionRouteParams,
        mcp_auth: Option<McpAuthContext>,
        request: ArchiveAgentRouteRequest,
    ) -> Result<ArchiveAgentRouteResponse, SessionSubagentRouteError> {
        let parent_id = self
            .resolve_mcp_subagent_parent_session_id(params, mcp_auth)
            .await?;
        self.archive_agent(parent_id, request.into_low_level())
            .await
            .map(ArchiveAgentRouteResponse::new)
            .map_err(subagent_route_error)
    }

    pub async fn list_agents_for_mcp_route(
        &self,
        params: SessionRouteParams,
        mcp_auth: Option<McpAuthContext>,
    ) -> Result<ListAgentsRouteResponse, SessionSubagentRouteError> {
        let parent_id = self
            .resolve_mcp_subagent_parent_session_id(params, mcp_auth)
            .await?;
        self.list_agents(parent_id)
            .await
            .map(ListAgentsRouteResponse::new)
            .map_err(subagent_route_error)
    }

    pub async fn get_agent_for_mcp_route(
        &self,
        params: SessionRouteParams,
        mcp_auth: Option<McpAuthContext>,
        request: GetAgentRouteRequest,
    ) -> Result<GetAgentRouteResponse, SessionSubagentRouteError> {
        let parent_id = self
            .resolve_mcp_subagent_parent_session_id(params, mcp_auth)
            .await?;
        self.get_agent(parent_id, request.into_low_level())
            .await
            .map(GetAgentRouteResponse::new)
            .map_err(subagent_route_error)
    }

    pub async fn interrupt_agent_for_mcp_route(
        &self,
        params: SessionRouteParams,
        mcp_auth: Option<McpAuthContext>,
        request: InterruptAgentRouteRequest,
    ) -> Result<InterruptAgentRouteResponse, SessionSubagentRouteError> {
        let parent_id = self
            .resolve_mcp_subagent_parent_session_id(params, mcp_auth)
            .await?;
        self.interrupt_agent(parent_id, request.into_low_level())
            .await
            .map(InterruptAgentRouteResponse::new)
            .map_err(subagent_route_error)
    }

    pub async fn wait_agent_for_mcp_route(
        &self,
        params: SessionRouteParams,
        mcp_auth: Option<McpAuthContext>,
        request: WaitAgentRouteRequest,
    ) -> Result<WaitAgentRouteResponse, SessionSubagentRouteError> {
        let parent_id = self
            .resolve_mcp_subagent_parent_session_id(params, mcp_auth)
            .await?;
        self.wait_agent(parent_id, request.into_low_level())
            .await
            .map(WaitAgentRouteResponse::new)
            .map_err(subagent_route_error)
    }

    async fn resolve_mcp_subagent_parent_session_id(
        &self,
        params: SessionRouteParams,
        mcp_auth: Option<McpAuthContext>,
    ) -> Result<SessionId, SessionSubagentRouteError> {
        let session_id = parse_subagent_route_id(params)?;
        if let Some(mcp_auth) = mcp_auth {
            self.require_scoped_mcp_session_context(mcp_auth, session_id)
                .await
                .map_err(scoped_mcp_session_route_error)?;
        }
        Ok(session_id)
    }
}

fn parse_subagent_route_id(
    params: SessionRouteParams,
) -> Result<SessionId, SessionSubagentRouteError> {
    parse_session_route_id(params.session_id())
        .map_err(|_| SessionSubagentRouteError::bad_request("invalid session id"))
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
    use ctx_subagent_service::route_contract::SessionSubagentRouteErrorKind;

    #[test]
    fn invalid_session_id_uses_existing_route_message() {
        let error = parse_subagent_route_id(SessionRouteParams::new("not-a-session")).unwrap_err();
        assert_eq!(error.kind(), SessionSubagentRouteErrorKind::BadRequest);
        assert_eq!(error.message(), "invalid session id");
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
