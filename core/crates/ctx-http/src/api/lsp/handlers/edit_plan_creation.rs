use super::super::*;

pub(in crate::api) async fn lsp_code_actions_by_diagnostic_plan(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspCodeActionsByDiagnosticPlanReq>,
) -> Result<Json<Vec<crate::edit_plans::EditPlanSummary>>, (StatusCode, Json<ApiErrorResp>)> {
    if !state.core.lsp.enabled() || !state.core.lsp_edit_plans_enabled {
        return Err((
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "LSP edit plans disabled".to_string(),
            }),
        ));
    }

    let (sid, _, worktree_id, root, file, _) =
        resolve_session_root_and_file(&state, &req.session_id, &req.path)
            .await
            .map_err(|status| {
                (
                    status,
                    Json(ApiErrorResp {
                        error: "invalid LSP target".to_string(),
                    }),
                )
            })?;

    let diag: lsp_types::Diagnostic =
        serde_json::from_value(req.diagnostic.clone()).map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;

    let actions = state
        .core
        .lsp
        .code_actions_typed(
            &root,
            &file,
            diag.range,
            vec![diag.clone()],
            Some(vec![lsp_types::CodeActionKind::QUICKFIX]),
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

    let mut plans: Vec<crate::edit_plans::EditPlanSummary> = Vec::new();
    for action in actions {
        let (title, edit, preferred) = match action {
            lsp_types::CodeActionOrCommand::CodeAction(ca) => {
                let edit = ca.edit.or_else(|| {
                    ca.command
                        .as_ref()
                        .and_then(extract_workspace_edit_from_command)
                });
                (ca.title, edit, ca.is_preferred.unwrap_or(false))
            }
            lsp_types::CodeActionOrCommand::Command(cmd) => {
                let edit = extract_workspace_edit_from_command(&cmd);
                (cmd.title, edit, false)
            }
        };
        let Some(edit) = edit else { continue };
        let plan =
            crate::edit_plans::workspace_edit_to_plan(&root, &root, sid, worktree_id, title, edit)
                .map_err(|e| {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(ApiErrorResp {
                            error: logs::redact_sensitive(&e.to_string()),
                        }),
                    )
                })?;
        let summary = plan.to_summary();
        state.persist_edit_plan(&plan);
        state
            .workspaces
            .edit_plans
            .lock()
            .await
            .insert(plan.id, plan);
        if preferred {
            plans.insert(0, summary);
        } else {
            plans.push(summary);
        }
    }

    Ok(Json(plans))
}

pub(in crate::api) async fn lsp_execute_command_plan(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspExecuteCommandReq>,
) -> Result<Json<crate::edit_plans::EditPlanSummary>, (StatusCode, Json<ApiErrorResp>)> {
    if !state.core.lsp.enabled() || !state.core.lsp_edit_plans_enabled {
        return Err((
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "LSP edit plans disabled".to_string(),
            }),
        ));
    }

    let session_id = req.file.session_id.clone().ok_or((
        StatusCode::BAD_REQUEST,
        Json(ApiErrorResp {
            error: "session_id required".to_string(),
        }),
    ))?;
    let sid = SessionId(uuid::Uuid::parse_str(&session_id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session_id".to_string(),
            }),
        )
    })?);

    let store = state.store_for_session(sid).await.map_err(|_| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        )
    })?;
    let session = store
        .get_session(sid)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load session".to_string(),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        ))?;
    let worktree_id = session.worktree_id;

    let (root, file) = resolve_lsp_target(&state, req.file).await.map_err(|sc| {
        (
            sc,
            Json(ApiErrorResp {
                error: "invalid LSP target".to_string(),
            }),
        )
    })?;

    let (result, edit) = state
        .core
        .lsp
        .execute_command_for_file(
            &root,
            &file,
            req.command.clone(),
            req.arguments.unwrap_or_default(),
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
    let Some(edit) = edit else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: format!("execute_command returned no WorkspaceEdit (result={result})"),
            }),
        ));
    };

    let plan = crate::edit_plans::workspace_edit_to_plan(
        &root,
        &root,
        sid,
        worktree_id,
        format!("Execute command: {}", req.command),
        edit,
    )
    .map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    let summary = plan.to_summary();
    state.persist_edit_plan(&plan);
    state
        .workspaces
        .edit_plans
        .lock()
        .await
        .insert(plan.id, plan);
    Ok(Json(summary))
}

pub(in crate::api) async fn lsp_rename_plan(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspRenamePlanReq>,
) -> Result<Json<crate::edit_plans::EditPlanSummary>, (StatusCode, Json<ApiErrorResp>)> {
    if !state.core.lsp.enabled() || !state.core.lsp_edit_plans_enabled {
        return Err((
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "LSP edit plans disabled".to_string(),
            }),
        ));
    }

    let (sid, _, worktree_id, root, file, _) =
        resolve_session_root_and_file(&state, &req.session_id, &req.path)
            .await
            .map_err(|status| {
                (
                    status,
                    Json(ApiErrorResp {
                        error: "invalid LSP target".to_string(),
                    }),
                )
            })?;
    let edit = state
        .core
        .lsp
        .rename(
            &root,
            &file,
            lsp_types::Position {
                line: req.line,
                character: req.character,
            },
            req.new_name,
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

    let plan = crate::edit_plans::workspace_edit_to_plan(
        &root,
        &root,
        sid,
        worktree_id,
        "Rename".to_string(),
        edit,
    )
    .map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;

    let summary = plan.to_summary();
    state.persist_edit_plan(&plan);
    state
        .workspaces
        .edit_plans
        .lock()
        .await
        .insert(plan.id, plan);
    Ok(Json(summary))
}

pub(in crate::api) async fn lsp_format_plan(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspFormatPlanReq>,
) -> Result<Json<crate::edit_plans::EditPlanSummary>, (StatusCode, Json<ApiErrorResp>)> {
    if !state.core.lsp.enabled() || !state.core.lsp_edit_plans_enabled {
        return Err((
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "LSP edit plans disabled".to_string(),
            }),
        ));
    }

    let (sid, _, worktree_id, root, file, _) =
        resolve_session_root_and_file(&state, &req.session_id, &req.path)
            .await
            .map_err(|status| {
                (
                    status,
                    Json(ApiErrorResp {
                        error: "invalid LSP target".to_string(),
                    }),
                )
            })?;
    let edits = state
        .core
        .lsp
        .format_document(&root, &file)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;

    let rel = file
        .strip_prefix(&root)
        .unwrap_or(&file)
        .to_string_lossy()
        .to_string();
    let plan = crate::edit_plans::text_edits_to_plan(
        &root,
        sid,
        worktree_id,
        "Format document".to_string(),
        rel,
        edits,
    )
    .map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    let summary = plan.to_summary();
    state.persist_edit_plan(&plan);
    state
        .workspaces
        .edit_plans
        .lock()
        .await
        .insert(plan.id, plan);
    Ok(Json(summary))
}

pub(in crate::api) async fn lsp_code_actions_plan(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspCodeActionPlanReq>,
) -> Result<Json<crate::edit_plans::EditPlanSummary>, (StatusCode, Json<ApiErrorResp>)> {
    if !state.core.lsp.enabled() || !state.core.lsp_edit_plans_enabled {
        return Err((
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "LSP edit plans disabled".to_string(),
            }),
        ));
    }

    let (sid, _, worktree_id, root, _) = resolve_session_root(&state, &req.session_id)
        .await
        .map_err(|status| {
            (
                status,
                Json(ApiErrorResp {
                    error: "invalid LSP target".to_string(),
                }),
            )
        })?;

    let action: lsp_types::CodeActionOrCommand = serde_json::from_value(req.action.clone())
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    let (title, edit) = match action {
        lsp_types::CodeActionOrCommand::CodeAction(ca) => {
            let edit = ca.edit.or_else(|| {
                ca.command
                    .as_ref()
                    .and_then(extract_workspace_edit_from_command)
            });
            (ca.title, edit)
        }
        lsp_types::CodeActionOrCommand::Command(cmd) => {
            let edit = extract_workspace_edit_from_command(&cmd);
            (cmd.title, edit)
        }
    };
    let Some(edit) = edit else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "code action has no edit".to_string(),
            }),
        ));
    };

    let plan =
        crate::edit_plans::workspace_edit_to_plan(&root, &root, sid, worktree_id, title, edit)
            .map_err(|e| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&e.to_string()),
                    }),
                )
            })?;
    let summary = plan.to_summary();
    state.persist_edit_plan(&plan);
    state
        .workspaces
        .edit_plans
        .lock()
        .await
        .insert(plan.id, plan);
    Ok(Json(summary))
}

pub(in crate::api) async fn lsp_organize_imports_plan(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspOrganizeImportsPlanReq>,
) -> Result<Json<crate::edit_plans::EditPlanSummary>, (StatusCode, Json<ApiErrorResp>)> {
    if !state.core.lsp.enabled() || !state.core.lsp_edit_plans_enabled {
        return Err((
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "LSP edit plans disabled".to_string(),
            }),
        ));
    }

    let (sid, _, worktree_id, root, file, _) =
        resolve_session_root_and_file(&state, &req.session_id, &req.path)
            .await
            .map_err(|status| {
                (
                    status,
                    Json(ApiErrorResp {
                        error: "invalid LSP target".to_string(),
                    }),
                )
            })?;

    let text = tokio::fs::read_to_string(&file).await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    let (end_line, end_character) = text
        .split('\n')
        .enumerate()
        .fold((0u32, 0u32), |(_l, _c), (i, line)| {
            (i as u32, line.chars().count() as u32)
        });

    let actions = state
        .core
        .lsp
        .code_actions_typed(
            &root,
            &file,
            lsp_types::Range {
                start: lsp_types::Position {
                    line: 0,
                    character: 0,
                },
                end: lsp_types::Position {
                    line: end_line,
                    character: end_character,
                },
            },
            vec![],
            Some(vec![lsp_types::CodeActionKind::SOURCE_ORGANIZE_IMPORTS]),
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

    let edit = actions.into_iter().find_map(|a| match a {
        lsp_types::CodeActionOrCommand::CodeAction(ca) => ca.edit.or_else(|| {
            ca.command
                .as_ref()
                .and_then(extract_workspace_edit_from_command)
        }),
        lsp_types::CodeActionOrCommand::Command(cmd) => extract_workspace_edit_from_command(&cmd),
    });
    let Some(edit) = edit else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "no organize-imports edit returned".to_string(),
            }),
        ));
    };

    let plan = crate::edit_plans::workspace_edit_to_plan(
        &root,
        &root,
        sid,
        worktree_id,
        "Organize imports".to_string(),
        edit,
    )
    .map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    let summary = plan.to_summary();
    state.persist_edit_plan(&plan);
    state
        .workspaces
        .edit_plans
        .lock()
        .await
        .insert(plan.id, plan);
    Ok(Json(summary))
}
