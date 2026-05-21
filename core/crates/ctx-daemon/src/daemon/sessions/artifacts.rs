use ctx_core::models::Artifact;
use ctx_observability::logs;
use ctx_route_contracts::sessions::{parse_session_route_id, SessionRouteParams};
use ctx_session_artifacts::route_contract::{
    SessionArtifactDownloadRouteParams, SessionArtifactInput, SessionArtifactRouteError,
    SessionArtifactsRouteResponse, SetSessionArtifactsRouteRequest,
};

use crate::daemon::handle::SessionsHandle;
use crate::daemon::route_files::{open_canonical_route_file, RouteFileDownloadError};
use crate::daemon::{ScopedMcpSessionAccessError, SessionStoreAccessError};
use ctx_core::ids::{ArtifactId, SessionId};

#[derive(Debug)]
pub struct SessionArtifactDownload {
    pub file: tokio::fs::File,
    pub size: u64,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub mime_type: String,
    pub name: Option<String>,
}

impl SessionsHandle {
    pub async fn list_session_artifacts_with_missing_for_route_params(
        &self,
        params: SessionRouteParams,
    ) -> Result<SessionArtifactsRouteResponse, SessionArtifactRouteError> {
        let session_id = parse_session_artifact_route_session_id(params.session_id())?;
        self.list_session_artifacts_with_missing_for_route(session_id)
            .await
            .map(Into::into)
    }

    pub async fn list_session_artifacts_with_missing_for_route(
        &self,
        session_id: SessionId,
    ) -> Result<Vec<Artifact>, SessionArtifactRouteError> {
        let store = self
            .existing_session_store(session_id)
            .await
            .map_err(session_artifact_store_error)?;
        let session = store
            .get_session(session_id)
            .await
            .map_err(session_artifact_internal_error)?
            .ok_or(SessionArtifactRouteError::NotFound)?;
        let session_spool_dir = self.session_tool_output_spool_dir(session.id);
        ctx_session_artifacts::list_session_artifacts_with_missing(
            &store,
            &session,
            &session_spool_dir,
        )
        .await
        .map_err(session_artifact_service_error)
    }

    pub async fn set_session_artifacts_for_route_params(
        &self,
        params: SessionRouteParams,
        mcp_auth: Option<ctx_mcp_auth::McpAuthContext>,
        request: SetSessionArtifactsRouteRequest,
    ) -> Result<SessionArtifactsRouteResponse, SessionArtifactRouteError> {
        let session_id = parse_session_artifact_route_session_id(params.session_id())?;
        if let Some(mcp_auth) = mcp_auth {
            self.require_scoped_mcp_session_context(mcp_auth, session_id)
                .await
                .map_err(scoped_mcp_session_artifact_route_error)?;
        }
        self.set_session_artifacts_for_route(session_id, request.into_artifacts())
            .await
            .map(Into::into)
    }

    pub async fn set_session_artifacts_for_route(
        &self,
        session_id: SessionId,
        inputs: Vec<SessionArtifactInput>,
    ) -> Result<Vec<Artifact>, SessionArtifactRouteError> {
        let store = self
            .existing_session_store_for_write(session_id)
            .await
            .map_err(session_artifact_store_error)?;
        let session = store
            .get_session(session_id)
            .await
            .map_err(session_artifact_internal_error)?
            .ok_or(SessionArtifactRouteError::NotFound)?;
        let service_inputs: Vec<ctx_session_artifacts::SessionArtifactInput> =
            inputs.into_iter().map(Into::into).collect();
        let session_spool_dir = self.session_tool_output_spool_dir(session.id);
        let artifacts = ctx_session_artifacts::build_session_artifacts(
            &store,
            &session,
            &session_spool_dir,
            service_inputs,
        )
        .await
        .map_err(session_artifact_service_error)?;
        self.replace_session_artifacts_and_publish(&session, &artifacts)
            .await
            .map_err(session_artifact_internal_error)?;
        Ok(artifacts)
    }

    pub async fn open_session_artifact_for_route_params(
        &self,
        params: SessionArtifactDownloadRouteParams,
    ) -> Result<SessionArtifactDownload, SessionArtifactRouteError> {
        let session_id = parse_session_artifact_route_session_id(params.session_id())?;
        let artifact_id = parse_session_artifact_route_artifact_id(params.artifact_id())?;
        self.open_session_artifact_for_route(session_id, artifact_id)
            .await
    }

    pub async fn open_session_artifact_for_route(
        &self,
        session_id: SessionId,
        artifact_id: ArtifactId,
    ) -> Result<SessionArtifactDownload, SessionArtifactRouteError> {
        let store = self
            .existing_session_store(session_id)
            .await
            .map_err(session_artifact_store_error)?;
        let session = store
            .get_session(session_id)
            .await
            .map_err(session_artifact_internal_error)?
            .ok_or(SessionArtifactRouteError::NotFound)?;
        let session_spool_dir = self.session_tool_output_spool_dir(session.id);
        let download = ctx_session_artifacts::resolve_session_artifact_download(
            &store,
            &session,
            &session_spool_dir,
            artifact_id,
        )
        .await
        .map_err(session_artifact_service_error)?;
        let file = open_canonical_route_file(&download.canonical_path)
            .await
            .map_err(session_artifact_file_error)?;
        let meta = file
            .metadata()
            .await
            .map_err(|_| SessionArtifactRouteError::NotFound)?;
        if !meta.is_file() {
            return Err(SessionArtifactRouteError::NotFound);
        }
        let size = meta.len();
        let modified = meta.modified().ok();
        let etag = modified.and_then(|modified| {
            ctx_session_artifacts::build_session_artifact_etag(size, modified)
        });
        let last_modified =
            modified.map(ctx_session_artifacts::build_session_artifact_last_modified);
        Ok(SessionArtifactDownload {
            file,
            size,
            etag,
            last_modified,
            mime_type: download.mime_type,
            name: download.name,
        })
    }
}

fn parse_session_artifact_route_session_id(
    value: &str,
) -> Result<SessionId, SessionArtifactRouteError> {
    parse_session_route_id(value)
        .map_err(|_| SessionArtifactRouteError::BadRequest("invalid session id".to_string()))
}

fn parse_session_artifact_route_artifact_id(
    value: &str,
) -> Result<ArtifactId, SessionArtifactRouteError> {
    uuid::Uuid::parse_str(value)
        .map(ArtifactId)
        .map_err(|_| SessionArtifactRouteError::BadRequest("invalid artifact id".to_string()))
}

fn scoped_mcp_session_artifact_route_error(
    error: ScopedMcpSessionAccessError,
) -> SessionArtifactRouteError {
    match error {
        ScopedMcpSessionAccessError::Unauthorized(message) => {
            SessionArtifactRouteError::Unauthorized(message.to_string())
        }
        ScopedMcpSessionAccessError::SessionNotFound => SessionArtifactRouteError::NotFound,
        ScopedMcpSessionAccessError::StoreUnavailable(error) => {
            session_artifact_internal_error(error)
        }
    }
}

fn session_artifact_store_error(error: SessionStoreAccessError) -> SessionArtifactRouteError {
    match error {
        SessionStoreAccessError::NotFound => SessionArtifactRouteError::NotFound,
        SessionStoreAccessError::LookupUnavailable(error) => session_artifact_internal_error(error),
        SessionStoreAccessError::StoreUnavailable => {
            SessionArtifactRouteError::Internal("workspace store unavailable".to_string())
        }
    }
}

fn session_artifact_service_error(
    error: ctx_session_artifacts::SessionArtifactError,
) -> SessionArtifactRouteError {
    match error {
        ctx_session_artifacts::SessionArtifactError::NotFound => {
            SessionArtifactRouteError::NotFound
        }
        ctx_session_artifacts::SessionArtifactError::BadRequest(message) => {
            SessionArtifactRouteError::BadRequest(logs::redact_sensitive(&message))
        }
        ctx_session_artifacts::SessionArtifactError::Internal(message) => {
            SessionArtifactRouteError::Internal(logs::redact_sensitive(&message))
        }
    }
}

fn session_artifact_internal_error(error: impl std::fmt::Display) -> SessionArtifactRouteError {
    SessionArtifactRouteError::Internal(logs::redact_sensitive(&error.to_string()))
}

fn session_artifact_file_error(error: RouteFileDownloadError) -> SessionArtifactRouteError {
    match error {
        RouteFileDownloadError::NotFound => SessionArtifactRouteError::NotFound,
        RouteFileDownloadError::Internal => {
            SessionArtifactRouteError::Internal("failed to open session artifact".to_string())
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn open_canonical_route_file_rejects_symlink_swap() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let outside_path = outside.path().join("outside.txt");
        std::fs::write(&outside_path, b"outside\n").unwrap();

        let artifact_path = root.path().join("artifact.txt");
        std::fs::write(&artifact_path, b"inside\n").unwrap();
        let canonical = tokio::fs::canonicalize(&artifact_path).await.unwrap();

        let renamed = root.path().join("artifact.saved");
        std::fs::rename(&artifact_path, &renamed).unwrap();
        std::os::unix::fs::symlink(&outside_path, &artifact_path).unwrap();

        let err = open_canonical_route_file(&canonical).await.unwrap_err();
        assert_eq!(err, RouteFileDownloadError::NotFound);
    }

    #[test]
    fn route_params_reject_invalid_session_and_artifact_ids() {
        let error = parse_session_artifact_route_session_id("not-a-session").unwrap_err();
        assert_eq!(
            error,
            SessionArtifactRouteError::BadRequest("invalid session id".to_string())
        );

        let error = parse_session_artifact_route_artifact_id("not-an-artifact").unwrap_err();
        assert_eq!(
            error,
            SessionArtifactRouteError::BadRequest("invalid artifact id".to_string())
        );
    }

    #[test]
    fn scoped_mcp_artifact_route_errors_preserve_public_mapping() {
        let unauthorized = scoped_mcp_session_artifact_route_error(
            ScopedMcpSessionAccessError::Unauthorized("scoped message"),
        );
        assert_eq!(
            unauthorized,
            SessionArtifactRouteError::Unauthorized("scoped message".to_string())
        );

        let missing =
            scoped_mcp_session_artifact_route_error(ScopedMcpSessionAccessError::SessionNotFound);
        assert_eq!(missing, SessionArtifactRouteError::NotFound);
    }
}
