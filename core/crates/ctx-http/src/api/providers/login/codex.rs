use super::*;

#[derive(Debug, Deserialize)]
pub(crate) struct CodexLoginStartReq {
    label: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct CodexLoginStartResp {
    account_id: String,
    auth_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected_callback_url: Option<String>,
    completion_token: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CodexLoginCompleteReq {
    callback_url: String,
    completion_token: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct CodexLoginCompleteResp {
    accepted: bool,
    status_code: u16,
}

struct CodexLoginProcess {
    login_id: String,
    auth_url: String,
    account_dir: PathBuf,
    child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    reader: tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
}

struct CodexLoginCompletion {
    success: bool,
    error: Option<String>,
}

async fn restore_completion_token(state: &Arc<AppState>, id: &str, completion_token: &str) {
    let mut map = state.providers.codex_login_sessions.lock().await;
    if let Some(status) = map.get_mut(id) {
        if status.status == "pending" && status.completion_token.is_none() {
            status.completion_token = Some(completion_token.to_string());
        }
    }
}

pub(crate) async fn complete_codex_login(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Json(req): Json<CodexLoginCompleteReq>,
) -> Result<Json<CodexLoginCompleteResp>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    let expected_callback = {
        let mut map = state.providers.codex_login_sessions.lock().await;
        let Some(status) = map.get_mut(&id) else {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "login not found".to_string(),
                }),
            ));
        };
        if status.status != "pending" {
            return Err((
                StatusCode::CONFLICT,
                Json(ApiErrorResp {
                    error: "login is not pending".to_string(),
                }),
            ));
        }
        if status.completion_token.as_deref() != Some(req.completion_token.as_str()) {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(ApiErrorResp {
                    error: "invalid completion token".to_string(),
                }),
            ));
        }
        let Some(expected_callback) = status.expected_callback_url.clone() else {
            return Err((
                StatusCode::CONFLICT,
                Json(ApiErrorResp {
                    error: "login is missing expected callback metadata".to_string(),
                }),
            ));
        };
        status.completion_token = None;
        expected_callback
    };

    if let Err(err) = validate_callback_url(&req.callback_url, Some(expected_callback.as_str())) {
        restore_completion_token(&state, &id, &req.completion_token).await;
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: err.to_string(),
            }),
        ));
    }

    let parsed_callback = Url::parse(&req.callback_url).map_err(|err| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: format!("invalid callback_url: {err}"),
            }),
        )
    })?;
    let callback_host = parsed_callback
        .host_str()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let client = if callback_host == "localhost" {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .resolve(
                "localhost",
                std::net::SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0),
            )
            .build()
            .map_err(|err| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: format!("failed to build callback replay client: {err}"),
                    }),
                )
            })?
    } else {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|err| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: format!("failed to build callback replay client: {err}"),
                    }),
                )
            })?
    };

    let response = match client
        .get(&req.callback_url)
        .timeout(Duration::from_secs(20))
        .send()
        .await
    {
        Ok(response) => response,
        Err(err) => {
            restore_completion_token(&state, &id, &req.completion_token).await;
            return Err((
                StatusCode::BAD_GATEWAY,
                Json(ApiErrorResp {
                    error: format!("failed to replay callback: {err}"),
                }),
            ));
        }
    };
    let status = response.status();
    if !status.is_success() {
        restore_completion_token(&state, &id, &req.completion_token).await;
        return Err((
            StatusCode::BAD_GATEWAY,
            Json(ApiErrorResp {
                error: format!("callback replay returned {status}"),
            }),
        ));
    }
    let status_code = status.as_u16();

    Ok(Json(CodexLoginCompleteResp {
        accepted: true,
        status_code,
    }))
}
pub(crate) async fn start_codex_login(
    State(state): State<Arc<AppState>>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Json(req): Json<CodexLoginStartReq>,
) -> Result<Json<CodexLoginStartResp>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    let account_id = uuid::Uuid::new_v4().to_string();
    let label = provider_accounts::normalize_label(req.label, &account_id);
    let account_dir =
        provider_accounts::ensure_codex_account_dir(&state.core.data_root, &account_id)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: e.to_string(),
                    }),
                )
            })?;
    let (cfg, managed_config_error) =
        crate::api::provider_launch::load_managed_agent_server_config_with_error(
            &state.core.data_root,
        )
        .await;
    if let Some(error) = managed_config_error {
        let _ = tokio::fs::remove_dir_all(&account_dir).await;
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp { error }),
        ));
    }
    let codex_bin = crate::installer::require_codex_cli_command_path_for_target(
        &cfg,
        Some(ctx_provider_install::install_state::InstallTarget::Host),
    )
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: e.to_string(),
            }),
        )
    })?;
    let login = match start_codex_login_process(&account_dir, &codex_bin).await {
        Ok(login) => login,
        Err(e) => {
            let _ = tokio::fs::remove_dir_all(&account_dir).await;
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            ));
        }
    };
    let expected_callback_url = expected_callback_from_auth_url(&login.auth_url);
    let completion_token = uuid::Uuid::new_v4().to_string();
    let auth_url = login.auth_url.clone();
    let status = provider_accounts::CodexLoginStatus {
        account_id: account_id.clone(),
        auth_url: auth_url.clone(),
        expected_callback_url: expected_callback_url.clone(),
        completion_token: Some(completion_token.clone()),
        status: "pending".to_string(),
        error: None,
    };
    {
        let mut map = state.providers.codex_login_sessions.lock().await;
        map.insert(account_id.clone(), status);
    }
    let state_clone = Arc::clone(&state);
    let account_id_for_task = account_id.clone();
    tokio::spawn(async move {
        monitor_codex_login(state_clone, account_id_for_task, label, login).await;
    });

    Ok(Json(CodexLoginStartResp {
        account_id,
        auth_url,
        expected_callback_url,
        completion_token,
    }))
}

pub(crate) async fn get_codex_login(
    State(state): State<Arc<AppState>>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Path(id): Path<String>,
) -> Result<Json<provider_accounts::CodexLoginStatus>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    let map = state.providers.codex_login_sessions.lock().await;
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

async fn start_codex_login_process(
    account_dir: &PathBuf,
    codex_bin: &str,
) -> anyhow::Result<CodexLoginProcess> {
    let mut child = spawn_codex_app_server(account_dir, codex_bin)?;
    let stdout = child
        .stdout
        .take()
        .context("codex app-server stdout unavailable")?;
    let mut reader = BufReader::new(stdout).lines();
    let mut stdin = child
        .stdin
        .take()
        .context("codex app-server stdin unavailable")?;

    let init_request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "clientInfo": {
                "name": "ctx",
                "title": "ctx",
                "version": env!("CARGO_PKG_VERSION")
            }
        }
    });
    send_codex_jsonrpc(&mut stdin, &init_request).await?;
    wait_for_codex_response(&mut reader, 1, CODEX_LOGIN_RPC_TIMEOUT).await?;
    send_codex_jsonrpc(
        &mut stdin,
        &serde_json::json!({"jsonrpc": "2.0", "method": "initialized"}),
    )
    .await?;

    let login_request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "account/login/start",
        "params": { "type": "chatgpt" }
    });
    send_codex_jsonrpc(&mut stdin, &login_request).await?;
    let response = wait_for_codex_response(&mut reader, 2, CODEX_LOGIN_RPC_TIMEOUT).await?;
    let result = response
        .get("result")
        .and_then(|v| v.as_object())
        .context("codex login missing result")?;
    let auth_url = result
        .get("authUrl")
        .or_else(|| result.get("auth_url"))
        .and_then(|v| v.as_str())
        .context("codex login missing auth_url")?
        .to_string();
    let login_id = result
        .get("loginId")
        .or_else(|| result.get("login_id"))
        .and_then(|v| v.as_str())
        .context("codex login missing login_id")?
        .to_string();

    Ok(CodexLoginProcess {
        login_id,
        auth_url,
        account_dir: account_dir.clone(),
        child,
        stdin,
        reader,
    })
}

async fn monitor_codex_login(
    state: Arc<AppState>,
    account_id: String,
    label: String,
    mut login: CodexLoginProcess,
) {
    let completion = wait_for_codex_login_completion(&mut login.reader, &login.login_id).await;
    let mut status = match completion {
        Ok(completion) => completion,
        Err(err) => CodexLoginCompletion {
            success: false,
            error: Some(err.to_string()),
        },
    };

    if status.success {
        let (email, plan_type) = fetch_codex_account_details(&mut login.stdin, &mut login.reader)
            .await
            .unwrap_or((None, None));
        if let Err(err) =
            persist_successful_codex_login(&state, &account_id, label, email, plan_type).await
        {
            status.success = false;
            status.error = Some(err.to_string());
            let _ = tokio::fs::remove_dir_all(&login.account_dir).await;
        }
    } else {
        let _ = tokio::fs::remove_dir_all(&login.account_dir).await;
    }

    {
        let mut map = state.providers.codex_login_sessions.lock().await;
        if let Some(entry) = map.get_mut(&account_id) {
            entry.status = if status.success {
                "success".to_string()
            } else {
                "failed".to_string()
            };
            entry.completion_token = None;
            entry.error = status.error;
        }
    }

    let _ = login.child.kill().await;
}

async fn persist_successful_codex_login(
    state: &Arc<AppState>,
    account_id: &str,
    label: String,
    email: Option<String>,
    plan_type: Option<String>,
) -> anyhow::Result<()> {
    let entry = provider_accounts::CodexAccountEntry {
        id: account_id.to_string(),
        label,
        kind: provider_accounts::CODEX_CREDENTIAL_KIND_OAUTH.to_string(),
        email,
        plan_type,
        created_at: Utc::now(),
        last_used_at: Some(Utc::now()),
        secret_ref: None,
        endpoint_profile: provider_accounts::CodexEndpointProfile::default(),
    };
    provider_accounts::upsert_codex_account(&state.core.data_root, entry)
        .await
        .with_context(|| format!("persisting codex account {account_id}"))?;

    let persist_result = async {
        let ingested = provider_accounts::ingest_codex_account_auth_to_secret_store(
            &state.core.data_root,
            account_id,
        )
        .await
        .with_context(|| format!("ingesting codex auth for account {account_id}"))?;
        if !ingested {
            anyhow::bail!("missing persisted codex auth file for account {account_id}");
        }
        provider_accounts::set_active_codex_account(
            &state.core.data_root,
            Some(account_id.to_string()),
        )
        .await
        .with_context(|| format!("setting active codex account {account_id}"))?;
        restarts::restart_codex_providers_for_auth_change(state, "codex auth updated").await;
        Ok(())
    }
    .await;

    if let Err(err) = persist_result {
        let _ = provider_accounts::remove_codex_account(&state.core.data_root, account_id).await;
        return Err(err);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctx_store::StoreManager;
    use std::collections::HashMap;

    #[tokio::test]
    async fn codex_login_persistence_requires_auth_file() {
        let data_dir = tempfile::tempdir().unwrap();
        let stores = StoreManager::open(data_dir.path()).await.unwrap();
        let state = Arc::new(AppState::new(
            data_dir.path().to_path_buf(),
            stores,
            HashMap::new(),
            "http://127.0.0.1:4399".to_string(),
            None,
        ));
        let account_id = "acct-missing-auth";
        provider_accounts::ensure_codex_account_dir(&state.core.data_root, account_id)
            .await
            .unwrap();

        let err = persist_successful_codex_login(
            &state,
            account_id,
            "Missing Auth".to_string(),
            None,
            None,
        )
        .await
        .unwrap_err();

        assert!(err
            .to_string()
            .contains("missing persisted codex auth file"));
        let registry = provider_accounts::load_codex_registry(&state.core.data_root).await;
        assert!(registry.accounts.is_empty());
        assert!(registry.active_account_id.is_none());
    }
}

fn spawn_codex_app_server(
    account_dir: &PathBuf,
    codex_bin: &str,
) -> anyhow::Result<tokio::process::Child> {
    let codex_bin = codex_bin.trim();
    if codex_bin.is_empty() {
        anyhow::bail!("codex-cli runtime path is empty");
    }
    if !std::path::Path::new(codex_bin).is_absolute() {
        anyhow::bail!("codex-cli runtime path must be absolute, got `{codex_bin}`");
    }
    let mut cmd = Command::new(codex_bin);
    cmd.args(CODEX_APP_SERVER_ARGS)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for key in [
        "CTX_PROVIDER_SESSION_REF",
        "CODEX_THREAD_ID",
        "CODEX_SESSION_ID",
        "CLAUDE_SESSION_ID",
        "CLAUDE_THREAD_ID",
        "GEMINI_SESSION_ID",
        "GEMINI_THREAD_ID",
        "ACP_SESSION_ID",
    ] {
        cmd.env_remove(key);
    }
    cmd.env("CODEX_HOME", account_dir);
    cmd.spawn()
        .with_context(|| format!("spawning codex app-server via `{codex_bin}`"))
}

async fn send_codex_jsonrpc(
    stdin: &mut tokio::process::ChildStdin,
    value: &serde_json::Value,
) -> anyhow::Result<()> {
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    stdin.write_all(&bytes).await?;
    stdin.flush().await?;
    Ok(())
}

async fn wait_for_codex_response(
    reader: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    request_id: i64,
    timeout: Duration,
) -> anyhow::Result<serde_json::Value> {
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            bail!("codex rpc timeout waiting for response");
        }
        let line = tokio::time::timeout(remaining, reader.next_line())
            .await
            .context("codex rpc read timeout")??;
        let line = line.ok_or_else(|| anyhow::anyhow!("codex rpc stdout closed"))?;
        let value: serde_json::Value = serde_json::from_str(&line)?;
        if let Some(id) = value.get("id").and_then(|v| v.as_i64()) {
            if id == request_id {
                if value.get("error").is_some() {
                    bail!("codex rpc error: {value}");
                }
                return Ok(value);
            }
        }
    }
}

async fn wait_for_codex_login_completion(
    reader: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    login_id: &str,
) -> anyhow::Result<CodexLoginCompletion> {
    loop {
        let line = reader
            .next_line()
            .await
            .context("codex login read failed")?;
        let line = line.ok_or_else(|| anyhow::anyhow!("codex login stdout closed"))?;
        let value: serde_json::Value = serde_json::from_str(&line)?;
        let method = value.get("method").and_then(|v| v.as_str());
        if method != Some("account/login/completed") {
            continue;
        }
        let params = value.get("params").unwrap_or(&serde_json::Value::Null);
        let found_login_id = params
            .get("loginId")
            .or_else(|| params.get("login_id"))
            .and_then(|v| v.as_str());
        if let Some(found) = found_login_id {
            if found != login_id {
                continue;
            }
        }
        let success = params
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let error = params
            .get("error")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        return Ok(CodexLoginCompletion { success, error });
    }
}

async fn fetch_codex_account_details(
    stdin: &mut tokio::process::ChildStdin,
    reader: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
) -> anyhow::Result<(Option<String>, Option<String>)> {
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "account/read",
        "params": { "refreshToken": false }
    });
    send_codex_jsonrpc(stdin, &request).await?;
    let response = wait_for_codex_response(reader, 3, CODEX_LOGIN_RPC_TIMEOUT).await?;
    let account = response
        .get("result")
        .and_then(|v| v.get("account"))
        .and_then(|v| v.as_object());
    let Some(account) = account else {
        return Ok((None, None));
    };
    if account.get("type").and_then(|v| v.as_str()) != Some("chatgpt") {
        return Ok((None, None));
    }
    let email = account
        .get("email")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let plan_type = account
        .get("planType")
        .or_else(|| account.get("plan_type"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    Ok((email, plan_type))
}
