use super::*;
use ctx_daemon::daemon::{
    ResourceUtilizationRouteError, ResourceUtilizationRouteErrorKind,
    ResourceUtilizationRouteQuery, ResourceUtilizationRouteResponse, WorkspacesHandle,
};

pub(in crate::api) async fn resource_utilization(
    State(state): State<WorkspacesHandle>,
    Query(query): Query<ResourceUtilizationRouteQuery>,
) -> Result<Json<ResourceUtilizationRouteResponse>, StatusCode> {
    state
        .workspace_resource_utilization_snapshot_for_route(query)
        .await
        .map(Json)
        .map_err(resource_utilization_status)
}

fn resource_utilization_status(error: ResourceUtilizationRouteError) -> StatusCode {
    match error.kind() {
        ResourceUtilizationRouteErrorKind::BadRequest => StatusCode::BAD_REQUEST,
        ResourceUtilizationRouteErrorKind::NotFound => StatusCode::NOT_FOUND,
        ResourceUtilizationRouteErrorKind::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
