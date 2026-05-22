use ctx_transport_runtime::web_sessions::{WebSessionInfo, WebSessionRunResponse};

use crate::daemon::TransportHandle;

use super::{
    create_web_session, eval_web_session, get_web_session, list_web_sessions, run_web_session,
    WebSessionActionError, WebSessionLaunchError, WebSessionLaunchErrorKind,
    WebSessionLaunchRequest,
};

use ctx_route_contracts::web_sessions::{
    WebSessionActionRouteRequest, WebSessionCreateRouteRequest, WebSessionCreateRouteSpec,
    WebSessionListRouteQuery, WebSessionRouteError,
};

fn web_session_launch_request(spec: WebSessionCreateRouteSpec) -> WebSessionLaunchRequest {
    WebSessionLaunchRequest {
        session_id: spec.session_id,
        worktree_id: spec.worktree_id,
        url: spec.url,
        viewport: spec.viewport,
        fps: spec.fps,
    }
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
        let request = web_session_launch_request(request.validate()?);
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
    use ctx_route_contracts::web_sessions::WebSessionRouteErrorKind;

    #[test]
    fn action_errors_map_to_route_status_classes() {
        let not_found = web_session_action_route_error(WebSessionActionError::NotFound);
        assert_eq!(not_found.kind(), WebSessionRouteErrorKind::NotFound);

        let internal = web_session_action_route_error(WebSessionActionError::Internal);
        assert_eq!(internal.kind(), WebSessionRouteErrorKind::Internal);
    }
}
