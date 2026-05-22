use ctx_route_contracts::downloads::TextRouteDownload;
pub use ctx_route_contracts::merge_queue::{
    ListMergeQueueEntriesRouteRequest, MergeQueueEntryRouteError, MergeQueueEntryRouteErrorKind,
    MergeQueueEntryRouteParams, MergeQueueEntryRouteResponse, MergeQueueLogDownloadRouteError,
    MergeQueueLogDownloadRouteErrorKind,
};

use crate::daemon::{RouteFileDownloadError, WorkspaceStoreAccessError, WorkspacesHandle};

impl WorkspacesHandle {
    pub async fn list_merge_queue_entry_responses_for_route(
        &self,
        req: ListMergeQueueEntriesRouteRequest,
    ) -> Result<Vec<MergeQueueEntryRouteResponse>, MergeQueueEntryRouteError> {
        let workspace_id = req.parse_workspace_id()?;
        let store = self
            .existing_workspace_store(workspace_id)
            .await
            .map_err(list_store_error)?;
        let entries = store
            .list_merge_queue_entries(workspace_id, req.limit())
            .await
            .map_err(|error| MergeQueueEntryRouteError::internal(error.to_string()))?;
        Ok(entries.into_iter().map(Into::into).collect())
    }

    pub async fn cancel_merge_queue_entry_for_route(
        &self,
        params: MergeQueueEntryRouteParams,
    ) -> Result<MergeQueueEntryRouteResponse, MergeQueueEntryRouteError> {
        let (workspace_id, entry_id) = params.parse()?;
        self.cancel_merge_queue_entry(workspace_id, entry_id)
            .await
            .map(Into::into)
            .map_err(|error| MergeQueueEntryRouteError::bad_request(error.to_string()))
    }

    pub async fn retry_merge_queue_entry_for_route(
        &self,
        params: MergeQueueEntryRouteParams,
    ) -> Result<MergeQueueEntryRouteResponse, MergeQueueEntryRouteError> {
        let (workspace_id, entry_id) = params.parse()?;
        self.retry_merge_queue_entry(workspace_id, entry_id)
            .await
            .map(Into::into)
            .map_err(|error| MergeQueueEntryRouteError::bad_request(error.to_string()))
    }

    pub async fn download_merge_queue_entry_logs_for_route_params(
        &self,
        params: MergeQueueEntryRouteParams,
    ) -> Result<TextRouteDownload, MergeQueueLogDownloadRouteError> {
        let (workspace_id, entry_id) = params.parse_for_log_download()?;
        self.download_merge_queue_entry_logs_for_route(workspace_id, entry_id)
            .await
            .map_err(merge_queue_log_download_route_file_error)
    }
}

fn merge_queue_log_download_route_file_error(
    error: RouteFileDownloadError,
) -> MergeQueueLogDownloadRouteError {
    match error {
        RouteFileDownloadError::NotFound => MergeQueueLogDownloadRouteError::not_found(),
        RouteFileDownloadError::Internal => MergeQueueLogDownloadRouteError::internal(),
    }
}

fn list_store_error(error: WorkspaceStoreAccessError) -> MergeQueueEntryRouteError {
    match error {
        WorkspaceStoreAccessError::NotFound => {
            MergeQueueEntryRouteError::internal("workspace not found")
        }
        WorkspaceStoreAccessError::Unavailable(error) => {
            MergeQueueEntryRouteError::internal(error.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_download_route_file_error_classification_is_transport_safe() {
        assert_eq!(
            merge_queue_log_download_route_file_error(RouteFileDownloadError::NotFound).kind(),
            MergeQueueLogDownloadRouteErrorKind::NotFound
        );
        assert_eq!(
            merge_queue_log_download_route_file_error(RouteFileDownloadError::Internal).kind(),
            MergeQueueLogDownloadRouteErrorKind::Internal
        );
    }
}
