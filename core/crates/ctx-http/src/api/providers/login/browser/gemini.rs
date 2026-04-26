use super::*;

#[derive(Debug, Deserialize)]
pub(crate) struct GeminiLoginStartReq {
    label: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct GeminiLoginStartResp {
    login_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    auth_url: Option<String>,
}

fn gemini_login_home(data_root: &StdPath, login_id: &str) -> PathBuf {
    data_root
        .join("providers")
        .join("gemini")
        .join("login-sessions")
        .join(login_id)
}

async fn monitor_gemini_login(state: Arc<AppState>, login_id: String, label: Option<String>) {
    let adapter = {
        let map = state.providers.adapters.lock().await;
        map.get("gemini").cloned()
    };
    let Some(adapter) = adapter else {
        let mut map = state.providers.gemini_login_sessions.lock().await;
        if let Some(entry) = map.get_mut(&login_id) {
            entry.status = "failed".to_string();
            entry.error = Some("provider adapter not available".to_string());
        }
        return;
    };

    let login_home = gemini_login_home(&state.core.data_root, &login_id);
    let workdir = login_home.join("workspace");
    if let Err(err) = tokio::fs::create_dir_all(&workdir).await {
        let mut map = state.providers.gemini_login_sessions.lock().await;
        if let Some(entry) = map.get_mut(&login_id) {
            entry.status = "failed".to_string();
            entry.error = Some(format!("failed to prepare login workspace: {err}"));
        }
        return;
    }

    let mut provider_env = HashMap::new();
    provider_env.insert("CTX_DAEMON_URL".to_string(), state.core.daemon_url.clone());
    provider_env.insert("CTX_MCP_DISABLED".to_string(), "1".to_string());
    provider_env.insert(
        "GEMINI_CLI_HOME".to_string(),
        login_home.to_string_lossy().to_string(),
    );
    provider_env.insert(
        provider_accounts::GEMINI_FORCE_FILE_STORAGE_ENV.to_string(),
        "true".to_string(),
    );
    provider_env.insert(
        "CTX_DATA_ROOT".to_string(),
        state.core.data_root.to_string_lossy().to_string(),
    );

    let (event_tx, mut event_rx) = mpsc::channel(64);
    let auth_result = adapter
        .authenticate_session(
            format!("gemini-login-{login_id}"),
            workdir,
            provider_env,
            Some(provider_accounts::GEMINI_CREDENTIAL_KIND_OAUTH_PERSONAL.to_string()),
            event_tx,
            ctx_providers::adapters::ProviderRunHooks::default(),
        )
        .await;
    if let Err(err) = auth_result {
        let mut map = state.providers.gemini_login_sessions.lock().await;
        if let Some(entry) = map.get_mut(&login_id) {
            entry.status = "failed".to_string();
            entry.error = Some(logs::redact_sensitive(&err.to_string()));
        }
        let _ = tokio::fs::remove_dir_all(&login_home).await;
        return;
    }

    let oauth_path = login_home.join(".gemini").join("oauth_creds.json");
    let google_accounts_path = login_home.join(".gemini").join("google_accounts.json");
    let started_at = Instant::now();
    let timeout = gemini_login_timeout();
    let mut observed_auth_url = false;

    loop {
        let mut channel_disconnected = false;
        loop {
            match event_rx.try_recv() {
                Ok(event) => {
                    if let Some(auth_url) = extract_auth_url_from_value(&event.payload_json) {
                        observed_auth_url = true;
                        let mut map = state.providers.gemini_login_sessions.lock().await;
                        if let Some(entry) = map.get_mut(&login_id) {
                            entry.auth_url = Some(auth_url);
                        }
                    }
                    if matches!(event.event_type, ctx_core::models::SessionEventType::Error) {
                        let message = event
                            .payload_json
                            .get("message")
                            .and_then(serde_json::Value::as_str)
                            .map(logs::redact_sensitive)
                            .unwrap_or_else(|| "gemini authenticate reported an error".to_string());
                        let mut map = state.providers.gemini_login_sessions.lock().await;
                        if let Some(entry) = map.get_mut(&login_id) {
                            entry.status = "failed".to_string();
                            entry.error = Some(message);
                        }
                        let _ = tokio::fs::remove_dir_all(&login_home).await;
                        return;
                    }
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    channel_disconnected = true;
                    break;
                }
            }
        }

        let oauth_raw = tokio::fs::read_to_string(&oauth_path)
            .await
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        if let Some(oauth_raw) = oauth_raw {
            let oauth_value = serde_json::from_str::<serde_json::Value>(&oauth_raw);
            let oauth_valid = oauth_value
                .as_ref()
                .ok()
                .is_some_and(serde_json::Value::is_object);
            if !oauth_valid {
                let mut map = state.providers.gemini_login_sessions.lock().await;
                if let Some(entry) = map.get_mut(&login_id) {
                    entry.status = "failed".to_string();
                    entry.error =
                        Some("captured oauth_creds.json is not a valid JSON object".to_string());
                }
                let _ = tokio::fs::remove_dir_all(&login_home).await;
                return;
            }

            let google_accounts_raw = tokio::fs::read_to_string(&google_accounts_path)
                .await
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty());
            let google_accounts_value = google_accounts_raw
                .as_deref()
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok());
            let email = google_accounts_value
                .as_ref()
                .and_then(first_email_from_google_accounts);
            let added = provider_accounts::add_gemini_account(
                &state.core.data_root,
                label.clone(),
                oauth_raw,
                google_accounts_raw,
                email,
            )
            .await;
            match added {
                Ok(registry) => {
                    if let Err(err) = restarts::restart_gemini_providers_for_auth_change(
                        &state,
                        "gemini auth updated",
                    )
                    .await
                    {
                        let mut map = state.providers.gemini_login_sessions.lock().await;
                        if let Some(entry) = map.get_mut(&login_id) {
                            entry.status = "failed".to_string();
                            entry.error = Some(logs::redact_sensitive(&err.to_string()));
                        }
                        return;
                    }
                    let mut map = state.providers.gemini_login_sessions.lock().await;
                    if let Some(entry) = map.get_mut(&login_id) {
                        entry.status = "success".to_string();
                        entry.account_id = registry.active_account_id.clone();
                        entry.error = None;
                    }
                }
                Err(err) => {
                    let mut map = state.providers.gemini_login_sessions.lock().await;
                    if let Some(entry) = map.get_mut(&login_id) {
                        entry.status = "failed".to_string();
                        entry.error = Some(logs::redact_sensitive(&err.to_string()));
                    }
                }
            }
            let _ = tokio::fs::remove_dir_all(&login_home).await;
            return;
        }

        if channel_disconnected && !observed_auth_url {
            let mut map = state.providers.gemini_login_sessions.lock().await;
            if let Some(entry) = map.get_mut(&login_id) {
                entry.status = "failed".to_string();
                if entry.error.is_none() {
                    entry.error = Some(
                        "Gemini sign-in did not emit an OAuth URL; the runtime may require API-key auth in this environment."
                            .to_string(),
                    );
                }
            }
            let _ = tokio::fs::remove_dir_all(&login_home).await;
            return;
        }

        if started_at.elapsed() >= timeout {
            let mut map = state.providers.gemini_login_sessions.lock().await;
            if let Some(entry) = map.get_mut(&login_id) {
                entry.status = "timeout".to_string();
                if entry.error.is_none() {
                    entry.error = Some("timed out waiting for Gemini OAuth completion".to_string());
                }
            }
            let _ = tokio::fs::remove_dir_all(&login_home).await;
            return;
        }

        tokio::time::sleep(GEMINI_LOGIN_POLL_INTERVAL).await;
    }
}

pub(crate) async fn start_gemini_login(
    State(state): State<Arc<AppState>>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Json(req): Json<GeminiLoginStartReq>,
) -> Result<Json<GeminiLoginStartResp>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    let login_id = uuid::Uuid::new_v4().to_string();
    {
        let mut map = state.providers.gemini_login_sessions.lock().await;
        map.insert(
            login_id.clone(),
            provider_accounts::GeminiLoginStatus {
                login_id: login_id.clone(),
                auth_url: None,
                status: "pending".to_string(),
                account_id: None,
                error: None,
            },
        );
    }

    let state_clone = Arc::clone(&state);
    let login_id_for_task = login_id.clone();
    tokio::spawn(async move {
        monitor_gemini_login(state_clone, login_id_for_task, req.label).await;
    });

    Ok(Json(GeminiLoginStartResp {
        login_id,
        auth_url: None,
    }))
}

pub(crate) async fn get_gemini_login(
    State(state): State<Arc<AppState>>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Path(id): Path<String>,
) -> Result<Json<provider_accounts::GeminiLoginStatus>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    let map = state.providers.gemini_login_sessions.lock().await;
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
