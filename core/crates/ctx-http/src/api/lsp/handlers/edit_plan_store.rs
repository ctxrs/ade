use super::super::*;

pub(in crate::api) async fn list_edit_plans_for_worktree(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<crate::edit_plans::EditPlanSummary>>, StatusCode> {
    let worktree_id = WorktreeId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let map = state.workspaces.edit_plans.lock().await;
    let mut out = map
        .values()
        .filter(|p| p.worktree_id == worktree_id)
        .map(|p| p.to_summary())
        .collect::<Vec<_>>();
    out.sort_by_key(|summary| std::cmp::Reverse(summary.created_at));
    Ok(Json(out))
}

pub(in crate::api) async fn get_edit_plan(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<crate::edit_plans::EditPlanSummary>, StatusCode> {
    let pid = crate::edit_plans::EditPlanId(
        uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?,
    );
    let map = state.workspaces.edit_plans.lock().await;
    let Some(plan) = map.get(&pid) else {
        return Err(StatusCode::NOT_FOUND);
    };
    Ok(Json(plan.to_summary()))
}

pub(in crate::api) async fn apply_edit_plan_patch(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<EditPlanApplyReq>,
) -> Result<Json<crate::edit_plans::EditPlanSummary>, (StatusCode, Json<ApiErrorResp>)> {
    if !state.core.lsp_edit_plans_enabled {
        return Err((
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "LSP edit plans disabled".to_string(),
            }),
        ));
    }
    let pid = crate::edit_plans::EditPlanId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid edit plan id".to_string(),
            }),
        )
    })?);
    if req.patch.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "patch is empty".to_string(),
            }),
        ));
    }

    let action = req.action.trim().to_lowercase();
    let patch = req.patch;

    match action.as_str() {
        "accept" => {
            let parsed = crate::edit_plans::parse_unified_diff(&patch);

            let (worktree_root, plan_files) = {
                let map = state.workspaces.edit_plans.lock().await;
                let Some(plan) = map.get(&pid) else {
                    return Err((
                        StatusCode::NOT_FOUND,
                        Json(ApiErrorResp {
                            error: "edit plan not found".to_string(),
                        }),
                    ));
                };
                (plan.worktree_root.clone(), plan.files.clone())
            };

            for pf in &parsed {
                let rel = if !pf.new_path.is_empty() {
                    &pf.new_path
                } else {
                    &pf.old_path
                };
                let Some(base) = plan_files
                    .iter()
                    .find(|f| {
                        plan_paths_match(&pf.old_path, &pf.new_path, &f.old_path, &f.new_path)
                    })
                    .map(|f| f.base_sha256.clone())
                else {
                    continue;
                };
                if base.trim().is_empty() {
                    continue;
                }
                let abs = worktree_root.join(rel);
                let current = tokio::fs::read_to_string(&abs).await.unwrap_or_default();
                let current_sha = sha256_hex(&current);
                if current_sha != base {
                    return Err((
                        StatusCode::CONFLICT,
                        Json(ApiErrorResp {
                            error: format!("edit plan is stale for {rel}; regenerate the plan"),
                        }),
                    ));
                }
            }

            let worktree_root = {
                let map = state.workspaces.edit_plans.lock().await;
                let Some(plan) = map.get(&pid) else {
                    return Err((
                        StatusCode::NOT_FOUND,
                        Json(ApiErrorResp {
                            error: "edit plan not found".to_string(),
                        }),
                    ));
                };
                plan.worktree_root.clone()
            };

            ctx_fs::git::git_apply_patch(
                worktree_root.to_string_lossy().as_ref(),
                &patch,
                ctx_fs::git::ApplyPatchTarget::Worktree,
                false,
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

            let mut updated_bases: Vec<(String, String)> = Vec::new();
            for pf in &parsed {
                let rel = if !pf.new_path.is_empty() {
                    pf.new_path.clone()
                } else {
                    pf.old_path.clone()
                };
                let abs = worktree_root.join(&rel);
                let current = tokio::fs::read_to_string(&abs).await.unwrap_or_default();
                updated_bases.push((rel, sha256_hex(&current)));
            }

            let (summary, to_persist, removed) = {
                let mut map = state.workspaces.edit_plans.lock().await;
                let Some(plan) = map.get_mut(&pid) else {
                    return Err((
                        StatusCode::NOT_FOUND,
                        Json(ApiErrorResp {
                            error: "edit plan not found".to_string(),
                        }),
                    ));
                };
                plan.remove_patch(&patch);
                for (rel, sha) in &updated_bases {
                    for f in &mut plan.files {
                        let f_rel = if !f.new_path.is_empty() {
                            &f.new_path
                        } else {
                            &f.old_path
                        };
                        if f_rel == rel {
                            f.base_sha256 = sha.clone();
                        }
                    }
                }
                let summary = plan.to_summary();
                let removed = plan.files.is_empty();
                let to_persist = if removed { None } else { Some(plan.clone()) };
                if removed {
                    map.remove(&pid);
                }
                (summary, to_persist, removed)
            };
            if let Some(plan) = to_persist {
                state.persist_edit_plan(&plan);
            } else if removed {
                state.delete_edit_plan_file(pid);
            }
            Ok(Json(summary))
        }
        "reject" => {
            let (summary, to_persist, removed) = {
                let mut map = state.workspaces.edit_plans.lock().await;
                let Some(plan) = map.get_mut(&pid) else {
                    return Err((
                        StatusCode::NOT_FOUND,
                        Json(ApiErrorResp {
                            error: "edit plan not found".to_string(),
                        }),
                    ));
                };
                plan.remove_patch(&patch);
                let summary = plan.to_summary();
                let removed = plan.files.is_empty();
                let to_persist = if removed { None } else { Some(plan.clone()) };
                if removed {
                    map.remove(&pid);
                }
                (summary, to_persist, removed)
            };
            if let Some(plan) = to_persist {
                state.persist_edit_plan(&plan);
            } else if removed {
                state.delete_edit_plan_file(pid);
            }
            Ok(Json(summary))
        }
        _ => Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "action must be accept or reject".to_string(),
            }),
        )),
    }
}

pub(in crate::api) async fn discard_edit_plan(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let pid = crate::edit_plans::EditPlanId(
        uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?,
    );
    state.workspaces.edit_plans.lock().await.remove(&pid);
    state.delete_edit_plan_file(pid);
    Ok(StatusCode::NO_CONTENT)
}
