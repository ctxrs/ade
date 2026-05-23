use ctx_route_contracts::workspaces::{WorkspaceFileCompletionsRouteQuery, WorkspaceRouteParams};

use super::file_completions::complete_files_for_workspace_with_runtime;
use super::route_contract::file_completions_route_error;
use super::WorkspaceRouteError;
use crate::daemon::WorkspaceFileCompletionsHandle;

impl WorkspaceFileCompletionsHandle {
    pub async fn workspace_file_completions_for_route(
        &self,
        params: WorkspaceRouteParams,
        query: WorkspaceFileCompletionsRouteQuery,
    ) -> Result<Vec<String>, WorkspaceRouteError> {
        let workspace_id = params.parse_workspace_id()?;
        let (query, limit) = query.into_parts();
        complete_files_for_workspace_with_runtime(
            self.global_store(),
            self.workspace_file_completions_cache(),
            self.perf_telemetry(),
            workspace_id,
            query,
            limit,
        )
        .await
        .map_err(file_completions_route_error)
    }
}
