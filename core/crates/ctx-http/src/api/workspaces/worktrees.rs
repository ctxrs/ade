use super::*;

pub(in crate::api) async fn get_worktree(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Worktree>, StatusCode> {
    let worktree_id = WorktreeId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let store = state
        .store_for_worktree(worktree_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    match store
        .get_worktree(worktree_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        Some(mut wt) => {
            let data_plane = crate::worktree_data_plane::resolve_worktree_data_plane(&state, &wt)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            wt.root_path = data_plane.live_worktree_root.to_string_lossy().to_string();
            Ok(Json(wt))
        }
        None => Err(StatusCode::NOT_FOUND),
    }
}

pub(in crate::api) async fn get_worktree_bootstrap_logs(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Response, StatusCode> {
    let worktree_id = WorktreeId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let store = state
        .store_for_worktree(worktree_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let worktree = store
        .get_worktree(worktree_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let Some(path) = worktree.bootstrap_log_path.as_deref() else {
        return Err(StatusCode::NOT_FOUND);
    };
    let log_root = logs::logs_dir(&state.core.data_root).join("worktree-bootstrap");
    if !path_resolves_within_root(StdPath::new(path), &log_root).await {
        return Err(StatusCode::NOT_FOUND);
    }
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

    let filename = format!("worktree-bootstrap-{}.log", worktree_id.0);
    let mut resp = Response::new(Body::from(bytes));
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    resp.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        header::HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
            .unwrap_or_else(|_| header::HeaderValue::from_static("attachment")),
    );
    Ok(resp)
}
