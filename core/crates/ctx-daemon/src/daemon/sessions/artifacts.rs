use std::path::{Path, PathBuf};

use chrono::Utc;
use ctx_core::ids::{ArtifactId, SessionId};
use ctx_core::models::{Artifact, Session};
use ctx_observability::logs;
use ctx_session_tools::{
    build_session_artifact_etag, build_session_artifact_last_modified,
    infer_session_artifact_mime_type, normalize_session_artifact_name,
};
use ctx_store::Store;

use crate::daemon::handle::SessionsHandle;
use crate::daemon::route_files::{
    canonicalize_existing_or_raw, open_canonical_route_file, RouteFileDownloadError,
};
use crate::daemon::SessionStoreAccessError;

#[derive(Debug)]
pub struct SessionArtifactInput {
    pub absolute_file_path: String,
    pub name: Option<String>,
    pub mime_type: Option<String>,
}

#[derive(Debug)]
pub struct SessionArtifactDownload {
    pub file: tokio::fs::File,
    pub size: u64,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub mime_type: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum SessionArtifactRouteError {
    NotFound,
    BadRequest(String),
    Internal(String),
}

impl SessionsHandle {
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
        let mut artifacts = store
            .list_session_artifacts(session.id)
            .await
            .map_err(session_artifact_internal_error)?;
        for artifact in artifacts.iter_mut() {
            if !self
                .session_artifact_path_is_accessible(
                    &store,
                    &session,
                    Path::new(&artifact.absolute_path),
                )
                .await
                .map_err(session_artifact_internal_error)?
            {
                artifact.missing = Some(true);
            }
        }
        Ok(artifacts)
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
        let artifacts = build_session_artifacts(self, &store, &session, inputs).await?;
        self.replace_session_artifacts_and_publish(&session, &artifacts)
            .await
            .map_err(session_artifact_internal_error)?;
        Ok(artifacts)
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
        let artifact = store
            .get_artifact(artifact_id)
            .await
            .map_err(session_artifact_internal_error)?
            .filter(|artifact| artifact.session_id == session.id)
            .ok_or(SessionArtifactRouteError::NotFound)?;
        let path = PathBuf::from(&artifact.absolute_path);
        let canonical_path =
            resolve_session_artifact_accessible_path(self, &store, &session, &path)
                .await?
                .ok_or(SessionArtifactRouteError::NotFound)?;
        let file = open_canonical_route_file(&canonical_path)
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
        let etag = modified.and_then(|modified| build_session_artifact_etag(size, modified));
        let last_modified = modified.map(build_session_artifact_last_modified);
        Ok(SessionArtifactDownload {
            file,
            size,
            etag,
            last_modified,
            mime_type: artifact.mime_type,
            name: artifact.name,
        })
    }
}

async fn build_session_artifacts(
    handle: &SessionsHandle,
    store: &Store,
    session: &Session,
    inputs: Vec<SessionArtifactInput>,
) -> Result<Vec<Artifact>, SessionArtifactRouteError> {
    let mut artifacts = Vec::with_capacity(inputs.len());
    for (idx, artifact) in inputs.into_iter().enumerate() {
        let raw = artifact.absolute_file_path.trim();
        if raw.is_empty() {
            return Err(SessionArtifactRouteError::BadRequest(format!(
                "artifact {} missing absolute_file_path",
                idx + 1
            )));
        }
        let path = PathBuf::from(raw);
        if !path.is_absolute() {
            return Err(SessionArtifactRouteError::BadRequest(format!(
                "artifact {} absolute_file_path must be absolute",
                idx + 1
            )));
        }
        let meta = tokio::fs::metadata(&path).await.map_err(|e| {
            SessionArtifactRouteError::BadRequest(format!(
                "artifact {} path not accessible: {}",
                idx + 1,
                logs::redact_sensitive(&e.to_string())
            ))
        })?;
        if !meta.is_file() {
            return Err(SessionArtifactRouteError::BadRequest(format!(
                "artifact {} path is not a file",
                idx + 1
            )));
        }
        let path = validate_session_artifact_write_path(handle, store, session, &path)
            .await
            .map_err(|error| {
                SessionArtifactRouteError::BadRequest(format!("artifact {} {error}", idx + 1))
            })?;

        let name = normalize_session_artifact_name(artifact.name, &path);
        let mime_type = infer_session_artifact_mime_type(&path, artifact.mime_type);
        let bytes = meta.len() as i64;
        let created_at = Utc::now();

        artifacts.push(Artifact {
            id: ArtifactId::new(),
            session_id: session.id,
            task_id: session.task_id,
            workspace_id: session.workspace_id,
            worktree_id: session.worktree_id,
            name,
            absolute_path: path.to_string_lossy().to_string(),
            mime_type,
            bytes,
            created_at,
            missing: None,
        });
    }
    Ok(artifacts)
}

async fn resolve_session_artifact_accessible_path(
    handle: &SessionsHandle,
    store: &Store,
    session: &Session,
    path: &Path,
) -> Result<Option<PathBuf>, SessionArtifactRouteError> {
    let roots = session_artifact_allowed_roots(handle, store, session)
        .await
        .map_err(session_artifact_internal_error)?;
    let canonical = match tokio::fs::canonicalize(path).await {
        Ok(canonical) => canonical,
        Err(_) => return Ok(None),
    };
    Ok(roots
        .iter()
        .any(|root| canonical.starts_with(root))
        .then_some(canonical))
}

async fn validate_session_artifact_write_path(
    handle: &SessionsHandle,
    store: &Store,
    session: &Session,
    path: &Path,
) -> Result<PathBuf, String> {
    let roots = session_artifact_allowed_roots(handle, store, session)
        .await
        .map_err(|error| {
            format!(
                "failed to resolve session artifact roots: {}",
                logs::redact_sensitive(&error.to_string())
            )
        })?;
    let canonical = tokio::fs::canonicalize(path)
        .await
        .map_err(|e| logs::redact_sensitive(&e.to_string()))?;
    if roots.iter().any(|root| canonical.starts_with(root)) {
        return Ok(canonical);
    }
    Err("absolute_file_path must stay inside the session worktree or tool-output spool".into())
}

async fn session_artifact_allowed_roots(
    handle: &SessionsHandle,
    store: &Store,
    session: &Session,
) -> anyhow::Result<Vec<PathBuf>> {
    let mut roots = Vec::with_capacity(2);
    if let Some(worktree) = store.get_worktree(session.worktree_id).await? {
        roots.push(canonicalize_existing_or_raw(&PathBuf::from(worktree.root_path)).await);
    }
    roots.push(
        canonicalize_existing_or_raw(
            &handle
                .state
                .core
                .tool_output_spool_dir
                .join(session.id.0.to_string()),
        )
        .await,
    );
    Ok(roots)
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
}
