use super::*;

pub(crate) async fn session_file_completions(
    State(state): State<SessionsHandle>,
    Path(id): Path<String>,
    Query(q): Query<FileCompletionsQuery>,
) -> Result<Json<Vec<String>>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    state
        .complete_files_for_session(session_id, q.query, q.limit)
        .await
        .map(Json)
        .map_err(map_file_completions_error)
}
