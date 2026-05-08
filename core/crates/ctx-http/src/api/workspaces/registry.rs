use super::*;

#[derive(Debug, Deserialize)]
pub(in crate::api) struct CreateWorkspaceReq {
    root_path: String,
    name: Option<String>,
}

pub(in crate::api) async fn list_workspaces(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<Workspace>>, StatusCode> {
    state
        .global_store()
        .list_workspaces()
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

pub(in crate::api) async fn get_workspace(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Workspace>, StatusCode> {
    let id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    match state
        .global_store()
        .get_workspace(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        Some(ws) => {
            state
                .telemetry
                .telemetry
                .emit(TelemetryEvent::workspace_opened())
                .await;
            Ok(Json(ws))
        }
        None => Err(StatusCode::NOT_FOUND),
    }
}

pub(in crate::api) async fn create_workspace(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateWorkspaceReq>,
) -> Result<Json<Workspace>, (StatusCode, Json<ApiErrorResp>)> {
    let raw = req.root_path.trim();
    if raw.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "root_path is required".to_string(),
            }),
        ));
    }

    let expanded = if raw == "~" || raw.starts_with("~/") {
        let base = directories::BaseDirs::new().ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "could not resolve home directory to expand '~'".to_string(),
                }),
            )
        })?;
        let home = base.home_dir();
        if raw == "~" {
            home.to_path_buf()
        } else {
            home.join(raw.trim_start_matches("~/"))
        }
    } else {
        PathBuf::from(raw)
    };

    let root_path = tokio::fs::canonicalize(&expanded).await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: format!("invalid root_path '{}': {}", expanded.to_string_lossy(), e),
            }),
        )
    })?;

    let vcs = vcs::driver_for_path(&root_path).await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: e.to_string(),
            }),
        )
    })?;
    vcs.assert_repo(&root_path).await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: e.to_string(),
            }),
        )
    })?;

    let primary_branch = detect_workspace_primary_branch(vcs.kind(), &root_path, vcs.as_ref())
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;

    let root_path_str = root_path.to_string_lossy().to_string();

    let name = req.name.unwrap_or_else(|| {
        root_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("workspace")
            .to_string()
    });
    let workspace = state
        .global_store()
        .create_workspace(name, root_path_str, vcs.kind())
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    let store = state.store_for_workspace(workspace.id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    workspace_config::update_primary_branch(&store, &primary_branch)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    state
        .telemetry
        .telemetry
        .emit(TelemetryEvent::workspace_registered())
        .await;
    Ok(Json(workspace))
}

pub(in crate::api) async fn delete_workspace(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let workspace = state
        .global_store()
        .get_workspace(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let worktrees = match state.store_for_workspace(id).await {
        Ok(store) => store.list_worktrees(id).await.unwrap_or_default(),
        Err(_) => Vec::new(),
    };
    state.core.stores.begin_workspace_delete(id).await;
    let delete_result = async {
        for worktree in &worktrees {
            if let Err(err) = vcs_hooks::cleanup_worktree_hooks(&state, &workspace, worktree).await
            {
                tracing::warn!(
                    workspace_id = %id.0,
                    worktree_id = %worktree.id.0,
                    "failed to remove vcs hooks: {err:#}"
                );
            }
        }
        state.cleanup_workspace(id).await;
        state.core.stores.evict_workspace_and_wait_closed(id).await;
        state
            .global_store()
            .delete_workspace_indexes(id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        state
            .global_store()
            .delete_workspace(id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        Ok::<(), StatusCode>(())
    }
    .await;
    state.core.stores.finish_workspace_delete(id).await;
    delete_result?;
    if let Err(err) = vcs_hooks::cleanup_workspace_hooks(&state.core.data_root, id).await {
        tracing::warn!(
            workspace_id = %id.0,
            "failed to remove vcs hooks: {err:#}"
        );
    }
    let workspace_db_dir = state
        .core
        .data_root
        .join("db")
        .join("workspaces")
        .join(id.0.to_string());
    let _ = tokio::fs::remove_dir_all(workspace_db_dir).await;
    Ok(StatusCode::NO_CONTENT)
}
