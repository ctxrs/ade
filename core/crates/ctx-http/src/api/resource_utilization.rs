use super::*;
use ctx_daemon::daemon::resource_utilization as daemon_resource_utilization;
use ctx_daemon::daemon::WorkspacesHandle;

#[derive(Debug, Deserialize)]
pub(in crate::api) struct ResourceUtilizationQuery {
    workspace_id: String,
}

pub(in crate::api) async fn resource_utilization(
    State(state): State<WorkspacesHandle>,
    Query(query): Query<ResourceUtilizationQuery>,
) -> Result<Json<ctx_resource_utilization::ResourceUtilizationSnapshot>, StatusCode> {
    let workspace_id = WorkspaceId(
        uuid::Uuid::parse_str(&query.workspace_id).map_err(|_| StatusCode::BAD_REQUEST)?,
    );
    state
        .workspace_resource_utilization_snapshot(workspace_id)
        .await
        .map(Json)
        .map_err(resource_utilization_status)
}

fn resource_utilization_status(
    error: daemon_resource_utilization::ResourceUtilizationSnapshotError,
) -> StatusCode {
    match error {
        daemon_resource_utilization::ResourceUtilizationSnapshotError::Disabled
        | daemon_resource_utilization::ResourceUtilizationSnapshotError::WorkspaceNotFound => {
            StatusCode::NOT_FOUND
        }
        daemon_resource_utilization::ResourceUtilizationSnapshotError::Internal => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}
