use super::*;

#[derive(Debug, Deserialize)]
pub(crate) struct MistralLoginStartReq {
    label: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct MistralLoginStartResp {
    login_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    auth_url: Option<String>,
}

fn mistral_login_home(data_root: &StdPath, login_id: &str) -> PathBuf {
    data_root
        .join("providers")
        .join("mistral")
        .join("login-sessions")
        .join(login_id)
}

async fn monitor_mistral_login(state: Arc<AppState>, login_id: String, label: Option<String>) {
    let adapter = {
        let map = state.providers.adapters.lock().await;
        map.get("mistral").cloned()
    };
    let Some(adapter) = adapter else {
        let mut map = state.providers.mistral_login_sessions.lock().await;
        if let Some(entry) = map.get_mut(&login_id) {
            entry.status = "failed".to_string();
            entry.error = Some("provider adapter not available".to_string());
        }
        return;
    };

    let login_home = mistral_login_home(&state.core.data_root, &login_id);
    let workdir = login_home.join("workspace");
    if let Err(err) = tokio::fs::create_dir_all(&workdir).await {
        let mut map = state.providers.mistral_login_sessions.lock().await;
        if let Some(entry) = map.get_mut(&login_id) {
            entry.status = "failed".to_string();
            entry.error = Some(format!("failed to prepare login workspace: {err}"));
        }
        return;
    }
    let mistral_home =
        match provider_accounts::ensure_mistral_runtime_home(&state.core.data_root).await {
            Ok(home) => home,
            Err(err) => {
                let mut map = state.providers.mistral_login_sessions.lock().await;
                if let Some(entry) = map.get_mut(&login_id) {
                    entry.status = "failed".to_string();
                    entry.error = Some(format!("failed to prepare mistral runtime home: {err}"));
                }
                let _ = tokio::fs::remove_dir_all(&login_home).await;
                return;
            }
        };

    let mut provider_env = HashMap::new();
    provider_env.insert("CTX_DAEMON_URL".to_string(), state.core.daemon_url.clone());
    provider_env.insert("CTX_MCP_DISABLED".to_string(), "1".to_string());
    provider_env.insert(
        "CTX_DATA_ROOT".to_string(),
        state.core.data_root.to_string_lossy().to_string(),
    );
    provider_env.insert(
        "HOME".to_string(),
        mistral_home.to_string_lossy().to_string(),
    );
    provider_env.insert(
        "XDG_CONFIG_HOME".to_string(),
        mistral_home.join(".config").to_string_lossy().to_string(),
    );
    provider_env.insert(
        "XDG_CACHE_HOME".to_string(),
        mistral_home.join(".cache").to_string_lossy().to_string(),
    );

    let (event_tx, mut event_rx) = mpsc::channel(64);
    let auth_result = adapter
        .authenticate_session(
            format!("mistral-login-{login_id}"),
            workdir,
            provider_env,
            None,
            event_tx,
            ctx_providers::adapters::ProviderRunHooks::default(),
        )
        .await;
    if let Err(err) = auth_result {
        let mut map = state.providers.mistral_login_sessions.lock().await;
        if let Some(entry) = map.get_mut(&login_id) {
            entry.status = "failed".to_string();
            entry.error = Some(logs::redact_sensitive(&err.to_string()));
        }
        let _ = tokio::fs::remove_dir_all(&login_home).await;
        return;
    }

    let started_at = Instant::now();
    let timeout = mistral_login_timeout();
    let mut observed_auth_url = false;
    let mut observed_email = None::<String>;

    loop {
        if started_at.elapsed() >= timeout {
            let mut map = state.providers.mistral_login_sessions.lock().await;
            if let Some(entry) = map.get_mut(&login_id) {
                entry.status = "timeout".to_string();
                if entry.error.is_none() {
                    entry.error =
                        Some("timed out waiting for Mistral OAuth completion".to_string());
                }
            }
            let _ = tokio::fs::remove_dir_all(&login_home).await;
            return;
        }

        let event = match tokio::time::timeout(MISTRAL_LOGIN_POLL_INTERVAL, event_rx.recv()).await {
            Ok(Some(event)) => event,
            Ok(None) => {
                let mut map = state.providers.mistral_login_sessions.lock().await;
                if let Some(entry) = map.get_mut(&login_id) {
                    entry.status = "failed".to_string();
                    if entry.error.is_none() {
                        entry.error = Some(if observed_auth_url {
                            "Mistral sign-in session ended before completion.".to_string()
                        } else {
                            "Mistral sign-in did not emit an OAuth URL in this environment."
                                .to_string()
                        });
                    }
                }
                let _ = tokio::fs::remove_dir_all(&login_home).await;
                return;
            }
            Err(_) => continue,
        };

        if let Some(auth_url) = extract_auth_url_from_value(&event.payload_json) {
            observed_auth_url = true;
            let mut map = state.providers.mistral_login_sessions.lock().await;
            if let Some(entry) = map.get_mut(&login_id) {
                entry.auth_url = Some(auth_url);
            }
        }
        if observed_email.is_none() {
            observed_email = first_email_from_value(&event.payload_json);
        }

        if is_auth_failure_notice_code(auth_notice_code(&event.payload_json)) {
            let message = event
                .payload_json
                .get("message")
                .and_then(serde_json::Value::as_str)
                .map(logs::redact_sensitive)
                .unwrap_or_else(|| "mistral authenticate reported an error".to_string());
            let mut map = state.providers.mistral_login_sessions.lock().await;
            if let Some(entry) = map.get_mut(&login_id) {
                entry.status = "failed".to_string();
                entry.error = Some(message);
            }
            let _ = tokio::fs::remove_dir_all(&login_home).await;
            return;
        }

        if matches!(event.event_type, ctx_core::models::SessionEventType::Notice) {
            let code = auth_notice_code(&event.payload_json);
            if is_auth_success_notice_code(code) {
                if let Err(err) = provider_accounts::upsert_mistral_account(
                    &state.core.data_root,
                    label.clone(),
                    observed_email.clone(),
                )
                .await
                {
                    let mut map = state.providers.mistral_login_sessions.lock().await;
                    if let Some(entry) = map.get_mut(&login_id) {
                        entry.status = "failed".to_string();
                        entry.error = Some(logs::redact_sensitive(&err.to_string()));
                    }
                    let _ = tokio::fs::remove_dir_all(&login_home).await;
                    return;
                }
                let restart_result = restarts::restart_mistral_providers_for_auth_change(
                    &state,
                    "mistral auth updated",
                )
                .await;
                let mut map = state.providers.mistral_login_sessions.lock().await;
                if let Some(entry) = map.get_mut(&login_id) {
                    entry.auth_url = None;
                    match restart_result {
                        Ok(()) => {
                            entry.status = "success".to_string();
                            entry.error = None;
                        }
                        Err(err) => {
                            entry.status = "failed".to_string();
                            entry.error = Some(logs::redact_sensitive(&format!(
                                "auth saved but provider restart failed: {err:#}"
                            )));
                        }
                    }
                }
                let _ = tokio::fs::remove_dir_all(&login_home).await;
                return;
            }
            if is_auth_failure_notice_code(code) {
                let message = event
                    .payload_json
                    .get("message")
                    .and_then(serde_json::Value::as_str)
                    .map(logs::redact_sensitive)
                    .unwrap_or_else(|| "Mistral sign-in failed. Retry.".to_string());
                let mut map = state.providers.mistral_login_sessions.lock().await;
                if let Some(entry) = map.get_mut(&login_id) {
                    entry.status = "failed".to_string();
                    entry.error = Some(message);
                }
                let _ = tokio::fs::remove_dir_all(&login_home).await;
                return;
            }
        }
    }
}

pub(crate) async fn start_mistral_login(
    State(state): State<Arc<AppState>>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Json(req): Json<MistralLoginStartReq>,
) -> Result<Json<MistralLoginStartResp>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    let label = req.label;
    let login_id = uuid::Uuid::new_v4().to_string();
    {
        let mut map = state.providers.mistral_login_sessions.lock().await;
        map.insert(
            login_id.clone(),
            provider_accounts::MistralLoginStatus {
                login_id: login_id.clone(),
                auth_url: None,
                status: "pending".to_string(),
                error: None,
            },
        );
    }

    let state_clone = Arc::clone(&state);
    let login_id_for_task = login_id.clone();
    tokio::spawn(async move {
        monitor_mistral_login(state_clone, login_id_for_task, label).await;
    });

    Ok(Json(MistralLoginStartResp {
        login_id,
        auth_url: None,
    }))
}

pub(crate) async fn get_mistral_login(
    State(state): State<Arc<AppState>>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Path(id): Path<String>,
) -> Result<Json<provider_accounts::MistralLoginStatus>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    let map = state.providers.mistral_login_sessions.lock().await;
    let status = map.get(&id).cloned().ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "login not found".to_string(),
            }),
        )
    })?;
    Ok(Json(status))
}
