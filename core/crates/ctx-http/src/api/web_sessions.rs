use super::*;

#[derive(Debug, Deserialize)]
pub(super) struct WebSessionCreatePayload {
    session_id: Option<String>,
    worktree_id: Option<String>,
    url: String,
    viewport: Option<WebSessionViewport>,
    fps: Option<u32>,
}

pub(super) async fn create_web_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<WebSessionCreatePayload>,
) -> Result<Json<WebSessionInfo>, (StatusCode, Json<ApiErrorResp>)> {
    if payload.url.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "url is required".to_string(),
            }),
        ));
    }

    let session_id = payload.session_id.clone();
    let worktree_id = payload.worktree_id.clone();
    let work_dir = resolve_web_session_work_dir(&state, session_id.clone(), worktree_id.clone())
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;

    let node_runtime = crate::installer::ensure_node_runtime(
        state.as_ref(),
        None,
        "web_session_worker",
        &state.core.data_root,
        ctx_provider_install::install_state::InstallTarget::Host,
    )
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: format!("failed to prepare node runtime: {e}"),
            }),
        )
    })?;

    let worker_bundle = crate::web_sessions::ensure_worker_bundle_for_node_runtime(
        &state.core.data_root,
        &node_runtime,
    )
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: format!("failed to prepare web session worker: {e}"),
            }),
        )
    })?;

    let req = WebSessionCreateRequest {
        url: payload.url,
        viewport: payload.viewport,
        fps: payload.fps,
        work_dir,
        session_id,
        worktree_id,
        node_bin: node_runtime.node_bin,
        worker_path: worker_bundle.worker_path,
        node_modules_path: worker_bundle.node_modules_path,
    };

    let handle = state
        .transport
        .web_sessions
        .create(req)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: format!("failed to create web session: {e}"),
                }),
            )
        })?;

    let mut info = handle.snapshot().await;
    let base_url = resolve_request_base_url(&headers, &state.core.daemon_url);
    info.stream_url = Some(format!("{}{}", base_url, info.stream_path));
    Ok(Json(info))
}

pub(super) async fn list_web_sessions(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<WebSessionInfo>>, StatusCode> {
    let mut sessions = state.transport.web_sessions.list().await;
    let base_url = resolve_request_base_url(&headers, &state.core.daemon_url);
    for session in sessions.iter_mut() {
        session.stream_url = Some(format!("{}{}", base_url, session.stream_path));
    }
    Ok(Json(sessions))
}

pub(super) async fn get_web_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<WebSessionInfo>, StatusCode> {
    let handle = state
        .transport
        .web_sessions
        .get(&id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;
    let mut info = handle.snapshot().await;
    let base_url = resolve_request_base_url(&headers, &state.core.daemon_url);
    info.stream_url = Some(format!("{}{}", base_url, info.stream_path));
    Ok(Json(info))
}

pub(super) async fn run_web_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(mut payload): Json<WebSessionRunRequest>,
) -> Result<Json<WebSessionRunResponse>, StatusCode> {
    if payload.timeout_ms.is_none() {
        payload.timeout_ms = Some(5 * 60 * 1000);
    }
    let resp = state
        .transport
        .web_sessions
        .run(&id, payload)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(resp))
}

pub(super) async fn eval_web_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(mut payload): Json<WebSessionRunRequest>,
) -> Result<Json<WebSessionRunResponse>, StatusCode> {
    if payload.timeout_ms.is_none() {
        payload.timeout_ms = Some(5 * 60 * 1000);
    }
    let resp = state
        .transport
        .web_sessions
        .eval(&id, payload)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(resp))
}

pub(super) async fn close_web_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    state
        .transport
        .web_sessions
        .close(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn web_session_view(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Response, StatusCode> {
    let handle = state
        .transport
        .web_sessions
        .get(&id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;
    let info = handle.snapshot().await;
    let body = render_web_session_view(&info);
    Ok(([(header::CONTENT_TYPE, "text/html; charset=utf-8")], body).into_response())
}

pub(super) async fn resolve_web_session_work_dir(
    state: &Arc<AppState>,
    session_id: Option<String>,
    worktree_id: Option<String>,
) -> anyhow::Result<Option<PathBuf>> {
    if let Some(worktree_id) = worktree_id {
        let worktree_id =
            WorktreeId(uuid::Uuid::parse_str(&worktree_id).context("invalid worktree id")?);
        let store = state.store_for_worktree(worktree_id).await?;
        let worktree = store
            .get_worktree(worktree_id)
            .await?
            .context("worktree not found")?;
        return Ok(Some(PathBuf::from(worktree.root_path)));
    }
    if let Some(session_id) = session_id {
        let session_id =
            SessionId(uuid::Uuid::parse_str(&session_id).context("invalid session id")?);
        let store = state.store_for_session(session_id).await?;
        let session = store
            .get_session(session_id)
            .await?
            .context("session not found")?;
        let worktree = store
            .get_worktree(session.worktree_id)
            .await?
            .context("worktree not found")?;
        return Ok(Some(PathBuf::from(worktree.root_path)));
    }
    Ok(None)
}
