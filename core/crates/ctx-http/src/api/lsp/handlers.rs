use super::*;

mod edit_plan_creation;
mod edit_plan_store;

pub(in crate::api) use edit_plan_creation::*;
pub(in crate::api) use edit_plan_store::*;

pub(in crate::api) async fn lsp_definition(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspPosReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .definition(
            &root,
            &file,
            lsp_types::Position {
                line: req.line,
                character: req.character,
            },
        )
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(in crate::api) async fn lsp_type_definition(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspPosReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .type_definition(
            &root,
            &file,
            lsp_types::Position {
                line: req.line,
                character: req.character,
            },
        )
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(in crate::api) async fn lsp_implementation(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspPosReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .implementation(
            &root,
            &file,
            lsp_types::Position {
                line: req.line,
                character: req.character,
            },
        )
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(in crate::api) async fn lsp_references(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspRefsReq>,
) -> Result<Json<Vec<serde_json::Value>>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.pos.file).await?;
    let out = state
        .core
        .lsp
        .references(
            &root,
            &file,
            lsp_types::Position {
                line: req.pos.line,
                character: req.pos.character,
            },
            req.include_declaration.unwrap_or(true),
        )
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .into_iter()
        .filter_map(|l| serde_json::to_value(l).ok())
        .collect::<Vec<_>>();
    Ok(Json(out))
}

pub(in crate::api) async fn lsp_hover(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspPosReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .hover(
            &root,
            &file,
            lsp_types::Position {
                line: req.line,
                character: req.character,
            },
        )
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(in crate::api) async fn lsp_signature_help(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspPosReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .signature_help(
            &root,
            &file,
            lsp_types::Position {
                line: req.line,
                character: req.character,
            },
        )
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(in crate::api) async fn lsp_completion(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspPosReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .completion(
            &root,
            &file,
            lsp_types::Position {
                line: req.line,
                character: req.character,
            },
        )
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(in crate::api) async fn lsp_completion_resolve(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspResolveReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .completion_resolve(&root, &file, req.item)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(in crate::api) async fn lsp_code_action_resolve(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspResolveReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .code_action_resolve(&root, &file, req.item)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(in crate::api) async fn lsp_inlay_hints(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspRangeReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .inlay_hints(
            &root,
            &file,
            lsp_types::Range {
                start: lsp_types::Position {
                    line: req.start_line,
                    character: req.start_character,
                },
                end: lsp_types::Position {
                    line: req.end_line,
                    character: req.end_character,
                },
            },
        )
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(in crate::api) async fn lsp_document_highlight(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspPosReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .document_highlight(
            &root,
            &file,
            lsp_types::Position {
                line: req.line,
                character: req.character,
            },
        )
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(in crate::api) async fn lsp_selection_ranges(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspSelectionRangesReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let positions = req
        .positions
        .into_iter()
        .map(|p| lsp_types::Position {
            line: p.line,
            character: p.character,
        })
        .collect::<Vec<_>>();
    let v = state
        .core
        .lsp
        .selection_ranges(&root, &file, positions)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(in crate::api) async fn lsp_call_hierarchy_prepare(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspPosReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .call_hierarchy_prepare(
            &root,
            &file,
            lsp_types::Position {
                line: req.line,
                character: req.character,
            },
        )
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(in crate::api) async fn lsp_call_hierarchy_incoming(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspResolveReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .call_hierarchy_incoming(&root, &file, req.item)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(in crate::api) async fn lsp_call_hierarchy_outgoing(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspResolveReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .call_hierarchy_outgoing(&root, &file, req.item)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(in crate::api) async fn lsp_code_lens(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspFileReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req).await?;
    let v = state
        .core
        .lsp
        .code_lens(&root, &file)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(in crate::api) async fn lsp_code_lens_resolve(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspResolveReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .code_lens_resolve(&root, &file, req.item)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(in crate::api) async fn lsp_prepare_rename(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspPosReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .prepare_rename(
            &root,
            &file,
            lsp_types::Position {
                line: req.line,
                character: req.character,
            },
        )
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(in crate::api) async fn lsp_document_links(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspFileReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req).await?;
    let v = state
        .core
        .lsp
        .document_links(&root, &file)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(in crate::api) async fn lsp_document_link_resolve(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspResolveReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .document_link_resolve(&root, &file, req.item)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(in crate::api) async fn lsp_semantic_tokens_full(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspFileReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req).await?;
    let v = state
        .core
        .lsp
        .semantic_tokens_full(&root, &file)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(in crate::api) async fn lsp_semantic_tokens_delta(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspSemanticTokensDeltaReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .semantic_tokens_delta(&root, &file, req.previous_result_id)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(in crate::api) async fn lsp_folding_ranges(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspFileReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req).await?;
    let v = state
        .core
        .lsp
        .folding_ranges(&root, &file)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(in crate::api) async fn lsp_linked_editing_range(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspPosReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .linked_editing_range(
            &root,
            &file,
            lsp_types::Position {
                line: req.line,
                character: req.character,
            },
        )
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(in crate::api) async fn lsp_type_hierarchy_prepare(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspPosReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .type_hierarchy_prepare(
            &root,
            &file,
            lsp_types::Position {
                line: req.line,
                character: req.character,
            },
        )
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(in crate::api) async fn lsp_type_hierarchy_supertypes(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspResolveReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .type_hierarchy_supertypes(&root, &file, req.item)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(in crate::api) async fn lsp_type_hierarchy_subtypes(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspResolveReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .type_hierarchy_subtypes(&root, &file, req.item)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(in crate::api) async fn lsp_execute_command(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspExecuteCommandReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let (result, edit) = state
        .core
        .lsp
        .execute_command_for_file(&root, &file, req.command, req.arguments.unwrap_or_default())
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let edit = edit
        .and_then(|e| serde_json::to_value(e).ok())
        .unwrap_or(serde_json::Value::Null);
    Ok(Json(serde_json::json!({
        "result": result,
        "workspace_edit": edit
    })))
}

pub(in crate::api) async fn lsp_document_symbols(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspFileReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req).await?;
    let v = state
        .core
        .lsp
        .document_symbols(&root, &file)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(in crate::api) async fn lsp_workspace_symbols(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspWorkspaceSymbolsReq>,
) -> Result<Json<Vec<serde_json::Value>>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }

    let root =
        resolve_lsp_root(&state, req.session_id.as_deref(), req.root_path.as_deref()).await?;

    let out = state
        .core
        .lsp
        .workspace_symbols(&root, req.query)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .into_iter()
        .filter_map(|s| serde_json::to_value(s).ok())
        .collect::<Vec<_>>();
    Ok(Json(out))
}

pub(in crate::api) async fn lsp_workspace_symbol_resolve(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspWorkspaceSymbolResolveReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }

    let root =
        resolve_lsp_root(&state, req.session_id.as_deref(), req.root_path.as_deref()).await?;

    let out = state
        .core
        .lsp
        .workspace_symbol_resolve(&root, req.item)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(out))
}

pub(in crate::api) async fn lsp_code_actions(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspCodeActionsReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .code_actions(
            &root,
            &file,
            lsp_types::Range {
                start: lsp_types::Position {
                    line: req.start_line,
                    character: req.start_character,
                },
                end: lsp_types::Position {
                    line: req.end_line,
                    character: req.end_character,
                },
            },
            vec![],
        )
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}
