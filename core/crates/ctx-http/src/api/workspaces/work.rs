use ctx_core::ids::WorkRecordId;

use super::*;

pub(in crate::api) async fn list_workspace_work(
    State(workspaces): State<WorkspaceWorkHandle>,
    Path(id): Path<String>,
    Query(query): Query<WorkspaceWorkListRouteQuery>,
) -> Result<Json<WorkspaceWorkListRouteResponse>, (StatusCode, Json<ApiErrorResp>)> {
    workspaces
        .list_workspace_work_for_route(WorkspaceRouteParams::new(id), query)
        .await
        .map(Json)
        .map_err(workspace_route_api_error)
}

pub(in crate::api) async fn get_workspace_work(
    State(workspaces): State<WorkspaceWorkHandle>,
    Path((id, work_id)): Path<(String, String)>,
) -> Result<Json<WorkspaceWorkDetailRouteResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let work_id = normalize_work_id(work_id);
    workspaces
        .get_workspace_work_for_route(WorkspaceRouteParams::new(id), work_id.0)
        .await
        .map(Json)
        .map_err(workspace_route_api_error)
}

pub(in crate::api) async fn get_workspace_work_report(
    State(workspaces): State<WorkspaceWorkHandle>,
    Path((id, work_id)): Path<(String, String)>,
) -> Result<Json<WorkspaceWorkReportRouteResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let work_id = normalize_work_id(work_id);
    workspaces
        .get_workspace_work_report_for_route(WorkspaceRouteParams::new(id), work_id.0)
        .await
        .map(Json)
        .map_err(workspace_route_api_error)
}

pub(in crate::api) async fn get_workspace_work_inspector(
    State(workspaces): State<WorkspaceWorkHandle>,
    Path((id, work_id)): Path<(String, String)>,
) -> Result<Json<WorkspaceWorkInspectorRouteResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let work_id = normalize_work_id(work_id);
    workspaces
        .get_workspace_work_inspector_for_route(WorkspaceRouteParams::new(id), work_id.0)
        .await
        .map(Json)
        .map_err(workspace_route_api_error)
}

pub(in crate::api) async fn get_workspace_work_context(
    State(workspaces): State<WorkspaceWorkHandle>,
    Path((id, work_id)): Path<(String, String)>,
    Query(query): Query<WorkspaceWorkContextRouteQuery>,
) -> Result<Json<WorkspaceWorkContextRouteResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let work_id = normalize_work_id(work_id);
    workspaces
        .get_workspace_work_context_for_route(WorkspaceRouteParams::new(id), work_id.0, query)
        .await
        .map(Json)
        .map_err(workspace_route_api_error)
}

pub(in crate::api) async fn get_workspace_work_timeline(
    State(workspaces): State<WorkspaceWorkHandle>,
    Path((id, work_id)): Path<(String, String)>,
    Query(query): Query<WorkspaceWorkTimelineRouteQuery>,
) -> Result<Json<WorkspaceWorkTimelineRouteResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let work_id = normalize_work_id(work_id);
    workspaces
        .get_workspace_work_timeline_for_route(WorkspaceRouteParams::new(id), work_id.0, query)
        .await
        .map(Json)
        .map_err(workspace_route_api_error)
}

pub(in crate::api) async fn get_workspace_work_evidence(
    State(workspaces): State<WorkspaceWorkHandle>,
    Path((id, work_id)): Path<(String, String)>,
) -> Result<Json<WorkspaceWorkEvidenceRouteResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let work_id = normalize_work_id(work_id);
    workspaces
        .get_workspace_work_evidence_for_route(WorkspaceRouteParams::new(id), work_id.0)
        .await
        .map(Json)
        .map_err(workspace_route_api_error)
}

pub(in crate::api) async fn create_workspace_work_evidence(
    State(workspaces): State<WorkspaceWorkHandle>,
    Path((id, work_id)): Path<(String, String)>,
    Json(request): Json<WorkspaceWorkEvidenceCreateRouteRequest>,
) -> Result<Json<WorkspaceWorkEvidenceCreateRouteResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let work_id = normalize_work_id(work_id);
    workspaces
        .create_workspace_work_evidence_for_route(WorkspaceRouteParams::new(id), work_id.0, request)
        .await
        .map(Json)
        .map_err(workspace_route_api_error)
}

pub(in crate::api) async fn create_workspace_work_summary(
    State(workspaces): State<WorkspaceWorkHandle>,
    Path((id, work_id)): Path<(String, String)>,
    Json(request): Json<WorkspaceWorkSummaryCreateRouteRequest>,
) -> Result<Json<WorkspaceWorkSummaryCreateRouteResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let work_id = normalize_work_id(work_id);
    workspaces
        .create_workspace_work_summary_for_route(WorkspaceRouteParams::new(id), work_id.0, request)
        .await
        .map(Json)
        .map_err(workspace_route_api_error)
}

fn normalize_work_id(value: String) -> WorkRecordId {
    WorkRecordId::from_id(value)
}
