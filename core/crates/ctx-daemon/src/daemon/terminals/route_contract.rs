pub use ctx_route_contracts::terminals::{
    CreateTerminalRouteRequest, CreateTerminalRouteSpec, DeleteTerminalRouteParams,
    ListWorkspaceTerminalsRouteParams, MintTerminalStreamTokenRouteParams, TerminalRouteError,
    TerminalRouteErrorKind, TerminalSessionRouteResponse, TerminalStatusRouteResponse,
    TerminalStreamConnectRouteResponse, TerminalStreamRouteParams,
};
use ctx_transport_runtime::terminal_launch::{TerminalLaunchError, TerminalLaunchErrorKind};
use ctx_transport_runtime::terminals::{TerminalStreamAccessError, TerminalStreamSession};

use crate::daemon::TransportHandle;

use super::launch::CreateTerminalLaunchRequest;

pub struct TerminalStreamRouteAdmission {
    pub session: TerminalStreamSession,
    pub tail_bytes: usize,
}

fn create_terminal_launch_request(spec: CreateTerminalRouteSpec) -> CreateTerminalLaunchRequest {
    CreateTerminalLaunchRequest {
        workspace_id: spec.workspace_id,
        task_id: spec.task_id,
        session_id: spec.session_id,
        worktree_id: spec.worktree_id,
        cwd: spec.cwd,
        shell: spec.shell,
    }
}

fn terminal_launch_route_error(error: TerminalLaunchError) -> TerminalRouteError {
    match error.kind() {
        TerminalLaunchErrorKind::BadRequest => TerminalRouteError::bad_request(error.message()),
        TerminalLaunchErrorKind::NotFound => TerminalRouteError::not_found(error.message()),
        TerminalLaunchErrorKind::Internal => TerminalRouteError::internal(error.message()),
    }
}

impl TransportHandle {
    pub async fn list_workspace_terminal_responses_for_route(
        &self,
        params: ListWorkspaceTerminalsRouteParams,
    ) -> Result<Vec<TerminalSessionRouteResponse>, TerminalRouteError> {
        let workspace_id = params.parse_workspace_id()?;
        let sessions = super::list_workspace_terminals(&self.state, workspace_id).await;
        Ok(sessions.into_iter().map(Into::into).collect())
    }

    pub async fn create_workspace_terminal_for_route(
        &self,
        raw_workspace_id: &str,
        req: CreateTerminalRouteRequest,
    ) -> Result<TerminalSessionRouteResponse, TerminalRouteError> {
        let launch_req = create_terminal_launch_request(req.parse(raw_workspace_id)?);
        super::create_workspace_terminal(&self.state, launch_req)
            .await
            .map(Into::into)
            .map_err(terminal_launch_route_error)
    }

    pub async fn delete_terminal_for_route(
        &self,
        params: DeleteTerminalRouteParams,
    ) -> Result<(), TerminalRouteError> {
        let terminal_id = params.parse_terminal_id()?;
        if super::delete_terminal(&self.state, terminal_id).await {
            return Ok(());
        }
        Err(TerminalRouteError::not_found("terminal not found"))
    }

    pub async fn mint_terminal_stream_token_for_route(
        &self,
        params: MintTerminalStreamTokenRouteParams,
    ) -> Result<TerminalStreamConnectRouteResponse, TerminalRouteError> {
        let terminal_id = params.parse_terminal_id()?;
        let token = super::mint_terminal_stream_token(&self.state, terminal_id)
            .await
            .ok_or_else(|| TerminalRouteError::not_found("terminal not found"))?;
        Ok(TerminalStreamConnectRouteResponse {
            stream_path: token.stream_path,
            expires_at: token.expires_at,
        })
    }

    pub async fn admit_terminal_stream_for_route(
        &self,
        params: TerminalStreamRouteParams,
    ) -> Result<TerminalStreamRouteAdmission, TerminalRouteError> {
        let terminal_id = params.parse_terminal_id()?;
        let tail_bytes = params.tail_bytes();
        let token = params
            .token()
            .ok_or_else(terminal_stream_missing_token_route_error)?;
        let session = self
            .state
            .transport
            .terminals
            .require_stream_access(terminal_id, token)
            .await
            .map_err(terminal_stream_access_route_error)?;

        Ok(TerminalStreamRouteAdmission {
            session,
            tail_bytes,
        })
    }
}

fn terminal_stream_missing_token_route_error() -> TerminalRouteError {
    TerminalRouteError::unauthorized("terminal stream token required")
}

fn terminal_stream_access_route_error(error: TerminalStreamAccessError) -> TerminalRouteError {
    match error {
        TerminalStreamAccessError::Unauthorized => {
            TerminalRouteError::unauthorized("terminal stream token required")
        }
        TerminalStreamAccessError::NotFound => TerminalRouteError::not_found("terminal not found"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctx_core::ids::TerminalId;

    #[test]
    fn terminal_stream_access_errors_map_to_route_errors() {
        let route_error = terminal_stream_missing_token_route_error();
        assert_eq!(route_error.kind(), TerminalRouteErrorKind::Unauthorized);
        assert_eq!(route_error.message(), "terminal stream token required");

        let route_error =
            terminal_stream_access_route_error(TerminalStreamAccessError::Unauthorized);
        assert_eq!(route_error.kind(), TerminalRouteErrorKind::Unauthorized);
        assert_eq!(route_error.message(), "terminal stream token required");

        let route_error = terminal_stream_access_route_error(TerminalStreamAccessError::NotFound);
        assert_eq!(route_error.kind(), TerminalRouteErrorKind::NotFound);
        assert_eq!(route_error.message(), "terminal not found");
    }

    #[tokio::test]
    async fn terminal_stream_route_checks_missing_token_before_terminal_lookup() {
        let temp = tempfile::tempdir().expect("tempdir");
        let daemon = crate::test_support::TestDaemon::new_for_test(
            temp.path().to_path_buf(),
            "http://127.0.0.1:4567".to_string(),
        )
        .await
        .expect("test daemon");
        let missing_terminal_id = TerminalId::new();

        let result = daemon
            .handle()
            .transport()
            .admit_terminal_stream_for_route(TerminalStreamRouteParams::new(
                missing_terminal_id.0.to_string(),
                None,
                None,
            ))
            .await;
        let error = match result {
            Ok(_) => panic!("missing token should reject before terminal lookup"),
            Err(error) => error,
        };

        assert_eq!(error.kind(), TerminalRouteErrorKind::Unauthorized);
        assert_eq!(error.message(), "terminal stream token required");
    }
}
