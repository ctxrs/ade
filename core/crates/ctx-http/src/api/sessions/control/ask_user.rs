use super::super::*;

pub(crate) async fn submit_ask_user_question(
    State(state): State<SessionsHandle>,
    Path(id): Path<String>,
    Json(req): Json<SubmitAskUserQuestionRouteRequest>,
) -> Result<
    Json<ctx_daemon::daemon::SubmitAskUserQuestionRouteResponse>,
    (StatusCode, Json<ApiErrorResp>),
> {
    state
        .submit_ask_user_question_for_route(SessionRouteParams::new(id), req)
        .await
        .map(Json)
        .map_err(session_control_api_error)
}
