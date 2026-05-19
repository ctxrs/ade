use ctx_core::ids::{RunId, WorkspaceId};
use ctx_core::models::{RunArchiveIngestBatch, RunArchiveIngestCursor};

use crate::daemon::{WorkspaceStoreAccessError, WorkspacesHandle};

#[derive(Debug)]
pub enum RunArchiveIngestError {
    WorkspaceNotFound,
    AcknowledgementConflict(&'static str),
    Internal(anyhow::Error),
}

impl WorkspacesHandle {
    pub async fn build_run_archive_ingest_batch(
        &self,
        workspace_id: WorkspaceId,
        run_id: RunId,
        max_items: u32,
    ) -> Result<Option<RunArchiveIngestBatch>, RunArchiveIngestError> {
        let store = self
            .existing_workspace_store(workspace_id)
            .await
            .map_err(run_archive_workspace_store_error)?;
        let batch = store
            .build_run_archive_ingest_batch(run_id, max_items)
            .await
            .map_err(RunArchiveIngestError::Internal)?;
        Ok(batch.filter(|batch| batch.run.workspace_id == workspace_id))
    }

    pub async fn acknowledge_run_archive_ingest_batch(
        &self,
        workspace_id: WorkspaceId,
        run_id: RunId,
        max_items: u32,
        batch: RunArchiveIngestBatch,
    ) -> Result<RunArchiveIngestCursor, RunArchiveIngestError> {
        let store = self
            .existing_workspace_store(workspace_id)
            .await
            .map_err(run_archive_workspace_store_error)?;
        let cursor = store
            .get_run_archive_ingest_cursor(run_id)
            .await
            .map_err(RunArchiveIngestError::Internal)?;
        let current_watermark = cursor
            .as_ref()
            .map(|cursor| cursor.watermark)
            .unwrap_or_default();
        if batch.from != current_watermark {
            return Err(RunArchiveIngestError::AcknowledgementConflict(
                "archive ingest acknowledgement is stale for the current cursor",
            ));
        }
        let Some(mut expected_batch) = store
            .build_run_archive_ingest_batch_after(run_id, batch.from, max_items, cursor.is_none())
            .await
            .map_err(RunArchiveIngestError::Internal)?
        else {
            return Err(RunArchiveIngestError::AcknowledgementConflict(
                "archive ingest acknowledgement does not match an available batch",
            ));
        };
        expected_batch.created_at = batch.created_at;
        if expected_batch != batch {
            return Err(RunArchiveIngestError::AcknowledgementConflict(
                "archive ingest acknowledgement does not match the current batch",
            ));
        }
        store
            .acknowledge_run_archive_ingest_batch(&batch)
            .await
            .map_err(RunArchiveIngestError::Internal)
    }
}

fn run_archive_workspace_store_error(error: WorkspaceStoreAccessError) -> RunArchiveIngestError {
    match error {
        WorkspaceStoreAccessError::NotFound => RunArchiveIngestError::WorkspaceNotFound,
        WorkspaceStoreAccessError::Unavailable(error) => RunArchiveIngestError::Internal(error),
    }
}
