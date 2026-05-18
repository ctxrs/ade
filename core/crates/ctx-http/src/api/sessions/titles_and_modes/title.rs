use super::*;

pub(crate) async fn generate_session_title(
    State(state): State<SessionsHandle>,
    Path(id): Path<String>,
    Json(req): Json<GenerateSessionTitleRouteRequest>,
) -> Result<Json<GenerateSessionTitleRouteResponse>, ApiErr> {
    state
        .generate_session_title_for_route(SessionRouteParams::new(id), req)
        .await
        .map(Json)
        .map_err(session_title_model_mode_api_error)
}
