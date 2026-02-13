use super::*;

pub(crate) async fn session_file_completions(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<FileCompletionsQuery>,
) -> Result<Json<Vec<String>>, StatusCode> {
    const DEFAULT_LIMIT: u32 = 20;
    const MAX_LIMIT: u32 = 200;
    const CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(10);

    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);

    let store = state
        .store_for_session(session_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let session = store
        .get_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let worktree = store
        .get_worktree(session.worktree_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let query = q.query.unwrap_or_default();
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT) as usize;

    let files = {
        let now = Instant::now();
        let mut cache = state.workspaces.file_completions_cache.lock().await;
        if let Some(entry) = cache.get_mut(&worktree.id) {
            entry.touch();
            if now.duration_since(entry.value.cached_at) <= CACHE_TTL {
                entry.value.files.clone()
            } else {
                drop(cache);
                load_and_cache_worktree_files(&state, &worktree, now).await?
            }
        } else {
            drop(cache);
            load_and_cache_worktree_files(&state, &worktree, now).await?
        }
    };

    Ok(Json(completions::filter_and_rank_paths(
        &files, &query, limit,
    )))
}
