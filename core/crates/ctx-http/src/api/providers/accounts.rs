use super::*;

pub(crate) async fn codex_accounts_response(state: &Arc<AppState>) -> CodexAccountsResponse {
    let registry = provider_accounts::load_codex_registry(&state.core.data_root).await;
    let logins = {
        let map = state.providers.codex_login_sessions.lock().await;
        map.values().cloned().collect::<Vec<_>>()
    };
    CodexAccountsResponse {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
        logins,
    }
}

pub(crate) async fn claude_accounts_response(state: &Arc<AppState>) -> ClaudeAccountsResponse {
    let registry = provider_accounts::load_claude_registry(&state.core.data_root).await;
    ClaudeAccountsResponse {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
    }
}

pub(crate) async fn gemini_accounts_response(state: &Arc<AppState>) -> GeminiAccountsResponse {
    let registry = provider_accounts::load_gemini_registry(&state.core.data_root).await;
    GeminiAccountsResponse {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
    }
}

pub(crate) async fn qwen_accounts_response(state: &Arc<AppState>) -> QwenAccountsResponse {
    let registry = provider_accounts::load_qwen_registry(&state.core.data_root).await;
    QwenAccountsResponse {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
    }
}

pub(crate) async fn kimi_accounts_response(state: &Arc<AppState>) -> KimiAccountsResponse {
    let registry = provider_accounts::load_kimi_registry(&state.core.data_root).await;
    KimiAccountsResponse {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
    }
}

pub(crate) async fn mistral_accounts_response(state: &Arc<AppState>) -> MistralAccountsResponse {
    let registry = provider_accounts::load_mistral_registry(&state.core.data_root).await;
    MistralAccountsResponse {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
    }
}

pub(crate) async fn copilot_accounts_response(state: &Arc<AppState>) -> CopilotAccountsResponse {
    let registry = provider_accounts::load_copilot_registry(&state.core.data_root).await;
    CopilotAccountsResponse {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
    }
}

pub(crate) async fn cursor_accounts_response(state: &Arc<AppState>) -> CursorAccountsResponse {
    let registry = provider_accounts::load_cursor_registry(&state.core.data_root).await;
    CursorAccountsResponse {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
    }
}

pub(crate) async fn amp_accounts_response(
    state: &Arc<AppState>,
) -> anyhow::Result<AmpAccountsResponse> {
    let registry =
        provider_accounts::ensure_amp_registry_from_runtime_auth(&state.core.data_root).await?;
    Ok(AmpAccountsResponse {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
    })
}

pub(crate) async fn list_codex_accounts(
    State(state): State<Arc<AppState>>,
) -> Result<Json<CodexAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    Ok(Json(codex_accounts_response(&state).await))
}

pub(crate) async fn probe_host_codex_import(
    State(_state): State<Arc<AppState>>,
) -> Result<Json<provider_accounts::CodexHostImportProbe>, (StatusCode, Json<ApiErrorResp>)> {
    Ok(Json(
        provider_accounts::probe_host_codex_auth_candidate().await,
    ))
}

pub(crate) async fn import_host_codex_auth(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CodexHostImportReq>,
) -> Result<Json<CodexAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::import_host_codex_auth_to_secret_store(&state.core.data_root, req.label)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restarts::restart_codex_providers_for_auth_change(&state, "codex auth updated").await;
    Ok(Json(codex_accounts_response(&state).await))
}

pub(crate) async fn get_codex_accounts_usage(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ProviderUsageQuery>,
) -> Result<Json<CodexAccountsUsageResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let refresh = query.refresh.unwrap_or(false);
    let registry = provider_accounts::load_codex_registry(&state.core.data_root).await;
    let active_id = registry.active_account_id.clone();
    let cached_active = if !refresh {
        let cache = state.providers.usage_cache.lock().await;
        cache.get("codex-crp").cloned()
    } else {
        None
    };

    let to_err = |e: anyhow::Error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: e.to_string(),
            }),
        )
    };

    let mut entries = Vec::new();
    let cfg = crate::installer::load_agent_server_config(&state.core.data_root)
        .await
        .unwrap_or_default();

    for account in registry.accounts {
        let _ = provider_accounts::hydrate_codex_account_home_from_secret(
            &state.core.data_root,
            &account.id,
        )
        .await;
        let mut env = provider_accounts::codex_env_for_account(&state.core.data_root, &account.id);
        crate::installer::ensure_codex_cli_command_env_for_target(
            &mut env,
            &cfg,
            "codex-crp",
            Some(ctx_provider_install::install_state::InstallTarget::Host),
        )
        .map_err(to_err)?;
        let usage = if active_id.as_deref() == Some(&account.id) {
            if let Some(snapshot) = cached_active.clone() {
                snapshot
            } else {
                provider_usage::fetch_codex_usage_snapshot(env)
                    .await
                    .map_err(to_err)?
            }
        } else {
            provider_usage::fetch_codex_usage_snapshot(env)
                .await
                .map_err(to_err)?
        };
        entries.push(CodexAccountUsageEntry {
            account_id: Some(account.id),
            label: account.label,
            email: account.email,
            plan_type: account.plan_type,
            last_used_at: account.last_used_at,
            usage,
        });
    }

    Ok(Json(CodexAccountsUsageResponse { entries }))
}

pub(crate) async fn set_codex_active_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CodexActiveAccountReq>,
) -> Result<Json<CodexAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    if let Some(ref account_id) = req.account_id {
        let registry = provider_accounts::load_codex_registry(&state.core.data_root).await;
        if !registry.accounts.iter().any(|a| a.id == *account_id) {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "unknown account".to_string(),
                }),
            ));
        }
    }
    let registry =
        provider_accounts::set_active_codex_account(&state.core.data_root, req.account_id)
            .await
            .map_err(|e| {
                let msg = e.to_string();
                let status = if msg.contains("api_shape=openai_responses")
                    || msg.contains("auth_type=bearer")
                    || msg.contains("unknown account")
                {
                    StatusCode::BAD_REQUEST
                } else {
                    StatusCode::INTERNAL_SERVER_ERROR
                };
                (status, Json(ApiErrorResp { error: msg }))
            })?;
    restarts::restart_codex_providers_for_auth_change(&state, "codex auth updated").await;
    let logins = {
        let map = state.providers.codex_login_sessions.lock().await;
        map.values().cloned().collect::<Vec<_>>()
    };
    Ok(Json(CodexAccountsResponse {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
        logins,
    }))
}

pub(crate) async fn delete_codex_account(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<CodexAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let registry = provider_accounts::remove_codex_account(&state.core.data_root, &id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restarts::restart_codex_providers_for_auth_change(&state, "codex auth updated").await;
    let logins = {
        let mut map = state.providers.codex_login_sessions.lock().await;
        map.remove(&id);
        map.values().cloned().collect::<Vec<_>>()
    };
    Ok(Json(CodexAccountsResponse {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
        logins,
    }))
}

pub(crate) async fn list_claude_accounts(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ClaudeAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    Ok(Json(claude_accounts_response(&state).await))
}

pub(crate) async fn upsert_claude_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ClaudeAccountUpsertReq>,
) -> Result<Json<ClaudeAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::add_claude_account(&state.core.data_root, req.label, req.setup_token)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restarts::restart_claude_providers_for_auth_change(&state, "claude auth updated").await;
    Ok(Json(claude_accounts_response(&state).await))
}

pub(crate) async fn set_claude_active_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ClaudeActiveAccountReq>,
) -> Result<Json<ClaudeAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    if let Some(ref account_id) = req.account_id {
        let registry = provider_accounts::load_claude_registry(&state.core.data_root).await;
        if !registry.accounts.iter().any(|a| a.id == *account_id) {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "unknown account".to_string(),
                }),
            ));
        }
    }
    provider_accounts::set_active_claude_account(&state.core.data_root, req.account_id)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restarts::restart_claude_providers_for_auth_change(&state, "claude auth updated").await;
    Ok(Json(claude_accounts_response(&state).await))
}

pub(crate) async fn delete_claude_account(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<ClaudeAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::remove_claude_account(&state.core.data_root, &id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restarts::restart_claude_providers_for_auth_change(&state, "claude auth updated").await;
    Ok(Json(claude_accounts_response(&state).await))
}

pub(crate) async fn list_amp_accounts(
    State(state): State<Arc<AppState>>,
) -> Result<Json<AmpAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let response = amp_accounts_response(&state).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: e.to_string(),
            }),
        )
    })?;
    Ok(Json(response))
}

pub(crate) async fn upsert_amp_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AmpAccountUpsertReq>,
) -> Result<Json<AmpAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::upsert_amp_account(&state.core.data_root, req.label, req.email)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restarts::restart_amp_providers_for_auth_change(&state, "amp auth updated").await;
    Ok(Json(amp_accounts_response(&state).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: e.to_string(),
            }),
        )
    })?))
}

pub(crate) async fn set_amp_active_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AmpActiveAccountReq>,
) -> Result<Json<AmpAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    if let Some(ref account_id) = req.account_id {
        let registry = provider_accounts::load_amp_registry(&state.core.data_root).await;
        if !registry.accounts.iter().any(|a| a.id == *account_id) {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "unknown account".to_string(),
                }),
            ));
        }
    }
    provider_accounts::set_active_amp_account(&state.core.data_root, req.account_id)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restarts::restart_amp_providers_for_auth_change(&state, "amp auth updated").await;
    let response = amp_accounts_response(&state).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: e.to_string(),
            }),
        )
    })?;
    Ok(Json(response))
}

pub(crate) async fn delete_amp_account(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<AmpAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::remove_amp_account(&state.core.data_root, &id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restarts::restart_amp_providers_for_auth_change(&state, "amp auth updated").await;
    let response = amp_accounts_response(&state).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: e.to_string(),
            }),
        )
    })?;
    Ok(Json(response))
}

pub(crate) async fn list_gemini_accounts(
    State(state): State<Arc<AppState>>,
) -> Result<Json<GeminiAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    Ok(Json(gemini_accounts_response(&state).await))
}

pub(crate) async fn upsert_gemini_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<GeminiAccountUpsertReq>,
) -> Result<Json<GeminiAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::add_gemini_account(
        &state.core.data_root,
        req.label,
        req.oauth_creds_json,
        req.google_accounts_json,
        req.email,
    )
    .await
    .map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: e.to_string(),
            }),
        )
    })?;
    restarts::restart_gemini_providers_for_auth_change(&state, "gemini auth updated").await;
    Ok(Json(gemini_accounts_response(&state).await))
}

pub(crate) async fn set_gemini_active_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<GeminiActiveAccountReq>,
) -> Result<Json<GeminiAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    if let Some(ref account_id) = req.account_id {
        let registry = provider_accounts::load_gemini_registry(&state.core.data_root).await;
        if !registry.accounts.iter().any(|a| a.id == *account_id) {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "unknown account".to_string(),
                }),
            ));
        }
    }
    provider_accounts::set_active_gemini_account(&state.core.data_root, req.account_id)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restarts::restart_gemini_providers_for_auth_change(&state, "gemini auth updated").await;
    Ok(Json(gemini_accounts_response(&state).await))
}

pub(crate) async fn delete_gemini_account(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<GeminiAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::remove_gemini_account(&state.core.data_root, &id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restarts::restart_gemini_providers_for_auth_change(&state, "gemini auth updated").await;
    Ok(Json(gemini_accounts_response(&state).await))
}

pub(crate) async fn list_qwen_accounts(
    State(state): State<Arc<AppState>>,
) -> Result<Json<QwenAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    Ok(Json(qwen_accounts_response(&state).await))
}

pub(crate) async fn upsert_qwen_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<QwenAccountUpsertReq>,
) -> Result<Json<QwenAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::add_qwen_account(
        &state.core.data_root,
        req.label,
        req.oauth_creds_json,
        req.email,
    )
    .await
    .map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: e.to_string(),
            }),
        )
    })?;
    restarts::restart_qwen_providers_for_auth_change(&state, "qwen auth updated").await;
    Ok(Json(qwen_accounts_response(&state).await))
}

pub(crate) async fn set_qwen_active_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<QwenActiveAccountReq>,
) -> Result<Json<QwenAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    if let Some(ref account_id) = req.account_id {
        let registry = provider_accounts::load_qwen_registry(&state.core.data_root).await;
        if !registry.accounts.iter().any(|a| a.id == *account_id) {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "unknown account".to_string(),
                }),
            ));
        }
    }
    provider_accounts::set_active_qwen_account(&state.core.data_root, req.account_id)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restarts::restart_qwen_providers_for_auth_change(&state, "qwen auth updated").await;
    Ok(Json(qwen_accounts_response(&state).await))
}

pub(crate) async fn delete_qwen_account(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<QwenAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::remove_qwen_account(&state.core.data_root, &id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restarts::restart_qwen_providers_for_auth_change(&state, "qwen auth updated").await;
    Ok(Json(qwen_accounts_response(&state).await))
}

pub(crate) async fn list_kimi_accounts(
    State(state): State<Arc<AppState>>,
) -> Result<Json<KimiAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    Ok(Json(kimi_accounts_response(&state).await))
}

pub(crate) async fn upsert_kimi_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<KimiAccountUpsertReq>,
) -> Result<Json<KimiAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::add_kimi_account(
        &state.core.data_root,
        req.label,
        req.provider,
        req.credentials_json,
        req.config_toml,
        req.email,
    )
    .await
    .map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: e.to_string(),
            }),
        )
    })?;
    restarts::restart_kimi_providers_for_auth_change(&state, "kimi auth updated").await;
    Ok(Json(kimi_accounts_response(&state).await))
}

pub(crate) async fn set_kimi_active_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<KimiActiveAccountReq>,
) -> Result<Json<KimiAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    if let Some(ref account_id) = req.account_id {
        let registry = provider_accounts::load_kimi_registry(&state.core.data_root).await;
        if !registry.accounts.iter().any(|a| a.id == *account_id) {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "unknown account".to_string(),
                }),
            ));
        }
    }
    provider_accounts::set_active_kimi_account(&state.core.data_root, req.account_id)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restarts::restart_kimi_providers_for_auth_change(&state, "kimi auth updated").await;
    Ok(Json(kimi_accounts_response(&state).await))
}

pub(crate) async fn delete_kimi_account(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<KimiAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::remove_kimi_account(&state.core.data_root, &id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restarts::restart_kimi_providers_for_auth_change(&state, "kimi auth updated").await;
    Ok(Json(kimi_accounts_response(&state).await))
}

pub(crate) async fn list_mistral_accounts(
    State(state): State<Arc<AppState>>,
) -> Result<Json<MistralAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    Ok(Json(mistral_accounts_response(&state).await))
}

pub(crate) async fn upsert_mistral_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<MistralAccountUpsertReq>,
) -> Result<Json<MistralAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::upsert_mistral_account(&state.core.data_root, req.label, req.email)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restarts::restart_mistral_providers_for_auth_change(&state, "mistral auth updated").await;
    Ok(Json(mistral_accounts_response(&state).await))
}

pub(crate) async fn set_mistral_active_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<MistralActiveAccountReq>,
) -> Result<Json<MistralAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    if let Some(ref account_id) = req.account_id {
        let registry = provider_accounts::load_mistral_registry(&state.core.data_root).await;
        if !registry.accounts.iter().any(|a| a.id == *account_id) {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "unknown account".to_string(),
                }),
            ));
        }
    }
    provider_accounts::set_active_mistral_account(&state.core.data_root, req.account_id)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restarts::restart_mistral_providers_for_auth_change(&state, "mistral auth updated").await;
    Ok(Json(mistral_accounts_response(&state).await))
}

pub(crate) async fn delete_mistral_account(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<MistralAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::remove_mistral_account(&state.core.data_root, &id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restarts::restart_mistral_providers_for_auth_change(&state, "mistral auth updated").await;
    Ok(Json(mistral_accounts_response(&state).await))
}

pub(crate) async fn list_copilot_accounts(
    State(state): State<Arc<AppState>>,
) -> Result<Json<CopilotAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    Ok(Json(copilot_accounts_response(&state).await))
}

pub(crate) async fn upsert_copilot_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CopilotAccountUpsertReq>,
) -> Result<Json<CopilotAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::add_copilot_account(&state.core.data_root, req.label, req.token, req.email)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restarts::restart_copilot_providers_for_auth_change(&state, "copilot auth updated").await;
    Ok(Json(copilot_accounts_response(&state).await))
}

pub(crate) async fn set_copilot_active_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CopilotActiveAccountReq>,
) -> Result<Json<CopilotAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    if let Some(ref account_id) = req.account_id {
        let registry = provider_accounts::load_copilot_registry(&state.core.data_root).await;
        if !registry.accounts.iter().any(|a| a.id == *account_id) {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "unknown account".to_string(),
                }),
            ));
        }
    }
    provider_accounts::set_active_copilot_account(&state.core.data_root, req.account_id)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restarts::restart_copilot_providers_for_auth_change(&state, "copilot auth updated").await;
    Ok(Json(copilot_accounts_response(&state).await))
}

pub(crate) async fn delete_copilot_account(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<CopilotAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::remove_copilot_account(&state.core.data_root, &id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restarts::restart_copilot_providers_for_auth_change(&state, "copilot auth updated").await;
    Ok(Json(copilot_accounts_response(&state).await))
}

pub(crate) async fn list_cursor_accounts(
    State(state): State<Arc<AppState>>,
) -> Result<Json<CursorAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    Ok(Json(cursor_accounts_response(&state).await))
}

pub(crate) async fn upsert_cursor_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CursorAccountUpsertReq>,
) -> Result<Json<CursorAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::add_cursor_account(&state.core.data_root, req.label, req.token, req.email)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restarts::restart_cursor_providers_for_auth_change(&state, "cursor auth updated").await;
    Ok(Json(cursor_accounts_response(&state).await))
}

pub(crate) async fn set_cursor_active_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CursorActiveAccountReq>,
) -> Result<Json<CursorAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    if let Some(ref account_id) = req.account_id {
        let registry = provider_accounts::load_cursor_registry(&state.core.data_root).await;
        if !registry.accounts.iter().any(|a| a.id == *account_id) {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "unknown account".to_string(),
                }),
            ));
        }
    }
    provider_accounts::set_active_cursor_account(&state.core.data_root, req.account_id)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restarts::restart_cursor_providers_for_auth_change(&state, "cursor auth updated").await;
    Ok(Json(cursor_accounts_response(&state).await))
}

pub(crate) async fn delete_cursor_account(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<CursorAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::remove_cursor_account(&state.core.data_root, &id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restarts::restart_cursor_providers_for_auth_change(&state, "cursor auth updated").await;
    Ok(Json(cursor_accounts_response(&state).await))
}
