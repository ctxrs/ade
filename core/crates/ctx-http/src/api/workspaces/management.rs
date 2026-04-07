use super::*;
use crate::git_status::emit_worktree_vcs_snapshot_for_worktree;

mod prompt_config;
mod provider_model_preferences;

pub(in crate::api) use prompt_config::*;
pub(in crate::api) use provider_model_preferences::*;

pub(in crate::api) async fn get_workspace_primary_branch(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<WorkspacePrimaryBranchResp>, (StatusCode, Json<ApiErrorResp>)> {
    let ws_id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid workspace id".to_string(),
            }),
        )
    })?);
    let _workspace = state
        .global_store()
        .get_workspace(ws_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "workspace not found".to_string(),
            }),
        ))?;
    let store = match state.core.stores.workspace_access_outcome(ws_id).await {
        Ok(ctx_store::manager::WorkspaceStoreAccessOutcome::Access(access)) => access.store,
        Ok(ctx_store::manager::WorkspaceStoreAccessOutcome::Missing)
        | Ok(ctx_store::manager::WorkspaceStoreAccessOutcome::Deleting) => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "workspace not found".to_string(),
                }),
            ));
        }
        Err(e) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            ));
        }
    };
    let primary_branch = workspace_config::load_primary_branch(&store)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "workspace primary branch is not configured".to_string(),
            }),
        ))?;
    Ok(Json(WorkspacePrimaryBranchResp { primary_branch }))
}

pub(in crate::api) async fn update_workspace_primary_branch(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateWorkspacePrimaryBranchReq>,
) -> Result<Json<WorkspacePrimaryBranchResp>, (StatusCode, Json<ApiErrorResp>)> {
    let ws_id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid workspace id".to_string(),
            }),
        )
    })?);
    let workspace = state
        .global_store()
        .get_workspace(ws_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "workspace not found".to_string(),
            }),
        ))?;
    let primary_branch = req.primary_branch.trim().to_string();
    if primary_branch.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "primary_branch is required".to_string(),
            }),
        ));
    }
    let driver = vcs::driver_for_path(StdPath::new(&workspace.root_path))
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    driver
        .rev_parse_ref(StdPath::new(&workspace.root_path), &primary_branch)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&format!(
                        "primary_branch `{}` does not resolve: {}",
                        primary_branch, e
                    )),
                }),
            )
        })?;
    let store = state.store_for_workspace(ws_id).await.map_err(|e| {
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
    let worktrees = store.list_worktrees(ws_id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    for worktree in worktrees {
        if let Err(err) = emit_worktree_vcs_snapshot_for_worktree(&state, &worktree, true).await {
            tracing::warn!(
                workspace_id = %ws_id.0,
                worktree_id = %worktree.id.0,
                "failed to refresh worktree vcs after primary branch update: {err:#}"
            );
        }
    }
    Ok(Json(WorkspacePrimaryBranchResp { primary_branch }))
}

pub(in crate::api) async fn update_merge_queue_config(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateMergeQueueConfigReq>,
) -> Result<Json<UpdateWorkspaceConfigResp>, (StatusCode, Json<ApiErrorResp>)> {
    let ws_id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid workspace id".to_string(),
            }),
        )
    })?);
    let _workspace = state
        .global_store()
        .get_workspace(ws_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "workspace not found".to_string(),
            }),
        ))?;

    let verify_commands = req
        .verify_command
        .as_ref()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .map(|v| vec![v])
        .unwrap_or_default();

    let store = state.store_for_workspace(ws_id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    let was_enabled = workspace_config::load_merge_queue_config(&store)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .enabled;
    workspace_config::update_merge_queue_config(
        &store,
        workspace_config::MergeQueueConfigUpdate {
            enabled: req.enabled,
            target_branch: req.target_branch,
            verify_commands,
            push_on_success: req.push_on_success,
            push_remote: req.push_remote,
            push_branch: req.push_branch,
            canonical_sync: Some(workspace_config::MergeQueueCanonicalSync::CleanOnly),
        },
    )
    .await
    .map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;

    if !was_enabled && req.enabled {
        crate::merge_queue::schedule_workspace_if_enabled_and_queued(&state, ws_id)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&e.to_string()),
                    }),
                )
            })?;
    } else if was_enabled && !req.enabled {
        crate::merge_queue::cancel_queued_entries_for_disabled_workspace(&state, &store, ws_id)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&e.to_string()),
                    }),
                )
            })?;
    }

    Ok(Json(UpdateWorkspaceConfigResp { ok: true }))
}

pub(in crate::api) async fn get_merge_queue_config(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<WorkspaceMergeQueueConfigResp>, (StatusCode, Json<ApiErrorResp>)> {
    let ws_id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid workspace id".to_string(),
            }),
        )
    })?);
    let _workspace = state
        .global_store()
        .get_workspace(ws_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "workspace not found".to_string(),
            }),
        ))?;

    let store = state.store_for_workspace(ws_id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    let cfg = workspace_config::load_merge_queue_config(&store)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;

    let verify_command = cfg.verify_commands.into_iter().next();
    Ok(Json(WorkspaceMergeQueueConfigResp {
        enabled: cfg.enabled,
        target_branch: cfg.target_branch,
        verify_command,
        push_on_success: cfg.push_on_success,
        push_remote: cfg.push_remote,
        push_branch: cfg.push_branch,
    }))
}

#[derive(Debug, Deserialize)]
pub(in crate::api) struct UpdateWorktreeBootstrapReq {
    #[serde(default)]
    setup_command: Option<String>,
    #[serde(default)]
    timeout_sec: Option<u64>,
    #[serde(default)]
    wait_for_completion: Option<bool>,
}

#[derive(Debug, Serialize)]
pub(in crate::api) struct WorkspaceWorktreeBootstrapConfigResp {
    #[serde(skip_serializing_if = "Option::is_none")]
    setup_command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timeout_sec: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    wait_for_completion: Option<bool>,
}

pub(in crate::api) async fn get_worktree_bootstrap_config(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<WorkspaceWorktreeBootstrapConfigResp>, (StatusCode, Json<ApiErrorResp>)> {
    let ws_id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid workspace id".to_string(),
            }),
        )
    })?);
    let _workspace = state
        .global_store()
        .get_workspace(ws_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "workspace not found".to_string(),
            }),
        ))?;

    let store = state.store_for_workspace(ws_id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    let cfg = workspace_config::load_worktree_bootstrap_config(&store)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;

    Ok(Json(WorkspaceWorktreeBootstrapConfigResp {
        setup_command: cfg.as_ref().and_then(|value| value.setup_command.clone()),
        timeout_sec: cfg.as_ref().and_then(|value| value.timeout_sec),
        wait_for_completion: cfg.as_ref().and_then(|value| value.wait_for_completion),
    }))
}

pub(in crate::api) async fn update_worktree_bootstrap_config(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateWorktreeBootstrapReq>,
) -> Result<Json<UpdateWorkspaceConfigResp>, (StatusCode, Json<ApiErrorResp>)> {
    let ws_id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid workspace id".to_string(),
            }),
        )
    })?);
    let _workspace = state
        .global_store()
        .get_workspace(ws_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "workspace not found".to_string(),
            }),
        ))?;

    let store = state.store_for_workspace(ws_id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    workspace_config::update_worktree_bootstrap_config(
        &store,
        workspace_config::WorktreeBootstrapConfigUpdate {
            setup_command: req.setup_command,
            timeout_sec: req.timeout_sec,
            wait_for_completion: req.wait_for_completion,
        },
    )
    .await
    .map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;

    Ok(Json(UpdateWorkspaceConfigResp { ok: true }))
}

#[derive(Debug, Deserialize)]
pub(in crate::api) struct UpdateExecutionConfigReq {
    environment: String,
    #[serde(default)]
    network_mode: Option<String>,
    #[serde(default)]
    allowlist: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub(in crate::api) struct WorkspaceExecutionConfigResp {
    source: String,               // "workspace" | "daemon_default"
    environment: String,          // "host" | "sandbox"
    network_mode: Option<String>, // "llm_only" | "allowlist" | "all"
    allowlist: Option<Vec<String>>,
}

pub(in crate::api) async fn get_execution_config(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<WorkspaceExecutionConfigResp>, (StatusCode, Json<ApiErrorResp>)> {
    let ws_id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid workspace id".to_string(),
            }),
        )
    })?);
    let _workspace = state
        .global_store()
        .get_workspace(ws_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "workspace not found".to_string(),
            }),
        ))?;

    let settings = crate::settings::load_settings(state.global_store())
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    let mut effective = settings.execution.clone().unwrap_or_default();
    let mut source = "daemon_default".to_string();
    let store = state.store_for_workspace(ws_id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    match workspace_config::load_execution_settings_override(&store).await {
        Ok(Some(ov)) => {
            workspace_config::apply_execution_settings_override(&mut effective, &ov);
            source = "workspace".to_string();
        }
        Ok(None) => {}
        Err(err) => {
            tracing::warn!("failed to load workspace execution config: {err:#}");
        }
    }

    let environment = match effective.mode {
        crate::settings::ExecutionMode::Host => "host",
        crate::settings::ExecutionMode::Sandbox => "sandbox",
    }
    .to_string();
    let network_mode = match effective.container.network_mode {
        crate::settings::ContainerNetworkMode::LlmOnly => "llm_only",
        crate::settings::ContainerNetworkMode::Allowlist => "allowlist",
        crate::settings::ContainerNetworkMode::All => "all",
    }
    .to_string();

    Ok(Json(WorkspaceExecutionConfigResp {
        source,
        environment,
        network_mode: Some(network_mode),
        allowlist: Some(effective.container.allowlist.clone()),
    }))
}

pub(in crate::api) async fn update_execution_config(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateExecutionConfigReq>,
) -> Result<Json<UpdateWorkspaceConfigResp>, (StatusCode, Json<ApiErrorResp>)> {
    let ws_id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid workspace id".to_string(),
            }),
        )
    })?);
    let _workspace = state
        .global_store()
        .get_workspace(ws_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "workspace not found".to_string(),
            }),
        ))?;

    let environment = match req.environment.trim() {
        "host" => crate::workspace_config::ExecutionEnvironment::Host,
        "sandbox" => {
            #[cfg(target_os = "macos")]
            {
                if !crate::workspace_runtime::local_runtime_available(
                    &state.core.data_root,
                    &crate::settings::ContainerRuntimeKind::SharedVmContainer,
                ) {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        Json(ApiErrorResp {
                            error: "AVF sandbox is unavailable on this macOS host. Install or launch through the desktop app so the AVF helper/runtime is present, then try again.".to_string(),
                        }),
                    ));
                }
            }
            crate::workspace_config::ExecutionEnvironment::Sandbox
        }
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "invalid environment (expected host|sandbox)".to_string(),
                }),
            ));
        }
    };

    let network_mode = match req.network_mode.as_deref().map(|v| v.trim()) {
        None | Some("") => None,
        Some("llm_only") => Some(crate::settings::ContainerNetworkMode::LlmOnly),
        Some("allowlist") => Some(crate::settings::ContainerNetworkMode::Allowlist),
        Some("all") => Some(crate::settings::ContainerNetworkMode::All),
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "invalid network_mode (expected llm_only|allowlist|all)".to_string(),
                }),
            ));
        }
    };

    let allowlist = req.allowlist.map(|values| {
        values
            .into_iter()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .collect::<Vec<String>>()
    });

    let store = state.store_for_workspace(ws_id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    workspace_config::update_execution_config(
        &store,
        workspace_config::ExecutionConfigUpdate {
            environment,
            network_mode,
            allowlist,
            image: None,
        },
    )
    .await
    .map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;

    Ok(Json(UpdateWorkspaceConfigResp { ok: true }))
}

#[derive(Debug, Deserialize)]
pub(in crate::api) struct CreateWorkspaceAttachmentReq {
    kind: WorkspaceAttachmentKind,
    name: String,
    source: String,
    #[serde(default)]
    revision: Option<String>,
    #[serde(default)]
    subpath: Option<String>,
    #[serde(default)]
    mount_relpath: Option<String>,
    #[serde(default)]
    mode: Option<AttachmentMode>,
    #[serde(default)]
    update_policy: Option<AttachmentUpdatePolicy>,
}

pub(in crate::api) async fn create_workspace_attachment(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<CreateWorkspaceAttachmentReq>,
) -> Result<Json<Vec<WorkspaceAttachment>>, (StatusCode, Json<ApiErrorResp>)> {
    if req.name.trim().is_empty() || req.source.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "name and source are required".to_string(),
            }),
        ));
    }
    let ws_id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid workspace id".to_string(),
            }),
        )
    })?);
    let workspace = state
        .global_store()
        .get_workspace(ws_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "workspace not found".to_string(),
            }),
        ))?;

    let cfg = attachments::AttachmentConfig {
        kind: req.kind,
        name: req.name,
        source: req.source,
        revision: req.revision,
        subpath: req.subpath,
        mount_relpath: req.mount_relpath,
        mode: req.mode,
        update_policy: req.update_policy,
    };
    attachments::upsert_workspace_attachment(state.as_ref(), workspace.id, cfg)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;

    let attachments = attachments::sync_workspace_attachments(Arc::clone(&state), &workspace, true)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    Ok(Json(attachments))
}

#[derive(Debug, Deserialize)]
pub(in crate::api) struct DeleteWorkspaceAttachmentReq {
    kind: WorkspaceAttachmentKind,
    name: String,
}

pub(in crate::api) async fn delete_workspace_attachment(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<DeleteWorkspaceAttachmentReq>,
) -> Result<Json<Vec<WorkspaceAttachment>>, (StatusCode, Json<ApiErrorResp>)> {
    if req.name.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "name is required".to_string(),
            }),
        ));
    }
    let ws_id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid workspace id".to_string(),
            }),
        )
    })?);
    let workspace = state
        .global_store()
        .get_workspace(ws_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "workspace not found".to_string(),
            }),
        ))?;

    let removed =
        attachments::delete_workspace_attachment(state.as_ref(), workspace.id, req.kind, &req.name)
            .await
            .map_err(|e| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&e.to_string()),
                    }),
                )
            })?;
    if !removed {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "attachment not found".to_string(),
            }),
        ));
    }

    let attachments =
        attachments::sync_workspace_attachments(Arc::clone(&state), &workspace, false)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&e.to_string()),
                    }),
                )
            })?;
    Ok(Json(attachments))
}

pub(in crate::api) async fn workspace_file_completions(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<FileCompletionsQuery>,
) -> Result<Json<Vec<String>>, StatusCode> {
    const DEFAULT_LIMIT: u32 = 20;
    const MAX_LIMIT: u32 = 200;
    const CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(10);

    let ws_id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let ws = state
        .global_store()
        .get_workspace(ws_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let root = PathBuf::from(&ws.root_path);
    if assert_git_repo(&root).await.is_err() {
        return Ok(Json(Vec::new()));
    }

    let query = q.query.unwrap_or_default();
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT) as usize;

    let files = {
        let now = Instant::now();
        let mut cache = state
            .workspaces
            .workspace_file_completions_cache
            .lock()
            .await;
        if let Some(entry) = cache.get_mut(&ws_id) {
            entry.touch();
            if now.duration_since(entry.value.cached_at) <= CACHE_TTL {
                entry.value.files.clone()
            } else {
                drop(cache);
                load_and_cache_workspace_files(&state, ws_id, &root, now).await?
            }
        } else {
            drop(cache);
            load_and_cache_workspace_files(&state, ws_id, &root, now).await?
        }
    };

    Ok(Json(completions::filter_and_rank_paths(
        &files, &query, limit,
    )))
}
