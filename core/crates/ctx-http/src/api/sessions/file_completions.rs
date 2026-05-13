use super::*;

pub(crate) async fn session_file_completions(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<FileCompletionsQuery>,
) -> Result<Json<Vec<String>>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    crate::daemon::workspaces::complete_files_for_session(&state, session_id, q.query, q.limit)
        .await
        .map(Json)
        .map_err(map_file_completions_error)
}
