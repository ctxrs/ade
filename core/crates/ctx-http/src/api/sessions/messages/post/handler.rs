use super::super::*;

pub(crate) async fn post_message(
    State(state): State<SessionsHandle>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
    Json(req): Json<PostSessionMessageRouteRequest>,
) -> Result<Json<PostSessionMessageRouteResponse>, ApiErr> {
    let run_id_header = headers
        .get("x-ctx-run-id")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_string());

    state
        .post_session_message_for_route(
            SessionRouteParams::new(id),
            req,
            PostSessionMessageRouteContext::new(run_id_header),
        )
        .await
        .map(Json)
        .map_err(session_message_api_error)
}
