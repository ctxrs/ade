use std::path::{Path, PathBuf};

use anyhow::Result;
use ctx_core::ids::SessionId;
use ctx_core::models::{Artifact, Session, SessionEventType, Worktree};
use ctx_store::Store;

use crate::daemon::handle::{SessionArtifactsHandle, SessionsHandle};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum SessionImageBlobStoreError {
    PayloadTooLarge,
    UnsupportedMediaType,
    Internal,
}

impl From<ctx_session_artifacts::ImageBlobStoreError> for SessionImageBlobStoreError {
    fn from(error: ctx_session_artifacts::ImageBlobStoreError) -> Self {
        match error {
            ctx_session_artifacts::ImageBlobStoreError::PayloadTooLarge => Self::PayloadTooLarge,
            ctx_session_artifacts::ImageBlobStoreError::UnsupportedMediaType => {
                Self::UnsupportedMediaType
            }
            ctx_session_artifacts::ImageBlobStoreError::Internal => Self::Internal,
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
        let session_spool_dir = self.session_tool_output_spool_dir(session.id);
        ctx_session_artifacts::session_artifact_path_is_accessible(
            store,
            session,
            &session_spool_dir,
            path,
        )
        .await
        .map_err(Into::into)
    }
}

impl SessionArtifactsHandle {
    pub(in crate::daemon) async fn replace_session_artifacts_and_publish(
        &self,
        session: &Session,
        artifacts: &[Artifact],
    ) -> Result<()> {
        let store = self
            .existing_session_store_for_write(session.id)
            .await
            .map_err(|error| anyhow::anyhow!("session artifact store unavailable: {error:?}"))?;
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
}
