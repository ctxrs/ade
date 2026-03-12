use super::*;

pub(crate) async fn mcp_agent_reply(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<AgentReplyReq>,
) -> Result<Json<AgentReplyResp>, (StatusCode, Json<ApiErrorResp>)> {
    let parent_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);

    let store = state.store_for_session(parent_id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    let parent = store
        .get_session(parent_id)
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
                error: "parent session not found".to_string(),
            }),
        ))?;

    let label = req.label.trim();
    if label.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "label is required".to_string(),
            }),
        ));
    }
    let child = store
        .get_subagent_session_by_label(parent.id, label)
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
                error: "subagent label not found".to_string(),
            }),
        ))?;

    let prompt = req.prompt.trim().to_string();
    if prompt.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "prompt is required".to_string(),
            }),
        ));
    }

    if store
        .get_running_turn_for_session(child.id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .is_some()
    {
        return Err((
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "subagent_busy: The subagent is still running. Please await it with subagent_wait or interrupt it with subagent_interrupt.".to_string(),
            }),
        ));
    }

    let mut context_window =
        estimate_context_window_for_prompt(&child.provider_id, &child.model_id, &prompt);
    if context_window.is_none() {
        context_window = context_window_for_session(&state, child.id).await;
    }

    let (_run_id, _message) = enqueue_subagent_prompt(&state, &child, prompt).await?;
    let worktree_path = worktree_path_for_child(&state, parent.worktree_id, child.id).await;

    Ok(Json(AgentReplyResp {
        label: label.to_string(),
        status: "running".to_string(),
        context_window,
        worktree_path,
    }))
}

pub(crate) async fn mcp_subagent_list(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<SubagentListItem>>, (StatusCode, Json<ApiErrorResp>)> {
    let parent_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);

    let store = state.store_for_session(parent_id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    let parent = store
        .get_session(parent_id)
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
                error: "parent session not found".to_string(),
            }),
        ))?;

    let subs = store.list_subagent_sessions(parent.id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;

    let mut results = Vec::with_capacity(subs.len());
    for sub in subs {
        let label = sub.title.trim().to_string();
        let status = match sub.status {
            SessionStatus::Active => "active",
            SessionStatus::Completed => "completed",
            SessionStatus::Failed => "failed",
            SessionStatus::Cancelled => "cancelled",
        }
        .to_string();
        let context_window = context_window_for_session(&state, sub.id).await;
        let worktree_path = worktree_path_for_child(&state, parent.worktree_id, sub.id).await;
        results.push(SubagentListItem {
            label,
            status,
            context_window,
            worktree_path,
        });
    }

    Ok(Json(results))
}

pub(crate) async fn mcp_subagent_interrupt(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<SubagentInterruptReq>,
) -> Result<Json<SubagentInterruptResp>, (StatusCode, Json<ApiErrorResp>)> {
    let parent_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);

    let store = state.store_for_session(parent_id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    let parent = store
        .get_session(parent_id)
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
                error: "parent session not found".to_string(),
            }),
        ))?;

    let use_all = req.all.unwrap_or(false);
    let labels = match (use_all, req.label) {
        (true, Some(_)) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "provide either label or all".to_string(),
                }),
            ));
        }
        (true, None) => {
            let subs = store.list_subagent_sessions(parent.id).await.map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&e.to_string()),
                    }),
                )
            })?;
            subs.into_iter()
                .map(|sub| sub.title.trim().to_string())
                .collect::<Vec<_>>()
        }
        (false, Some(label)) => vec![label],
        (false, None) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "label or all is required".to_string(),
                }),
            ));
        }
    };

    let mut results = Vec::with_capacity(labels.len());
    for label in labels {
        let trimmed = label.trim();
        if trimmed.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "label cannot be empty".to_string(),
                }),
            ));
        }
        let child = store
            .get_subagent_session_by_label(parent.id, trimmed)
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
                    error: format!("subagent label '{trimmed}' not found"),
                }),
            ))?;

        let tx = state.ensure_scheduler(child.clone()).await;
        let _ = tx.send(SchedulerCommand::Interrupt).await;

        let context_window = context_window_for_session(&state, child.id).await;
        results.push(
            build_subagent_result_for_session(
                &state,
                parent.worktree_id,
                &child,
                trimmed.to_string(),
                "interrupt_requested".to_string(),
                None,
                context_window,
            )
            .await
            .map_err(|error| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp { error }),
                )
            })?,
        );
    }

    Ok(Json(SubagentInterruptResp {
        status: "interrupt_requested".to_string(),
        results,
    }))
}

pub(crate) async fn mcp_oracle(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<oracle::OracleRequest>,
) -> Result<Json<oracle::OracleResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);

    let workspace_id = state
        .global_store()
        .get_workspace_id_for_session(session_id)
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
                error: "session not found".to_string(),
            }),
        ))?;

    let store = state.store_for_workspace(workspace_id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    store
        .get_session(session_id)
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
                error: "session not found".to_string(),
            }),
        ))?;

    let prompt = req.prompt.trim();
    if prompt.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "prompt is required".to_string(),
            }),
        ));
    }

    let settings = user_settings::load_settings(state.global_store())
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    let cfg = settings.oracle.as_ref().ok_or((
        StatusCode::BAD_REQUEST,
        Json(ApiErrorResp {
            error: "oracle is not configured".to_string(),
        }),
    ))?;

    let resp = oracle::oracle_one_shot(cfg, req).await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;

    Ok(Json(resp))
}

pub(crate) async fn mcp_subagent_wait(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<SubagentWaitReq>,
) -> Result<Json<SubagentWaitResp>, (StatusCode, Json<ApiErrorResp>)> {
    let parent_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);

    let store = state.store_for_session(parent_id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    let parent = store
        .get_session(parent_id)
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
                error: "parent session not found".to_string(),
            }),
        ))?;

    let mut labels = match (req.label, req.labels) {
        (Some(label), None) => vec![label],
        (None, Some(labels)) => labels,
        (Some(_), Some(_)) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "provide either label or labels".to_string(),
                }),
            ));
        }
        (None, None) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "label or labels is required".to_string(),
                }),
            ));
        }
    };
    if labels.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "labels is required".to_string(),
            }),
        ));
    }
    let mut seen = HashSet::new();
    for label in labels.iter_mut() {
        let trimmed = label.trim().to_string();
        if trimmed.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "label cannot be empty".to_string(),
                }),
            ));
        }
        if !seen.insert(trimmed.clone()) {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: format!("duplicate label '{trimmed}'"),
                }),
            ));
        }
        *label = trimmed;
    }

    let mut results = Vec::with_capacity(labels.len());
    for label in labels {
        let child = store
            .get_subagent_session_by_label(parent.id, &label)
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
                    error: format!("subagent label '{label}' not found"),
                }),
            ))?;

        let running_turn = store
            .get_running_turn_for_session(child.id)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&e.to_string()),
                    }),
                )
            })?;

        let (status, run_id) = if let Some(turn) = running_turn {
            let run_id = turn.run_id.ok_or((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "subagent run_id missing; cannot wait".to_string(),
                }),
            ))?;
            let terminal = wait_for_run_terminal_event(&state, child.id, run_id).await;
            let status = match terminal {
                Ok(SessionEventType::Done) | Ok(SessionEventType::TurnFinished) => "completed",
                Ok(SessionEventType::TurnInterrupted) => "interrupted",
                Ok(SessionEventType::Error) => "failed",
                Ok(_) => "completed",
                Err(_) => "unknown",
            }
            .to_string();
            (status, Some(run_id))
        } else if let Some(turn) =
            store
                .get_latest_turn_for_session(child.id)
                .await
                .map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiErrorResp {
                            error: logs::redact_sensitive(&e.to_string()),
                        }),
                    )
                })?
        {
            let status = match turn.status {
                SessionTurnStatus::Completed => "completed",
                SessionTurnStatus::Interrupted => "interrupted",
                SessionTurnStatus::Failed => "failed",
                SessionTurnStatus::Running | SessionTurnStatus::Queued => "running",
            }
            .to_string();
            (status, turn.run_id)
        } else {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: format!("subagent '{label}' has no runs to wait for"),
                }),
            ));
        };

        let content = match run_id {
            Some(run_id) => store
                .get_last_assistant_message_for_run(child.id, run_id)
                .await
                .ok()
                .flatten()
                .map(|m| m.content),
            None => None,
        };
        let context_window = match run_id {
            Some(run_id) => context_window_for_run(&state, child.id, run_id).await,
            None => context_window_for_session(&state, child.id).await,
        };

        results.push(
            build_subagent_result_for_session(
                &state,
                parent.worktree_id,
                &child,
                label,
                status,
                content,
                context_window,
            )
            .await
            .map_err(|error| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp { error }),
                )
            })?,
        );
    }

    let status = aggregate_subagent_status(&results);

    Ok(Json(SubagentWaitResp {
        status: status.to_string(),
        results,
    }))
}
