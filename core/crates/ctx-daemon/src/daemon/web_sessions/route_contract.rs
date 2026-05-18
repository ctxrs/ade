use ctx_core::ids::{SessionId, WorktreeId};
use ctx_transport_runtime::web_sessions::{
    WebSessionInfo, WebSessionRunRequest, WebSessionRunResponse, WebSessionViewport,
};
use serde::Deserialize;

use crate::daemon::TransportHandle;

use super::{
    create_web_session, eval_web_session, get_web_session, list_web_sessions, run_web_session,
    WebSessionActionError, WebSessionLaunchError, WebSessionLaunchErrorKind,
    WebSessionLaunchRequest,
};

const DEFAULT_WEB_SESSION_ACTION_TIMEOUT_MS: u64 = 5 * 60 * 1000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WebSessionRouteErrorKind {
    BadRequest,
    Forbidden,
    NotFound,
    Internal,
}

#[derive(Debug, Eq, PartialEq)]
pub struct WebSessionRouteError {
    kind: WebSessionRouteErrorKind,
    message: String,
}

impl WebSessionRouteError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            kind: WebSessionRouteErrorKind::BadRequest,
            message: message.into(),
        }
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self {
            kind: WebSessionRouteErrorKind::Forbidden,
            message: message.into(),
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            kind: WebSessionRouteErrorKind::NotFound,
            message: message.into(),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            kind: WebSessionRouteErrorKind::Internal,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> WebSessionRouteErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

#[derive(Debug, Deserialize)]
pub struct WebSessionCreateRouteRequest {
    pub session_id: Option<String>,
    pub worktree_id: Option<String>,
    pub url: String,
    pub viewport: Option<WebSessionViewport>,
    pub fps: Option<u32>,
}

impl WebSessionCreateRouteRequest {
    fn into_launch_request(self) -> Result<WebSessionLaunchRequest, WebSessionRouteError> {
        if self.url.trim().is_empty() {
            return Err(WebSessionRouteError::bad_request("url is required"));
        }

        let session_id = parse_optional_session_id(self.session_id.as_deref())?;
        let worktree_id = parse_optional_worktree_id(self.worktree_id.as_deref())?;

        Ok(WebSessionLaunchRequest {
            session_id,
            worktree_id,
            url: self.url,
            viewport: self.viewport,
            fps: self.fps,
        })
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct WebSessionListRouteQuery {
    pub session_id: Option<String>,
}

impl WebSessionListRouteQuery {
    fn validated_session_id(&self) -> Result<Option<&str>, WebSessionRouteError> {
        let Some(session_id) = self.session_id.as_deref() else {
            return Ok(None);
        };
        uuid::Uuid::parse_str(session_id)
            .map_err(|_| WebSessionRouteError::bad_request("invalid session id"))?;
        Ok(Some(session_id))
    }
}

#[derive(Debug, Deserialize)]
pub struct WebSessionActionRouteRequest {
    pub code: Option<String>,
    pub script_path: Option<String>,
    pub timeout_ms: Option<u64>,
}

impl WebSessionActionRouteRequest {
    fn into_run_request(self) -> WebSessionRunRequest {
        WebSessionRunRequest {
            code: self.code,
            script_path: self.script_path,
            timeout_ms: Some(
                self.timeout_ms
                    .unwrap_or(DEFAULT_WEB_SESSION_ACTION_TIMEOUT_MS),
            ),
        }
    }
}

fn parse_optional_session_id(raw: Option<&str>) -> Result<Option<SessionId>, WebSessionRouteError> {
    raw.map(uuid::Uuid::parse_str)
        .transpose()
        .map_err(|_| WebSessionRouteError::bad_request("invalid session id"))
        .map(|id| id.map(SessionId))
}

fn parse_optional_worktree_id(
    raw: Option<&str>,
) -> Result<Option<WorktreeId>, WebSessionRouteError> {
    raw.map(uuid::Uuid::parse_str)
        .transpose()
        .map_err(|_| WebSessionRouteError::bad_request("invalid worktree id"))
        .map(|id| id.map(WorktreeId))
}

fn web_session_launch_route_error(error: WebSessionLaunchError) -> WebSessionRouteError {
    match error.kind() {
        WebSessionLaunchErrorKind::BadRequest => WebSessionRouteError::bad_request(error.message()),
        WebSessionLaunchErrorKind::Forbidden => WebSessionRouteError::forbidden(error.message()),
        WebSessionLaunchErrorKind::Internal => WebSessionRouteError::internal(error.message()),
    }
}

fn web_session_action_route_error(error: WebSessionActionError) -> WebSessionRouteError {
    match error {
        WebSessionActionError::NotFound => WebSessionRouteError::not_found("web session not found"),
        WebSessionActionError::Internal => {
            WebSessionRouteError::internal("web session action failed")
        }
    }
}

impl TransportHandle {
    pub async fn create_web_session_for_route(
        &self,
        request: WebSessionCreateRouteRequest,
    ) -> Result<WebSessionInfo, WebSessionRouteError> {
        let request = request.into_launch_request()?;
        create_web_session(&self.state, request)
            .await
            .map_err(web_session_launch_route_error)
    }

    pub async fn list_web_sessions_for_route(
        &self,
        query: WebSessionListRouteQuery,
    ) -> Result<Vec<WebSessionInfo>, WebSessionRouteError> {
        let session_id = query.validated_session_id()?;
        let mut sessions = list_web_sessions(&self.state).await;
        if let Some(session_id) = session_id {
            sessions.retain(|session| session.session_id.as_deref() == Some(session_id));
        }
        Ok(sessions)
    }

    pub async fn get_web_session_for_route(
        &self,
        id: &str,
    ) -> Result<WebSessionInfo, WebSessionRouteError> {
        get_web_session(&self.state, id)
            .await
            .ok_or_else(|| WebSessionRouteError::not_found("web session not found"))
    }

    pub async fn run_web_session_for_route(
        &self,
        id: &str,
        request: WebSessionActionRouteRequest,
    ) -> Result<WebSessionRunResponse, WebSessionRouteError> {
        run_web_session(&self.state, id, request.into_run_request())
            .await
            .map_err(web_session_action_route_error)
    }

    pub async fn eval_web_session_for_route(
        &self,
        id: &str,
        request: WebSessionActionRouteRequest,
    ) -> Result<WebSessionRunResponse, WebSessionRouteError> {
        eval_web_session(&self.state, id, request.into_run_request())
            .await
            .map_err(web_session_action_route_error)
    }

    pub async fn close_web_session_for_route(&self, id: &str) -> Result<(), WebSessionRouteError> {
        super::close_web_session(&self.state, id)
            .await
            .map_err(web_session_action_route_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_route_error(request: WebSessionCreateRouteRequest) -> WebSessionRouteError {
        match request.into_launch_request() {
            Ok(_) => panic!("expected route request to fail"),
            Err(error) => error,
        }
    }

    #[test]
    fn create_route_request_preserves_empty_url_precedence() {
        let request = WebSessionCreateRouteRequest {
            session_id: Some("not-a-uuid".to_string()),
            worktree_id: Some("also-not-a-uuid".to_string()),
            url: " ".to_string(),
            viewport: None,
            fps: None,
        };

        let error = create_route_error(request);
        assert_eq!(error.kind(), WebSessionRouteErrorKind::BadRequest);
        assert_eq!(error.message(), "url is required");
    }

    #[test]
    fn create_route_request_rejects_invalid_session_id() {
        let request = WebSessionCreateRouteRequest {
            session_id: Some("not-a-uuid".to_string()),
            worktree_id: None,
            url: "https://example.test".to_string(),
            viewport: None,
            fps: None,
        };

        let error = create_route_error(request);
        assert_eq!(error.kind(), WebSessionRouteErrorKind::BadRequest);
        assert_eq!(error.message(), "invalid session id");
    }

    #[test]
    fn create_route_request_rejects_invalid_worktree_id() {
        let request = WebSessionCreateRouteRequest {
            session_id: None,
            worktree_id: Some("not-a-uuid".to_string()),
            url: "https://example.test".to_string(),
            viewport: None,
            fps: None,
        };

        let error = create_route_error(request);
        assert_eq!(error.kind(), WebSessionRouteErrorKind::BadRequest);
        assert_eq!(error.message(), "invalid worktree id");
    }

    #[test]
    fn list_route_query_validates_but_preserves_raw_session_filter() {
        let raw = uuid::Uuid::new_v4().to_string();
        let query = WebSessionListRouteQuery {
            session_id: Some(raw.clone()),
        };

        assert_eq!(query.validated_session_id().unwrap(), Some(raw.as_str()));

        let error = WebSessionListRouteQuery {
            session_id: Some("not-a-uuid".to_string()),
        }
        .validated_session_id()
        .unwrap_err();
        assert_eq!(error.kind(), WebSessionRouteErrorKind::BadRequest);
        assert_eq!(error.message(), "invalid session id");
    }

    #[test]
    fn action_route_request_defaults_timeout_only_when_missing() {
        let defaulted = WebSessionActionRouteRequest {
            code: Some("1 + 1".to_string()),
            script_path: None,
            timeout_ms: None,
        }
        .into_run_request();
        assert_eq!(
            defaulted.timeout_ms,
            Some(DEFAULT_WEB_SESSION_ACTION_TIMEOUT_MS)
        );

        let explicit = WebSessionActionRouteRequest {
            code: None,
            script_path: Some("script.js".to_string()),
            timeout_ms: Some(42),
        }
        .into_run_request();
        assert_eq!(explicit.timeout_ms, Some(42));
    }

    #[test]
    fn action_errors_map_to_route_status_classes() {
        let not_found = web_session_action_route_error(WebSessionActionError::NotFound);
        assert_eq!(not_found.kind(), WebSessionRouteErrorKind::NotFound);

        let internal = web_session_action_route_error(WebSessionActionError::Internal);
        assert_eq!(internal.kind(), WebSessionRouteErrorKind::Internal);
    }
}
