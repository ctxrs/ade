use super::*;

pub(super) async fn load_workspace_primary_branch(
    store: &ctx_store::Store,
) -> WorkspaceApiResult<WorkspacePrimaryBranchResp> {
    let primary_branch = workspace_config::load_primary_branch(store)
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&error.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "workspace primary branch is not configured".to_string(),
            }),
        ))?;
    Ok(WorkspacePrimaryBranchResp { primary_branch })
}

pub(super) async fn update_workspace_primary_branch_config(
    state: &Arc<AppState>,
    ctx: &WorkspaceRequestContext,
    req: UpdateWorkspacePrimaryBranchReq,
) -> WorkspaceApiResult<WorkspacePrimaryBranchResp> {
    let primary_branch = req.primary_branch.trim().to_string();
    if primary_branch.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "primary_branch is required".to_string(),
            }),
        ));
    }
    let driver = vcs::driver_for_path(StdPath::new(&ctx.workspace.root_path))
        .await
        .map_err(|error| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&error.to_string()),
                }),
            )
        })?;
    driver
        .rev_parse_ref(StdPath::new(&ctx.workspace.root_path), &primary_branch)
        .await
        .map_err(|error| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&format!(
                        "primary_branch `{primary_branch}` does not resolve: {error}"
                    )),
                }),
            )
        })?;
    workspace_config::update_primary_branch(&ctx.store, &primary_branch)
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&error.to_string()),
                }),
            )
        })?;
    let worktrees = ctx
        .store
        .list_worktrees(ctx.workspace_id)
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&error.to_string()),
                }),
            )
        })?;
    for worktree in worktrees {
        if let Err(error) = emit_worktree_vcs_snapshot_for_worktree(state, &worktree, true).await {
            tracing::warn!(
                workspace_id = %ctx.workspace_id.0,
                worktree_id = %worktree.id.0,
                "failed to refresh worktree vcs after primary branch update: {error:#}"
            );
        }
    }
    Ok(WorkspacePrimaryBranchResp { primary_branch })
}

pub(super) async fn update_workspace_merge_queue_config(
    state: &Arc<AppState>,
    ctx: &WorkspaceRequestContext,
    req: UpdateMergeQueueConfigReq,
) -> WorkspaceApiResult<UpdateWorkspaceConfigResp> {
    let verify_commands = req
        .verify_command
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(|value| vec![value])
        .unwrap_or_default();

    let was_enabled = workspace_config::load_merge_queue_config(&ctx.store)
        .await
        .map_err(|error| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&error.to_string()),
                }),
            )
        })?
        .enabled;
    workspace_config::update_merge_queue_config(
        &ctx.store,
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
    .map_err(|error| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&error.to_string()),
            }),
        )
    })?;

    if !was_enabled && req.enabled {
        crate::merge_queue::schedule_workspace_if_enabled_and_queued(state, ctx.workspace_id)
            .await
            .map_err(|error| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&error.to_string()),
                    }),
                )
            })?;
    } else if was_enabled && !req.enabled {
        crate::merge_queue::cancel_queued_entries_for_disabled_workspace(
            state,
            &ctx.store,
            ctx.workspace_id,
        )
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&error.to_string()),
                }),
            )
        })?;
    }

    Ok(UpdateWorkspaceConfigResp { ok: true })
}

pub(super) async fn load_workspace_merge_queue_config(
    store: &ctx_store::Store,
) -> WorkspaceApiResult<WorkspaceMergeQueueConfigResp> {
    let cfg = workspace_config::load_merge_queue_config(store)
        .await
        .map_err(|error| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&error.to_string()),
                }),
            )
        })?;

    let verify_command = cfg.verify_commands.into_iter().next();
    Ok(WorkspaceMergeQueueConfigResp {
        enabled: cfg.enabled,
        target_branch: cfg.target_branch,
        verify_command,
        push_on_success: cfg.push_on_success,
        push_remote: cfg.push_remote,
        push_branch: cfg.push_branch,
    })
}

pub(super) async fn load_workspace_execution_config(
    state: &Arc<AppState>,
    ctx: &WorkspaceRequestContext,
) -> WorkspaceApiResult<WorkspaceExecutionConfigResp> {
    let settings = crate::settings::load_settings(state.global_store())
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&error.to_string()),
                }),
            )
        })?;
    let mut effective = settings.execution.clone().unwrap_or_default();
    let mut source = "daemon_default".to_string();
    match workspace_config::load_execution_settings_override(&ctx.store).await {
        Ok(Some(override_config)) => {
            workspace_config::apply_execution_settings_override(&mut effective, &override_config);
            source = "workspace".to_string();
        }
        Ok(None) => {}
        Err(error) => {
            tracing::warn!("failed to load workspace execution config: {error:#}");
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

    Ok(WorkspaceExecutionConfigResp {
        source,
        environment,
        network_mode: Some(network_mode),
        allowlist: Some(effective.container.allowlist.clone()),
    })
}

pub(super) async fn update_workspace_execution_config(
    state: &Arc<AppState>,
    ctx: &WorkspaceRequestContext,
    req: UpdateExecutionConfigReq,
) -> WorkspaceApiResult<UpdateWorkspaceConfigResp> {
    let environment = match req.environment.trim() {
        "host" => ctx_workspace_config::ExecutionEnvironment::Host,
        "sandbox" => {
            #[cfg(target_os = "macos")]
            {
                if !ctx_harness_runtime::local_runtime_available(
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
            ctx_workspace_config::ExecutionEnvironment::Sandbox
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

    let network_mode = match req.network_mode.as_deref().map(|value| value.trim()) {
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
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect::<Vec<String>>()
    });

    workspace_config::update_execution_config(
        &ctx.store,
        workspace_config::ExecutionConfigUpdate {
            environment,
            network_mode,
            allowlist,
            image: None,
        },
    )
    .await
    .map_err(|error| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&error.to_string()),
            }),
        )
    })?;

    Ok(UpdateWorkspaceConfigResp { ok: true })
}
