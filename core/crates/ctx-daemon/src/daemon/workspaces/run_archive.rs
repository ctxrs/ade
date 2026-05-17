use ctx_core::ids::{RunId, WorkspaceId};
use ctx_core::models::{RunArchiveIngestBatch, RunArchiveIngestCursor};

use crate::daemon::WorkspacesHandle;

use super::RunArchiveIngestError;

const DEFAULT_RUN_ARCHIVE_BATCH_ITEMS: u32 = 250;
const MAX_RUN_ARCHIVE_BATCH_ITEMS: u32 = 1_000;

#[derive(Debug)]
pub struct BuildRunArchiveIngestBatchRouteRequest {
    pub workspace_id: WorkspaceId,
    pub run_id: RunId,
    pub max_items: Option<u32>,
}

#[derive(Debug)]
pub struct AcknowledgeRunArchiveIngestBatchRouteRequest {
    pub workspace_id: WorkspaceId,
    pub run_id: RunId,
    pub max_items: Option<u32>,
    pub batch: RunArchiveIngestBatch,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum RunArchiveRouteErrorKind {
    BadRequest,
    NotFound,
    Conflict,
    Internal,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RunArchiveRouteError {
    kind: RunArchiveRouteErrorKind,
    message: String,
}

impl RunArchiveRouteError {
    fn new(kind: RunArchiveRouteErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    fn bad_request(message: impl Into<String>) -> Self {
        Self::new(RunArchiveRouteErrorKind::BadRequest, message)
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self::new(RunArchiveRouteErrorKind::NotFound, message)
    }

    fn conflict(message: impl Into<String>) -> Self {
        Self::new(RunArchiveRouteErrorKind::Conflict, message)
    }

    fn internal(message: impl Into<String>) -> Self {
        Self::new(RunArchiveRouteErrorKind::Internal, message)
    }

    pub fn kind(&self) -> RunArchiveRouteErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl WorkspacesHandle {
    pub async fn build_run_archive_ingest_batch_for_route(
        &self,
        req: BuildRunArchiveIngestBatchRouteRequest,
    ) -> Result<Option<RunArchiveIngestBatch>, RunArchiveRouteError> {
        let max_items = requested_batch_item_limit(req.max_items)?;
        self.build_run_archive_ingest_batch(req.workspace_id, req.run_id, max_items)
            .await
            .map_err(|error| run_archive_route_error("build", error))
    }

    pub async fn acknowledge_run_archive_ingest_batch_for_route(
        &self,
        req: AcknowledgeRunArchiveIngestBatchRouteRequest,
    ) -> Result<RunArchiveIngestCursor, RunArchiveRouteError> {
        let max_items = requested_batch_item_limit(req.max_items)?;
        validate_acknowledgement_batch(req.workspace_id, req.run_id, &req.batch)?;
        self.acknowledge_run_archive_ingest_batch(
            req.workspace_id,
            req.run_id,
            max_items,
            req.batch,
        )
        .await
        .map_err(|error| run_archive_route_error("acknowledge", error))
    }
}

fn requested_batch_item_limit(max_items: Option<u32>) -> Result<u32, RunArchiveRouteError> {
    let max_items = max_items.unwrap_or(DEFAULT_RUN_ARCHIVE_BATCH_ITEMS);
    if max_items == 0 || max_items > MAX_RUN_ARCHIVE_BATCH_ITEMS {
        return Err(RunArchiveRouteError::bad_request(format!(
            "max_items must be between 1 and {MAX_RUN_ARCHIVE_BATCH_ITEMS}"
        )));
    }
    Ok(max_items)
}

fn validate_acknowledgement_batch(
    workspace_id: WorkspaceId,
    run_id: RunId,
    batch: &RunArchiveIngestBatch,
) -> Result<(), RunArchiveRouteError> {
    if batch.run.workspace_id != workspace_id {
        return Err(RunArchiveRouteError::bad_request(
            "archive ingest batch workspace_id must match route workspace id",
        ));
    }
    if batch.run.id != run_id {
        return Err(RunArchiveRouteError::bad_request(
            "archive ingest batch run id must match route run id",
        ));
    }
    if batch.run.org_id.is_none() || !batch.scope.is_cloud_visible() {
        return Err(RunArchiveRouteError::bad_request(
            "archive ingest acknowledgement requires an org-visible batch",
        ));
    }
    Ok(())
}

fn run_archive_route_error(
    action: &'static str,
    error: RunArchiveIngestError,
) -> RunArchiveRouteError {
    match error {
        RunArchiveIngestError::WorkspaceNotFound => {
            RunArchiveRouteError::not_found("workspace not found for run archive ingest")
        }
        RunArchiveIngestError::AcknowledgementConflict(message) => {
            RunArchiveRouteError::conflict(message)
        }
        RunArchiveIngestError::Internal(error) => RunArchiveRouteError::internal(format!(
            "failed to {action} run archive ingest batch: {error:#}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requested_batch_item_limit_defaults_and_accepts_boundaries() {
        assert_eq!(requested_batch_item_limit(None).unwrap(), 250);
        assert_eq!(requested_batch_item_limit(Some(1)).unwrap(), 1);
        assert_eq!(requested_batch_item_limit(Some(1_000)).unwrap(), 1_000);
    }

    #[test]
    fn requested_batch_item_limit_rejects_out_of_range_values() {
        for max_items in [0, 1_001] {
            let error = requested_batch_item_limit(Some(max_items)).unwrap_err();
            assert_eq!(error.kind(), RunArchiveRouteErrorKind::BadRequest);
            assert_eq!(error.message(), "max_items must be between 1 and 1000");
        }
    }
}
