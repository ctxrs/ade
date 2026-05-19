use std::path::{Path, PathBuf};

use anyhow::Result;
use ctx_core::ids::{ArtifactId, SessionId};
use ctx_core::models::{Artifact, Session, SessionEventType, Worktree};
use ctx_store::Store;

use crate::daemon::handle::SessionsHandle;
use crate::daemon::route_files::canonicalize_existing_or_raw;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum SessionImageBlobStoreError {
    PayloadTooLarge,
    UnsupportedMediaType,
    Internal,
}

impl From<crate::daemon::blobs::ImageBlobStoreError> for SessionImageBlobStoreError {
    fn from(error: crate::daemon::blobs::ImageBlobStoreError) -> Self {
        match error {
            crate::daemon::blobs::ImageBlobStoreError::PayloadTooLarge => Self::PayloadTooLarge,
            crate::daemon::blobs::ImageBlobStoreError::UnsupportedMediaType => {
                Self::UnsupportedMediaType
            }
            crate::daemon::blobs::ImageBlobStoreError::Internal => Self::Internal,
        }
    }
}

impl SessionsHandle {
    pub async fn get_blob(
        &self,
        id: &str,
    ) -> Result<
        Option<(
            String,
            String,
            i64,
            Option<String>,
            chrono::DateTime<chrono::Utc>,
        )>,
    > {
        self.state.global_store().get_blob(id).await
    }

    pub async fn get_session_for_artifacts(
        &self,
        session_id: SessionId,
    ) -> Result<Option<Session>> {
        let Some(store) = self.session_store_or_none(session_id).await? else {
            return Ok(None);
        };
        store.get_session(session_id).await
    }

    pub async fn get_session_worktree(&self, session: &Session) -> Result<Option<Worktree>> {
        let store = self.store_for_session(session.id).await?;
        store.get_worktree(session.worktree_id).await
    }

    pub async fn get_session_artifact_for_download(
        &self,
        session_id: SessionId,
        artifact_id: ArtifactId,
    ) -> Result<Option<(Session, Artifact)>> {
        let Some(store) = self.session_store_or_none(session_id).await? else {
            return Ok(None);
        };
        let Some(session) = store.get_session(session_id).await? else {
            return Ok(None);
        };
        let Some(artifact) = store.get_artifact(artifact_id).await? else {
            return Ok(None);
        };
        if artifact.session_id != session.id {
            return Ok(None);
        }
        Ok(Some((session, artifact)))
    }

    pub async fn list_session_artifacts_for_route(
        &self,
        session_id: SessionId,
    ) -> Result<Option<(Session, Vec<Artifact>)>> {
        let Some(store) = self.session_store_or_none(session_id).await? else {
            return Ok(None);
        };
        let Some(session) = store.get_session(session_id).await? else {
            return Ok(None);
        };
        let artifacts = store.list_session_artifacts(session.id).await?;
        Ok(Some((session, artifacts)))
    }

    pub async fn replace_session_artifacts_and_publish(
        &self,
        session: &Session,
        artifacts: &[Artifact],
    ) -> Result<()> {
        let store = self.store_for_session(session.id).await?;
        store
            .replace_session_artifacts(session.id, artifacts)
            .await?;
        let event = store
            .append_session_event(
                session.id,
                None,
                None,
                SessionEventType::ArtifactsSet,
                serde_json::json!({ "artifacts": artifacts }),
            )
            .await?;
        self.publish_event(event).await;
        Ok(())
    }

    pub fn session_tool_output_spool_dir(&self, session_id: SessionId) -> PathBuf {
        self.state
            .core
            .tool_output_spool_dir
            .join(session_id.0.to_string())
    }

    pub async fn store_inline_image_blob(
        &self,
        bytes: &[u8],
        mime_type: &str,
        name: Option<&str>,
    ) -> Result<String, SessionImageBlobStoreError> {
        crate::daemon::blobs::store_image_blob_for_state(
            self.state.as_ref(),
            bytes,
            mime_type,
            name,
        )
        .await
        .map(|stored| stored.blob_id)
        .map_err(SessionImageBlobStoreError::from)
    }

    pub async fn session_artifact_path_is_accessible(
        &self,
        store: &Store,
        session: &Session,
        path: &Path,
    ) -> anyhow::Result<bool> {
        let roots = session_artifact_allowed_roots(self, store, session).await?;
        let canonical = match tokio::fs::canonicalize(path).await {
            Ok(canonical) => canonical,
            Err(_) => return Ok(false),
        };
        Ok(roots.iter().any(|root| canonical.starts_with(root)))
    }
}

pub(super) async fn session_artifact_allowed_roots(
    handle: &SessionsHandle,
    store: &Store,
    session: &Session,
) -> anyhow::Result<Vec<PathBuf>> {
    let mut roots = Vec::with_capacity(2);
    if let Some(worktree) = store.get_worktree(session.worktree_id).await? {
        roots.push(canonicalize_existing_or_raw(&PathBuf::from(worktree.root_path)).await);
    }
    roots.push(
        canonicalize_existing_or_raw(&handle.session_tool_output_spool_dir(session.id)).await,
    );
    Ok(roots)
}
