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
pub(crate) struct ClaudeLoginStartReq {
    label: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ClaudeLoginStartResp {
    login_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    auth_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ClaudeLoginCompleteReq {
    callback_code: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct ClaudeLoginCompleteResp {
    accepted: bool,
}

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

#[derive(Debug, Deserialize)]
pub(crate) struct QwenLoginStartReq {
    label: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct QwenLoginStartResp {
    login_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    auth_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AmpLoginStartReq {
    label: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct AmpLoginStartResp {
    login_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    auth_url: Option<String>,
}

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

struct ClaudeLoginProcess {
    line_rx: mpsc::UnboundedReceiver<String>,
    input_rx: mpsc::UnboundedReceiver<String>,
    buffered_lines: Vec<String>,
    auth_url: Option<String>,
    exit_rx: oneshot::Receiver<anyhow::Result<portable_pty::ExitStatus>>,
    killer: Arc<StdMutex<Box<dyn portable_pty::ChildKiller + Send + Sync>>>,
    writer: Arc<StdMutex<Box<dyn Write + Send>>>,
}

struct ClaudeLoginSpawn {
    line_rx: mpsc::UnboundedReceiver<String>,
    exit_rx: oneshot::Receiver<anyhow::Result<portable_pty::ExitStatus>>,
    killer: Arc<StdMutex<Box<dyn portable_pty::ChildKiller + Send + Sync>>>,
    writer: Arc<StdMutex<Box<dyn Write + Send>>>,
}

const CODEX_LOGIN_RPC_TIMEOUT: Duration = Duration::from_secs(30);
const CLAUDE_LOGIN_URL_WAIT: Duration = Duration::from_secs(4);
const GEMINI_LOGIN_TIMEOUT_DEFAULT: Duration = Duration::from_secs(300);
const GEMINI_LOGIN_POLL_INTERVAL: Duration = Duration::from_millis(700);
const QWEN_LOGIN_TIMEOUT_DEFAULT: Duration = Duration::from_secs(300);
const QWEN_LOGIN_POLL_INTERVAL: Duration = Duration::from_millis(700);
const AMP_LOGIN_TIMEOUT_DEFAULT: Duration = Duration::from_secs(300);
const AMP_LOGIN_POLL_INTERVAL: Duration = Duration::from_millis(700);
const MISTRAL_LOGIN_TIMEOUT_DEFAULT: Duration = Duration::from_secs(300);
const MISTRAL_LOGIN_POLL_INTERVAL: Duration = Duration::from_millis(700);
const QWEN_OAUTH_AUTH_METHOD_ID: &str = "qwen-oauth";
const AMP_BROWSER_AUTH_METHOD_ID: &str = "amp_browser_login";
const CLAUDE_LOGIN_NO_AUTH_URL_TIMEOUT: Duration = Duration::from_secs(8);
const CLAUDE_LOGIN_URL_SETTLE_WAIT: Duration = Duration::from_millis(500);
const CLAUDE_LOGIN_COMPLETION_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const CLAUDE_LOGIN_EXIT_GRACE_WAIT: Duration = Duration::from_millis(400);

fn is_loopback_host(value: &str) -> bool {
    let host = value.trim().to_ascii_lowercase();
    if host == "localhost" {
        return true;
    }
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return ip.is_loopback();
    }
    false
}

pub(super) fn expected_callback_from_auth_url(auth_url: &str) -> Option<String> {
    let parsed = Url::parse(auth_url).ok()?;
    let redirect = parsed
        .query_pairs()
        .find_map(|(key, value)| (key == "redirect_uri").then_some(value.into_owned()))?;
    let callback = Url::parse(&redirect).ok()?;
    let host = callback.host_str()?;
    if !is_loopback_host(host) {
        return None;
    }
    Some(callback.to_string())
}

pub(super) fn validate_callback_url(
    callback_url: &str,
    expected_callback_url: Option<&str>,
) -> anyhow::Result<()> {
    let callback = Url::parse(callback_url)
        .with_context(|| format!("invalid callback_url: {callback_url}"))?;
    if callback.scheme() != "http" {
        anyhow::bail!("callback_url must use http scheme");
    }
    let host = callback
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("callback_url must include host"))?;
    if !is_loopback_host(host) {
        anyhow::bail!("callback_url host must be loopback");
    }
    if callback.port().is_none() {
        anyhow::bail!("callback_url must include explicit port");
    }
    if !callback.path().starts_with("/auth/callback") {
        anyhow::bail!("callback_url path must start with /auth/callback");
    }
    if callback.query().is_none() {
        anyhow::bail!("callback_url must include query parameters");
    }

    if let Some(expected_raw) = expected_callback_url {
        let expected = Url::parse(expected_raw)
            .with_context(|| format!("invalid expected callback URL: {expected_raw}"))?;
        if callback.port() != expected.port() {
            anyhow::bail!("callback_url port mismatch");
        }
        if callback.path() != expected.path() {
            anyhow::bail!("callback_url path mismatch");
        }
    }
    Ok(())
}

fn gemini_login_timeout() -> Duration {
    let seconds = std::env::var("CTX_GEMINI_LOGIN_TIMEOUT_SECS")
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(GEMINI_LOGIN_TIMEOUT_DEFAULT.as_secs());
    Duration::from_secs(seconds)
}

fn qwen_login_timeout() -> Duration {
    let seconds = std::env::var("CTX_QWEN_LOGIN_TIMEOUT_SECS")
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(QWEN_LOGIN_TIMEOUT_DEFAULT.as_secs());
    Duration::from_secs(seconds)
}

fn amp_login_timeout() -> Duration {
    let seconds = std::env::var("CTX_AMP_LOGIN_TIMEOUT_SECS")
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(AMP_LOGIN_TIMEOUT_DEFAULT.as_secs());
    Duration::from_secs(seconds)
}

fn mistral_login_timeout() -> Duration {
    let seconds = std::env::var("CTX_MISTRAL_LOGIN_TIMEOUT_SECS")
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(MISTRAL_LOGIN_TIMEOUT_DEFAULT.as_secs());
    Duration::from_secs(seconds)
}

fn first_email_from_google_accounts(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Object(map) => {
            let email = map
                .get("email")
                .or_else(|| map.get("accountEmail"))
                .or_else(|| map.get("account_email"))
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|candidate| !candidate.is_empty())
                .map(ToString::to_string);
            email.or_else(|| map.values().find_map(first_email_from_google_accounts))
        }
        serde_json::Value::Array(values) => {
            values.iter().find_map(first_email_from_google_accounts)
        }
        _ => None,
    }
}

fn first_email_from_value(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Object(map) => {
            let direct = map
                .get("email")
                .or_else(|| map.get("accountEmail"))
                .or_else(|| map.get("account_email"))
                .or_else(|| map.get("user_email"))
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|candidate| !candidate.is_empty())
                .map(ToString::to_string);
            direct.or_else(|| map.values().find_map(first_email_from_value))
        }
        serde_json::Value::Array(values) => values.iter().find_map(first_email_from_value),
        _ => None,
    }
}

pub(super) fn extract_auth_url_from_value(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(raw) => {
            let trimmed = raw.trim();
            if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
                return Some(trimmed.to_string());
            }
            extract_auth_url(trimmed)
        }
        serde_json::Value::Object(map) => {
            let direct = [
                "auth_url",
                "authUrl",
                "url",
                "login_url",
                "loginUrl",
                "authorize_url",
            ]
            .into_iter()
            .find_map(|key| map.get(key))
            .and_then(extract_auth_url_from_value);
            direct.or_else(|| map.values().find_map(extract_auth_url_from_value))
        }
        serde_json::Value::Array(values) => values.iter().find_map(extract_auth_url_from_value),
        _ => None,
    }
}

fn auth_notice_code(payload: &serde_json::Value) -> &str {
    payload
        .get("code")
        .or_else(|| payload.get("kind"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
}

fn is_auth_success_notice_code(code: &str) -> bool {
    matches!(
        code,
        "auth_complete" | "auth_completed" | "auth_success" | "authenticated"
    )
}

fn is_auth_failure_notice_code(code: &str) -> bool {
    matches!(code, "auth_failed" | "auth_error")
}

pub(crate) async fn complete_codex_login(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<CodexLoginCompleteReq>,
) -> Result<Json<CodexLoginCompleteResp>, (StatusCode, Json<ApiErrorResp>)> {
    let expected_callback = {
        let map = state.providers.codex_login_sessions.lock().await;
        let Some(status) = map.get(&id) else {
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
        status.expected_callback_url.clone()
    };

    validate_callback_url(&req.callback_url, expected_callback.as_deref()).map_err(|err| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: err.to_string(),
            }),
        )
    })?;

    let response = reqwest::Client::new()
        .get(&req.callback_url)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|err| {
            (
                StatusCode::BAD_GATEWAY,
                Json(ApiErrorResp {
                    error: format!("failed to replay callback: {err}"),
                }),
            )
        })?;
    let status = response.status();
    if !status.is_success() {
        return Err((
            StatusCode::BAD_GATEWAY,
            Json(ApiErrorResp {
                error: format!("callback replay returned {status}"),
            }),
        ));
    }
    let status_code = status.as_u16();

    {
        let mut map = state.providers.codex_login_sessions.lock().await;
        if let Some(status) = map.get_mut(&id) {
            status.completion_token = None;
        }
    }

    Ok(Json(CodexLoginCompleteResp {
        accepted: true,
        status_code,
    }))
}

pub(crate) async fn start_codex_login(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CodexLoginStartReq>,
) -> Result<Json<CodexLoginStartResp>, (StatusCode, Json<ApiErrorResp>)> {
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
    let login = match start_codex_login_process(&account_dir).await {
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
    Path(id): Path<String>,
) -> Result<Json<provider_accounts::CodexLoginStatus>, (StatusCode, Json<ApiErrorResp>)> {
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

pub(crate) async fn start_claude_login(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ClaudeLoginStartReq>,
) -> Result<Json<ClaudeLoginStartResp>, (StatusCode, Json<ApiErrorResp>)> {
    let login_id = uuid::Uuid::new_v4().to_string();
    let label = req.label;
    let mut login = start_claude_login_process(&state).await.map_err(|e| {
        let msg = e.to_string();
        let status = if msg.contains("runtime_command_") {
            StatusCode::BAD_REQUEST
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };
        (status, Json(ApiErrorResp { error: msg }))
    })?;
    let (input_tx, input_rx) = mpsc::unbounded_channel::<String>();
    {
        let mut map = state.providers.claude_login_inputs.lock().await;
        map.insert(login_id.clone(), input_tx);
    }
    login.input_rx = input_rx;
    let auth_url = login.auth_url.clone();
    let status = provider_accounts::ClaudeLoginStatus {
        login_id: login_id.clone(),
        auth_url: auth_url.clone(),
        status: "pending".to_string(),
        account_id: None,
        error: None,
    };
    {
        let mut map = state.providers.claude_login_sessions.lock().await;
        map.insert(login_id.clone(), status);
    }
    let state_clone = Arc::clone(&state);
    let login_id_for_task = login_id.clone();
    tokio::spawn(async move {
        monitor_claude_login(state_clone, login_id_for_task, label, login).await;
    });

    Ok(Json(ClaudeLoginStartResp { login_id, auth_url }))
}

pub(crate) async fn complete_claude_login(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<ClaudeLoginCompleteReq>,
) -> Result<Json<ClaudeLoginCompleteResp>, (StatusCode, Json<ApiErrorResp>)> {
    let callback_code = req.callback_code.trim();
    if callback_code.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "callback_code is required".to_string(),
            }),
        ));
    }
    {
        let sessions = state.providers.claude_login_sessions.lock().await;
        let Some(status) = sessions.get(&id) else {
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
                    error: "login is no longer pending".to_string(),
                }),
            ));
        }
    }
    let tx = {
        let map = state.providers.claude_login_inputs.lock().await;
        map.get(&id).cloned()
    }
    .ok_or_else(|| {
        (
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "login session is not accepting callback input".to_string(),
            }),
        )
    })?;
    tx.send(callback_code.to_string()).map_err(|_| {
        (
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "login session is no longer accepting callback input".to_string(),
            }),
        )
    })?;
    Ok(Json(ClaudeLoginCompleteResp { accepted: true }))
}

pub(crate) async fn get_claude_login(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<provider_accounts::ClaudeLoginStatus>, (StatusCode, Json<ApiErrorResp>)> {
    let map = state.providers.claude_login_sessions.lock().await;
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
    if let Some(token) = state.core.auth_token.clone() {
        provider_env.insert("CTX_AUTH_TOKEN".to_string(), token);
    }
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
            format!("gemini-login-{}", login_id),
            workdir,
            provider_env,
            Some(provider_accounts::GEMINI_CREDENTIAL_KIND_OAUTH_PERSONAL.to_string()),
            event_tx,
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
                    let mut map = state.providers.gemini_login_sessions.lock().await;
                    if let Some(entry) = map.get_mut(&login_id) {
                        entry.status = "success".to_string();
                        entry.account_id = registry.active_account_id.clone();
                        entry.error = None;
                    }
                    restarts::restart_gemini_providers_for_auth_change(
                        &state,
                        "gemini auth updated",
                    )
                    .await;
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
    Json(req): Json<GeminiLoginStartReq>,
) -> Result<Json<GeminiLoginStartResp>, (StatusCode, Json<ApiErrorResp>)> {
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
    Path(id): Path<String>,
) -> Result<Json<provider_accounts::GeminiLoginStatus>, (StatusCode, Json<ApiErrorResp>)> {
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

fn qwen_login_home(data_root: &StdPath, login_id: &str) -> PathBuf {
    data_root
        .join("providers")
        .join("qwen")
        .join("login-sessions")
        .join(login_id)
}

async fn monitor_qwen_login(state: Arc<AppState>, login_id: String, label: Option<String>) {
    let adapter = {
        let map = state.providers.adapters.lock().await;
        map.get("qwen").cloned()
    };
    let Some(adapter) = adapter else {
        let mut map = state.providers.qwen_login_sessions.lock().await;
        if let Some(entry) = map.get_mut(&login_id) {
            entry.status = "failed".to_string();
            entry.error = Some("provider adapter not available".to_string());
        }
        return;
    };

    let login_home = qwen_login_home(&state.core.data_root, &login_id);
    let workdir = login_home.join("workspace");
    if let Err(err) = tokio::fs::create_dir_all(&workdir).await {
        let mut map = state.providers.qwen_login_sessions.lock().await;
        if let Some(entry) = map.get_mut(&login_id) {
            entry.status = "failed".to_string();
            entry.error = Some(format!("failed to prepare login workspace: {err}"));
        }
        return;
    }
    let _ = tokio::fs::create_dir_all(login_home.join(".config")).await;
    let _ = tokio::fs::create_dir_all(login_home.join(".cache")).await;

    let mut provider_env = HashMap::new();
    provider_env.insert("CTX_DAEMON_URL".to_string(), state.core.daemon_url.clone());
    if let Some(token) = state.core.auth_token.clone() {
        provider_env.insert("CTX_AUTH_TOKEN".to_string(), token);
    }
    provider_env.insert(
        "CTX_DATA_ROOT".to_string(),
        state.core.data_root.to_string_lossy().to_string(),
    );
    provider_env.insert("HOME".to_string(), login_home.to_string_lossy().to_string());
    provider_env.insert(
        "XDG_CONFIG_HOME".to_string(),
        login_home.join(".config").to_string_lossy().to_string(),
    );
    provider_env.insert(
        "XDG_CACHE_HOME".to_string(),
        login_home.join(".cache").to_string_lossy().to_string(),
    );

    let (event_tx, mut event_rx) = mpsc::channel(64);
    let auth_result = adapter
        .authenticate_session(
            format!("qwen-login-{login_id}"),
            workdir,
            provider_env,
            Some(QWEN_OAUTH_AUTH_METHOD_ID.to_string()),
            event_tx,
        )
        .await;
    if let Err(err) = auth_result {
        let mut map = state.providers.qwen_login_sessions.lock().await;
        if let Some(entry) = map.get_mut(&login_id) {
            entry.status = "failed".to_string();
            entry.error = Some(logs::redact_sensitive(&err.to_string()));
        }
        let _ = tokio::fs::remove_dir_all(&login_home).await;
        return;
    }

    let oauth_path = login_home.join(provider_accounts::QWEN_OAUTH_CREDS_RELATIVE_PATH);
    let started_at = Instant::now();
    let timeout = qwen_login_timeout();
    let mut observed_auth_url = false;
    let mut observed_email = None::<String>;

    loop {
        let mut channel_disconnected = false;
        loop {
            match event_rx.try_recv() {
                Ok(event) => {
                    if let Some(auth_url) = extract_auth_url_from_value(&event.payload_json) {
                        observed_auth_url = true;
                        let mut map = state.providers.qwen_login_sessions.lock().await;
                        if let Some(entry) = map.get_mut(&login_id) {
                            entry.auth_url = Some(auth_url);
                        }
                    }
                    if observed_email.is_none() {
                        observed_email = first_email_from_value(&event.payload_json);
                    }
                    if matches!(event.event_type, ctx_core::models::SessionEventType::Error) {
                        let message = event
                            .payload_json
                            .get("message")
                            .and_then(serde_json::Value::as_str)
                            .map(logs::redact_sensitive)
                            .unwrap_or_else(|| "qwen authenticate reported an error".to_string());
                        let mut map = state.providers.qwen_login_sessions.lock().await;
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
                let mut map = state.providers.qwen_login_sessions.lock().await;
                if let Some(entry) = map.get_mut(&login_id) {
                    entry.status = "failed".to_string();
                    entry.error =
                        Some("captured oauth_creds.json is not a valid JSON object".to_string());
                }
                let _ = tokio::fs::remove_dir_all(&login_home).await;
                return;
            }

            let added = provider_accounts::add_qwen_account(
                &state.core.data_root,
                label.clone(),
                oauth_raw,
                observed_email.clone(),
            )
            .await;
            match added {
                Ok(registry) => {
                    let mut map = state.providers.qwen_login_sessions.lock().await;
                    if let Some(entry) = map.get_mut(&login_id) {
                        entry.status = "success".to_string();
                        entry.account_id = registry.active_account_id.clone();
                        entry.error = None;
                    }
                    restarts::restart_qwen_providers_for_auth_change(&state, "qwen auth updated")
                        .await;
                }
                Err(err) => {
                    let mut map = state.providers.qwen_login_sessions.lock().await;
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
            let mut map = state.providers.qwen_login_sessions.lock().await;
            if let Some(entry) = map.get_mut(&login_id) {
                entry.status = "failed".to_string();
                if entry.error.is_none() {
                    entry.error = Some(
                        "Qwen sign-in did not emit an OAuth URL in this environment.".to_string(),
                    );
                }
            }
            let _ = tokio::fs::remove_dir_all(&login_home).await;
            return;
        }

        if started_at.elapsed() >= timeout {
            let mut map = state.providers.qwen_login_sessions.lock().await;
            if let Some(entry) = map.get_mut(&login_id) {
                entry.status = "timeout".to_string();
                if entry.error.is_none() {
                    entry.error = Some("timed out waiting for Qwen OAuth completion".to_string());
                }
            }
            let _ = tokio::fs::remove_dir_all(&login_home).await;
            return;
        }

        tokio::time::sleep(QWEN_LOGIN_POLL_INTERVAL).await;
    }
}

pub(crate) async fn start_qwen_login(
    State(state): State<Arc<AppState>>,
    Json(req): Json<QwenLoginStartReq>,
) -> Result<Json<QwenLoginStartResp>, (StatusCode, Json<ApiErrorResp>)> {
    let login_id = uuid::Uuid::new_v4().to_string();
    {
        let mut map = state.providers.qwen_login_sessions.lock().await;
        map.insert(
            login_id.clone(),
            provider_accounts::QwenLoginStatus {
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
        monitor_qwen_login(state_clone, login_id_for_task, req.label).await;
    });

    Ok(Json(QwenLoginStartResp {
        login_id,
        auth_url: None,
    }))
}

pub(crate) async fn get_qwen_login(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<provider_accounts::QwenLoginStatus>, (StatusCode, Json<ApiErrorResp>)> {
    let map = state.providers.qwen_login_sessions.lock().await;
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

fn amp_login_home(data_root: &StdPath, login_id: &str) -> PathBuf {
    data_root
        .join("providers")
        .join("amp")
        .join("login-sessions")
        .join(login_id)
}

async fn monitor_amp_login(state: Arc<AppState>, login_id: String, label: Option<String>) {
    let adapter = {
        let map = state.providers.adapters.lock().await;
        map.get("amp").cloned()
    };
    let Some(adapter) = adapter else {
        let mut map = state.providers.amp_login_sessions.lock().await;
        if let Some(entry) = map.get_mut(&login_id) {
            entry.status = "failed".to_string();
            entry.error = Some("provider adapter not available".to_string());
        }
        return;
    };

    let login_home = amp_login_home(&state.core.data_root, &login_id);
    let workdir = login_home.join("workspace");
    if let Err(err) = tokio::fs::create_dir_all(&workdir).await {
        let mut map = state.providers.amp_login_sessions.lock().await;
        if let Some(entry) = map.get_mut(&login_id) {
            entry.status = "failed".to_string();
            entry.error = Some(format!("failed to prepare login workspace: {err}"));
        }
        return;
    }
    let amp_home = match provider_accounts::ensure_amp_runtime_home(&state.core.data_root).await {
        Ok(home) => home,
        Err(err) => {
            let mut map = state.providers.amp_login_sessions.lock().await;
            if let Some(entry) = map.get_mut(&login_id) {
                entry.status = "failed".to_string();
                entry.error = Some(format!("failed to prepare amp runtime home: {err}"));
            }
            let _ = tokio::fs::remove_dir_all(&login_home).await;
            return;
        }
    };

    let mut provider_env = HashMap::new();
    provider_env.insert("CTX_DAEMON_URL".to_string(), state.core.daemon_url.clone());
    if let Some(token) = state.core.auth_token.clone() {
        provider_env.insert("CTX_AUTH_TOKEN".to_string(), token);
    }
    provider_env.insert(
        "CTX_DATA_ROOT".to_string(),
        state.core.data_root.to_string_lossy().to_string(),
    );
    provider_env.insert("HOME".to_string(), amp_home.to_string_lossy().to_string());
    provider_env.insert(
        "XDG_CONFIG_HOME".to_string(),
        amp_home.join(".config").to_string_lossy().to_string(),
    );
    provider_env.insert(
        "XDG_CACHE_HOME".to_string(),
        amp_home.join(".cache").to_string_lossy().to_string(),
    );

    let (event_tx, mut event_rx) = mpsc::channel(64);
    let auth_result = adapter
        .authenticate_session(
            format!("amp-login-{login_id}"),
            workdir,
            provider_env,
            Some(AMP_BROWSER_AUTH_METHOD_ID.to_string()),
            event_tx,
        )
        .await;
    if let Err(err) = auth_result {
        let mut map = state.providers.amp_login_sessions.lock().await;
        if let Some(entry) = map.get_mut(&login_id) {
            entry.status = "failed".to_string();
            entry.error = Some(logs::redact_sensitive(&err.to_string()));
        }
        let _ = tokio::fs::remove_dir_all(&login_home).await;
        return;
    }

    let started_at = Instant::now();
    let timeout = amp_login_timeout();
    let mut observed_auth_url = false;
    let mut observed_email = None::<String>;

    loop {
        if started_at.elapsed() >= timeout {
            let mut map = state.providers.amp_login_sessions.lock().await;
            if let Some(entry) = map.get_mut(&login_id) {
                entry.status = "timeout".to_string();
                if entry.error.is_none() {
                    entry.error = Some("timed out waiting for Amp OAuth completion".to_string());
                }
            }
            let _ = tokio::fs::remove_dir_all(&login_home).await;
            return;
        }

        let event = match tokio::time::timeout(AMP_LOGIN_POLL_INTERVAL, event_rx.recv()).await {
            Ok(Some(event)) => event,
            Ok(None) => {
                let mut map = state.providers.amp_login_sessions.lock().await;
                if let Some(entry) = map.get_mut(&login_id) {
                    entry.status = "failed".to_string();
                    if entry.error.is_none() {
                        entry.error = Some(if observed_auth_url {
                            "Amp sign-in session ended before completion.".to_string()
                        } else {
                            "Amp sign-in did not emit an OAuth URL in this environment.".to_string()
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
            let mut map = state.providers.amp_login_sessions.lock().await;
            if let Some(entry) = map.get_mut(&login_id) {
                entry.auth_url = Some(auth_url);
            }
        }
        if observed_email.is_none() {
            observed_email = first_email_from_value(&event.payload_json);
        }

        if matches!(event.event_type, ctx_core::models::SessionEventType::Error) {
            let message = event
                .payload_json
                .get("message")
                .and_then(serde_json::Value::as_str)
                .map(logs::redact_sensitive)
                .unwrap_or_else(|| "amp authenticate reported an error".to_string());
            let mut map = state.providers.amp_login_sessions.lock().await;
            if let Some(entry) = map.get_mut(&login_id) {
                entry.status = "failed".to_string();
                entry.error = Some(message);
            }
            let _ = tokio::fs::remove_dir_all(&login_home).await;
            return;
        }

        if matches!(event.event_type, ctx_core::models::SessionEventType::Notice) {
            let code = event
                .payload_json
                .get("code")
                .or_else(|| event.payload_json.get("kind"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            if matches!(
                code,
                "auth_complete" | "auth_completed" | "auth_success" | "authenticated"
            ) {
                if let Err(err) = provider_accounts::upsert_amp_account(
                    &state.core.data_root,
                    label.clone(),
                    observed_email.clone(),
                )
                .await
                {
                    let mut map = state.providers.amp_login_sessions.lock().await;
                    if let Some(entry) = map.get_mut(&login_id) {
                        entry.status = "failed".to_string();
                        entry.error = Some(logs::redact_sensitive(&err.to_string()));
                    }
                    let _ = tokio::fs::remove_dir_all(&login_home).await;
                    return;
                }
                let mut map = state.providers.amp_login_sessions.lock().await;
                if let Some(entry) = map.get_mut(&login_id) {
                    entry.status = "success".to_string();
                    entry.auth_url = None;
                    entry.error = None;
                }
                restarts::restart_amp_providers_for_auth_change(&state, "amp auth updated").await;
                let _ = tokio::fs::remove_dir_all(&login_home).await;
                return;
            }
            if matches!(code, "auth_failed" | "auth_error" | "auth_required") {
                let message = event
                    .payload_json
                    .get("message")
                    .and_then(serde_json::Value::as_str)
                    .map(logs::redact_sensitive)
                    .unwrap_or_else(|| "Amp sign-in failed. Retry.".to_string());
                let mut map = state.providers.amp_login_sessions.lock().await;
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

pub(crate) async fn start_amp_login(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AmpLoginStartReq>,
) -> Result<Json<AmpLoginStartResp>, (StatusCode, Json<ApiErrorResp>)> {
    let label = req.label;
    let login_id = uuid::Uuid::new_v4().to_string();
    {
        let mut map = state.providers.amp_login_sessions.lock().await;
        map.insert(
            login_id.clone(),
            provider_accounts::AmpLoginStatus {
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
        monitor_amp_login(state_clone, login_id_for_task, label).await;
    });

    Ok(Json(AmpLoginStartResp {
        login_id,
        auth_url: None,
    }))
}

pub(crate) async fn get_amp_login(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<provider_accounts::AmpLoginStatus>, (StatusCode, Json<ApiErrorResp>)> {
    let map = state.providers.amp_login_sessions.lock().await;
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
    if let Some(token) = state.core.auth_token.clone() {
        provider_env.insert("CTX_AUTH_TOKEN".to_string(), token);
    }
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

        if matches!(event.event_type, ctx_core::models::SessionEventType::Error) {
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
                let mut map = state.providers.mistral_login_sessions.lock().await;
                if let Some(entry) = map.get_mut(&login_id) {
                    entry.status = "success".to_string();
                    entry.auth_url = None;
                    entry.error = None;
                }
                restarts::restart_mistral_providers_for_auth_change(&state, "mistral auth updated")
                    .await;
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
    Json(req): Json<MistralLoginStartReq>,
) -> Result<Json<MistralLoginStartResp>, (StatusCode, Json<ApiErrorResp>)> {
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
    Path(id): Path<String>,
) -> Result<Json<provider_accounts::MistralLoginStatus>, (StatusCode, Json<ApiErrorResp>)> {
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

async fn resolve_runtime_provider_command_from_config(
    data_root: &std::path::Path,
    provider_id: &str,
) -> anyhow::Result<Option<installer::ProviderRuntimeCommand>> {
    let cfg = installer::load_agent_server_config(data_root)
        .await
        .context("loading agent server config")?;
    installer::resolve_runtime_provider_command(&cfg, provider_id)
        .with_context(|| format!("resolving runtime command for {provider_id}"))
}

pub(super) fn should_attempt_claude_cli_bootstrap(
    resolution: &anyhow::Result<Option<installer::ProviderRuntimeCommand>>,
) -> bool {
    match resolution {
        Ok(Some(_)) => false,
        Ok(None) => true,
        Err(err) => {
            let msg = err.to_string();
            msg.contains("runtime_command_missing")
                || msg.contains("runtime_command_not_found")
                || msg.contains("runtime_command_not_absolute")
        }
    }
}

pub(super) async fn resolve_claude_setup_token_runtime_with_bootstrap<F, Fut>(
    data_root: &std::path::Path,
    bootstrap_managed_runtime: F,
) -> anyhow::Result<installer::ProviderRuntimeCommand>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<()>>,
{
    let first_resolution =
        resolve_runtime_provider_command_from_config(data_root, "claude-cli").await;
    if let Ok(Some(runtime_command)) = first_resolution.as_ref() {
        return Ok(runtime_command.clone());
    }

    if should_attempt_claude_cli_bootstrap(&first_resolution) {
        bootstrap_managed_runtime()
            .await
            .context("installing managed claude-cli runtime for subscription login")?;
        let resolved = resolve_runtime_provider_command_from_config(data_root, "claude-cli")
            .await?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "runtime_command_missing: provider=claude-cli (bundle claude-cli or configure an absolute runtime command)"
                )
            })?;
        return Ok(resolved);
    }

    match first_resolution {
        Ok(Some(runtime_command)) => Ok(runtime_command),
        Ok(None) => anyhow::bail!(
            "runtime_command_missing: provider=claude-cli (bundle claude-cli or configure an absolute runtime command)"
        ),
        Err(err) => Err(err),
    }
}

async fn resolve_claude_setup_token_runtime(
    state: &Arc<AppState>,
) -> anyhow::Result<installer::ProviderRuntimeCommand> {
    resolve_claude_setup_token_runtime_with_bootstrap(&state.core.data_root, || async {
        installer::install_provider(state.as_ref(), "claude-cli").await
    })
    .await
}

fn spawn_claude_setup_token_command(
    runtime: &installer::ProviderRuntimeCommand,
) -> anyhow::Result<ClaudeLoginSpawn> {
    let pty = NativePtySystem::default();
    let pair = pty
        .openpty(PtySize {
            rows: 30,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .context("opening pty for claude setup-token")?;

    let mut cmd = CommandBuilder::new(&runtime.command_abs_path);
    for arg in &runtime.args {
        cmd.arg(arg);
    }
    cmd.arg("setup-token");
    cmd.env("NO_COLOR", "1");
    cmd.env("TERM", "xterm-256color");

    let mut child = pair.slave.spawn_command(cmd).with_context(|| {
        format!(
            "spawning claude setup-token via {}",
            runtime.command_abs_path
        )
    })?;
    let killer = Arc::new(StdMutex::new(child.clone_killer()));
    drop(pair.slave);
    let writer = Arc::new(StdMutex::new(
        pair.master
            .take_writer()
            .context("taking pty writer for claude setup-token")?,
    ));

    let reader = pair
        .master
        .try_clone_reader()
        .context("cloning pty reader for claude setup-token")?;
    let (line_tx, line_rx) = mpsc::unbounded_channel();
    std::thread::spawn(move || {
        pump_claude_login_output(reader, line_tx);
    });

    let (exit_tx, exit_rx) = oneshot::channel();
    std::thread::spawn(move || {
        let result = child
            .wait()
            .context("waiting for claude setup-token process");
        let _ = exit_tx.send(result);
    });

    Ok(ClaudeLoginSpawn {
        line_rx,
        exit_rx,
        killer,
        writer,
    })
}

fn strip_ansi_sequences(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let mut idx = 0usize;
    while idx < chars.len() {
        let ch = chars[idx];
        if ch == '\u{1b}' {
            idx += 1;
            if idx < chars.len() {
                if chars[idx] == '[' {
                    idx += 1;
                    while idx < chars.len() {
                        let c = chars[idx];
                        idx += 1;
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                    continue;
                }
                if chars[idx] == ']' {
                    idx += 1;
                    let mut payload = String::new();
                    while idx < chars.len() {
                        let c = chars[idx];
                        if c == '\u{7}' {
                            idx += 1;
                            break;
                        }
                        if c == '\u{1b}' && (idx + 1) < chars.len() && chars[idx + 1] == '\\' {
                            idx += 2;
                            break;
                        }
                        payload.push(c);
                        idx += 1;
                    }
                    if let Some(url) = payload
                        .strip_prefix("8;;")
                        .filter(|value| !value.is_empty())
                    {
                        out.push(' ');
                        out.push_str(url);
                        out.push(' ');
                    }
                    continue;
                }
            }
            continue;
        }
        if ch != '\r' && ch != '\u{7}' {
            out.push(ch);
        }
        idx += 1;
    }
    out
}

pub(super) fn normalize_claude_login_line(line: &str) -> String {
    strip_ansi_sequences(line.trim_end_matches('\r'))
}

fn pump_claude_login_output<R>(mut reader: R, tx: mpsc::UnboundedSender<String>)
where
    R: std::io::Read,
{
    let mut pending = String::new();
    let mut buf = [0u8; 4096];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                pending.push_str(&String::from_utf8_lossy(&buf[..n]));
                while let Some(newline_idx) = pending.find('\n') {
                    let raw = pending[..newline_idx].to_string();
                    pending.drain(..=newline_idx);
                    if tx.send(normalize_claude_login_line(&raw)).is_err() {
                        return;
                    }
                }
            }
            Err(_) => break,
        }
    }
    if !pending.is_empty() {
        let _ = tx.send(normalize_claude_login_line(&pending));
    }
}

fn is_auth_url_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric()
        || matches!(
            ch,
            '-' | '.'
                | '_'
                | '~'
                | ':'
                | '/'
                | '?'
                | '#'
                | '['
                | ']'
                | '@'
                | '!'
                | '$'
                | '&'
                | '\''
                | '('
                | ')'
                | '*'
                | '+'
                | ','
                | ';'
                | '='
                | '%'
        )
}

fn should_continue_auth_url_after_break(current: &str, next_fragment: &str) -> bool {
    if next_fragment.is_empty() {
        return false;
    }
    if next_fragment.chars().any(|ch| !ch.is_ascii_alphanumeric()) {
        return true;
    }
    if next_fragment
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_digit())
    {
        return true;
    }
    matches!(
        current.chars().last(),
        Some('%' | ':' | '=' | '&' | '?' | '/' | '#' | '-' | '_')
    )
}

pub(super) fn extract_auth_url(text: &str) -> Option<String> {
    let normalized = strip_ansi_sequences(text);
    let chars: Vec<char> = normalized.chars().collect();
    let mut idx = 0usize;
    while idx < chars.len() {
        let remaining: String = chars[idx..].iter().collect();
        let scheme_offset = remaining
            .find("https://")
            .or_else(|| remaining.find("http://"))?;
        idx += scheme_offset;
        let mut end = idx;
        let mut candidate = String::new();
        while end < chars.len() {
            let ch = chars[end];
            if is_auth_url_char(ch) {
                candidate.push(ch);
                end += 1;
                continue;
            }
            if ch.is_whitespace() {
                let mut probe = end;
                while probe < chars.len() && chars[probe].is_whitespace() {
                    probe += 1;
                }
                if probe < chars.len() && is_auth_url_char(chars[probe]) {
                    let mut token_end = probe;
                    while token_end < chars.len() && is_auth_url_char(chars[token_end]) {
                        token_end += 1;
                    }
                    let next_fragment: String = chars[probe..token_end].iter().collect();
                    if should_continue_auth_url_after_break(&candidate, &next_fragment) {
                        end = probe;
                        continue;
                    }
                }
            }
            break;
        }
        let trimmed = candidate
            .trim_matches(|c: char| {
                matches!(
                    c,
                    '"' | '\'' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>' | ',' | ';' | '.'
                )
            })
            .to_string();
        if Url::parse(&trimmed).is_ok() {
            return Some(trimmed);
        }
        idx = end.saturating_add(1);
    }
    None
}

pub(super) fn auth_url_looks_complete(auth_url: &str) -> bool {
    let parsed = match Url::parse(auth_url) {
        Ok(value) => value,
        Err(_) => return false,
    };
    let maybe_redirect = parsed
        .query_pairs()
        .find_map(|(key, value)| (key == "redirect_uri").then_some(value.into_owned()));
    let Some(redirect_uri) = maybe_redirect else {
        return true;
    };
    let redirect = match Url::parse(&redirect_uri) {
        Ok(value) => value,
        Err(_) => return false,
    };
    let Some(host) = redirect.host_str() else {
        return false;
    };
    if !is_loopback_host(host) {
        return true;
    }
    redirect.port().is_some()
}

fn is_claude_setup_token_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'
}

fn is_setup_token_fragment(value: &str) -> bool {
    !value.is_empty() && value.chars().all(is_claude_setup_token_char)
}

fn leading_setup_token_fragment(value: &str) -> &str {
    let mut end = 0usize;
    for (idx, ch) in value.char_indices() {
        if is_claude_setup_token_char(ch) {
            end = idx + ch.len_utf8();
            continue;
        }
        break;
    }
    &value[..end]
}

fn is_setup_token_continuation_fragment(value: &str) -> bool {
    if !is_setup_token_fragment(value) {
        return false;
    }
    value
        .chars()
        .any(|ch| ch.is_ascii_digit() || ch == '-' || ch == '_')
}

fn trim_known_setup_token_prose_suffix(token: &str) -> String {
    const PROSE_CANONICAL: &str = "StorethistokensecurelyYouwontbeabletoseeitagain";
    const MIN_MATCH_LEN: usize = 5;

    let mut out = token.to_string();
    for phrase in [
        PROSE_CANONICAL,
        "storethistokensecurelyyouwontbeabletoseeitagain",
    ] {
        let max = std::cmp::min(out.len(), phrase.len());
        let mut truncate_at: Option<usize> = None;
        for len in (MIN_MATCH_LEN..=max).rev() {
            if out.ends_with(&phrase[..len]) {
                truncate_at = Some(out.len() - len);
                break;
            }
        }
        if let Some(idx) = truncate_at {
            out.truncate(idx);
        }
    }
    out
}

pub(super) fn extract_claude_setup_token(output: &str) -> Option<String> {
    let lines: Vec<&str> = output.lines().collect();
    for (idx, line) in lines.iter().enumerate() {
        let Some(start) = line.find("sk-ant-oat") else {
            continue;
        };
        let first_fragment = leading_setup_token_fragment(&line[start..]);
        if first_fragment.is_empty() {
            continue;
        }
        let mut token = first_fragment.to_string();
        for next in lines.iter().skip(idx + 1) {
            let trimmed = next.trim();
            if trimmed.is_empty() {
                break;
            }
            let fragment = leading_setup_token_fragment(trimmed);
            if !is_setup_token_continuation_fragment(fragment) {
                break;
            }
            token.push_str(fragment);
        }
        let cleaned = trim_known_setup_token_prose_suffix(&token);
        if cleaned.len() > 40 {
            return Some(cleaned);
        }
    }
    None
}

async fn start_claude_login_process(state: &Arc<AppState>) -> anyhow::Result<ClaudeLoginProcess> {
    let runtime = resolve_claude_setup_token_runtime(state).await?;
    let ClaudeLoginSpawn {
        line_rx: mut rx,
        exit_rx,
        killer,
        writer,
    } = spawn_claude_setup_token_command(&runtime)?;

    let mut buffered_lines = Vec::new();
    let mut auth_url = None;
    let mut transcript = String::new();
    let hard_deadline = Instant::now() + CLAUDE_LOGIN_URL_WAIT;
    let mut settle_deadline: Option<Instant> = None;
    loop {
        let now = Instant::now();
        let remaining = if let Some(settle) = settle_deadline {
            std::cmp::min(
                hard_deadline.saturating_duration_since(now),
                settle.saturating_duration_since(now),
            )
        } else {
            hard_deadline.saturating_duration_since(now)
        };
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, rx.recv()).await {
            Ok(Some(line)) => {
                transcript.push_str(&line);
                transcript.push('\n');
                buffered_lines.push(line);
                if let Some(candidate) = extract_auth_url(&transcript) {
                    auth_url = Some(candidate);
                    settle_deadline = Some(Instant::now() + CLAUDE_LOGIN_URL_SETTLE_WAIT);
                }
            }
            Ok(None) => break,
            Err(_) => break,
        }
    }

    Ok(ClaudeLoginProcess {
        line_rx: rx,
        input_rx: {
            let (_tx, rx) = mpsc::unbounded_channel();
            rx
        },
        buffered_lines,
        auth_url,
        exit_rx,
        killer,
        writer,
    })
}

async fn append_claude_login_line(
    state: &Arc<AppState>,
    login_id: &str,
    observed_auth_url: &mut Option<String>,
    transcript: &mut String,
    line: String,
) {
    let needs_auth_url_upgrade = match observed_auth_url.as_deref() {
        None => true,
        Some(url) => !auth_url_looks_complete(url),
    };
    if needs_auth_url_upgrade {
        if let Some(candidate) = extract_auth_url(&line) {
            let should_replace = match observed_auth_url.as_ref() {
                None => true,
                Some(current) => {
                    !auth_url_looks_complete(current) && candidate.len() >= current.len()
                }
            };
            if should_replace {
                *observed_auth_url = Some(candidate);
            }
        }
    }
    transcript.push_str(&line);
    transcript.push('\n');
    let needs_auth_url_upgrade = match observed_auth_url.as_deref() {
        None => true,
        Some(url) => !auth_url_looks_complete(url),
    };
    if needs_auth_url_upgrade {
        if let Some(candidate) = extract_auth_url(transcript) {
            let should_replace = match observed_auth_url.as_ref() {
                None => true,
                Some(current) => {
                    !auth_url_looks_complete(current) && candidate.len() >= current.len()
                }
            };
            if should_replace {
                *observed_auth_url = Some(candidate);
            }
        }
    }
    if let Some(url) = observed_auth_url.clone() {
        let mut map = state.providers.claude_login_sessions.lock().await;
        if let Some(entry) = map.get_mut(login_id) {
            entry.auth_url = Some(url);
        }
    }
}

fn format_claude_exit_status(status: &portable_pty::ExitStatus) -> String {
    if let Some(signal) = status.signal() {
        return format!("signal {signal}");
    }
    status.exit_code().to_string()
}

async fn kill_claude_login_process(
    killer: Arc<StdMutex<Box<dyn portable_pty::ChildKiller + Send + Sync>>>,
) -> anyhow::Result<()> {
    tokio::task::spawn_blocking(move || {
        let mut guard = killer
            .lock()
            .map_err(|_| anyhow::anyhow!("claude setup-token killer lock poisoned"))?;
        guard.kill().context("killing claude setup-token process")
    })
    .await
    .context("joining claude setup-token kill task")?
}

async fn write_claude_login_input(
    writer: Arc<StdMutex<Box<dyn Write + Send>>>,
    input: &str,
) -> anyhow::Result<()> {
    let mut payload = input.trim().to_string();
    if payload.is_empty() {
        bail!("callback code is required");
    }
    payload.push('\n');
    tokio::task::spawn_blocking(move || {
        let mut guard = writer
            .lock()
            .map_err(|_| anyhow::anyhow!("claude setup-token writer lock poisoned"))?;
        guard
            .write_all(payload.as_bytes())
            .context("writing callback code to claude setup-token")?;
        guard
            .flush()
            .context("flushing callback code to claude setup-token")
    })
    .await
    .context("joining callback input writer task")?
}

pub(super) async fn read_trailing_claude_login_lines(
    line_rx: &mut mpsc::UnboundedReceiver<String>,
    grace: Duration,
) -> Vec<String> {
    let mut lines = Vec::new();
    let mut deadline = Instant::now() + grace;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, line_rx.recv()).await {
            Ok(Some(line)) => {
                lines.push(line);
                deadline = Instant::now() + grace;
            }
            Ok(None) => break,
            Err(_) => break,
        }
    }
    lines
}

async fn monitor_claude_login(
    state: Arc<AppState>,
    login_id: String,
    label: Option<String>,
    mut login: ClaudeLoginProcess,
) {
    let mut transcript = String::new();
    let mut observed_auth_url = login.auth_url.clone();
    let mut output_closed = false;
    let auth_url_deadline = Instant::now() + CLAUDE_LOGIN_NO_AUTH_URL_TIMEOUT;
    let mut completion_deadline = observed_auth_url
        .as_ref()
        .map(|_| Instant::now() + CLAUDE_LOGIN_COMPLETION_TIMEOUT);

    for line in std::mem::take(&mut login.buffered_lines) {
        let had_auth_url = observed_auth_url.is_some();
        append_claude_login_line(
            &state,
            &login_id,
            &mut observed_auth_url,
            &mut transcript,
            line,
        )
        .await;
        if !had_auth_url && observed_auth_url.is_some() {
            completion_deadline = Some(Instant::now() + CLAUDE_LOGIN_COMPLETION_TIMEOUT);
        }
    }

    let mut exit_result: Option<anyhow::Result<portable_pty::ExitStatus>> = None;
    let mut timeout_error: Option<String> = None;

    loop {
        let deadline = completion_deadline.unwrap_or(auth_url_deadline);
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            timeout_error = Some(if observed_auth_url.is_some() {
                "claude setup-token timed out waiting for browser sign-in completion".to_string()
            } else {
                "claude setup-token did not emit an authentication URL".to_string()
            });
            break;
        }
        let timeout_future = tokio::time::sleep(remaining);
        tokio::pin!(timeout_future);

        tokio::select! {
            maybe_line = login.line_rx.recv(), if !output_closed => {
                match maybe_line {
                    Some(line) => {
                        let had_auth_url = observed_auth_url.is_some();
                        append_claude_login_line(
                            &state,
                            &login_id,
                            &mut observed_auth_url,
                            &mut transcript,
                            line,
                        )
                        .await;
                        if !had_auth_url && observed_auth_url.is_some() {
                            completion_deadline = Some(Instant::now() + CLAUDE_LOGIN_COMPLETION_TIMEOUT);
                        }
                    }
                    None => {
                        output_closed = true;
                    }
                }
            }
            exit = &mut login.exit_rx => {
                exit_result = Some(match exit {
                    Ok(result) => result,
                    Err(err) => Err(anyhow::anyhow!("claude setup-token exit channel closed: {err}")),
                });
                break;
            }
            maybe_input = login.input_rx.recv() => {
                if let Some(input) = maybe_input {
                    if let Err(err) = write_claude_login_input(Arc::clone(&login.writer), &input).await {
                        timeout_error = Some(format!("failed to submit callback code to claude setup-token: {err}"));
                        break;
                    }
                }
            }
            _ = &mut timeout_future => {
                timeout_error = Some(if observed_auth_url.is_some() {
                    "claude setup-token timed out waiting for browser sign-in completion".to_string()
                } else {
                    "claude setup-token did not emit an authentication URL".to_string()
                });
                break;
            }
        }
    }

    if exit_result.is_some() {
        // PTY reader runs on a separate thread, so process exit can arrive before final
        // buffered lines. Keep consuming briefly before token parsing.
        for line in
            read_trailing_claude_login_lines(&mut login.line_rx, CLAUDE_LOGIN_EXIT_GRACE_WAIT).await
        {
            append_claude_login_line(
                &state,
                &login_id,
                &mut observed_auth_url,
                &mut transcript,
                line,
            )
            .await;
        }
    } else {
        while let Ok(line) = login.line_rx.try_recv() {
            append_claude_login_line(
                &state,
                &login_id,
                &mut observed_auth_url,
                &mut transcript,
                line,
            )
            .await;
        }
    }

    if timeout_error.is_some() {
        if let Err(err) = kill_claude_login_process(Arc::clone(&login.killer)).await {
            let suffix = format!("; failed to terminate setup-token process cleanly: {err}");
            timeout_error = Some(match timeout_error.take() {
                Some(base) => format!("{base}{suffix}"),
                None => suffix,
            });
        }
        if exit_result.is_none() {
            if let Ok(exit) =
                tokio::time::timeout(CLAUDE_LOGIN_EXIT_GRACE_WAIT, &mut login.exit_rx).await
            {
                exit_result = Some(match exit {
                    Ok(result) => result,
                    Err(err) => Err(anyhow::anyhow!(
                        "claude setup-token exit channel closed: {err}"
                    )),
                });
            }
        }
    }

    let mut final_status = "failed".to_string();
    let mut final_error: Option<String> = timeout_error;
    let mut final_account_id: Option<String> = None;

    if final_error.is_none() {
        match exit_result {
            Some(Ok(exit)) if exit.success() => match extract_claude_setup_token(&transcript) {
                Some(setup_token) => {
                    match provider_accounts::add_claude_account(
                        &state.core.data_root,
                        label.clone(),
                        setup_token,
                    )
                    .await
                    {
                        Ok(registry) => {
                            final_status = "success".to_string();
                            final_account_id = registry.active_account_id;
                            restarts::restart_claude_providers_for_auth_change(
                                &state,
                                "claude auth updated",
                            )
                            .await;
                        }
                        Err(err) => {
                            final_error = Some(logs::redact_sensitive(&err.to_string()));
                        }
                    }
                }
                None => {
                    final_error = Some(
                        "claude setup-token completed but no setup token was detected".to_string(),
                    );
                }
            },
            Some(Ok(exit)) => {
                final_error = Some(format!(
                    "claude setup-token exited with status {}",
                    format_claude_exit_status(&exit)
                ));
            }
            Some(Err(err)) => {
                final_error = Some(format!("waiting for claude setup-token failed: {err}"));
            }
            None => {
                final_error = Some(
                    "claude setup-token monitor ended before process exit was observed".to_string(),
                );
            }
        }
    }

    {
        let mut map = state.providers.claude_login_sessions.lock().await;
        if let Some(entry) = map.get_mut(&login_id) {
            entry.status = final_status;
            entry.account_id = final_account_id;
            entry.error = final_error;
            if entry.auth_url.is_none() {
                entry.auth_url = observed_auth_url;
            }
        }
    }
    {
        let mut map = state.providers.claude_login_inputs.lock().await;
        map.remove(&login_id);
    }
}

async fn start_codex_login_process(account_dir: &PathBuf) -> anyhow::Result<CodexLoginProcess> {
    let mut child = spawn_codex_app_server(account_dir)?;
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
    let status = match completion {
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
        let entry = provider_accounts::CodexAccountEntry {
            id: account_id.clone(),
            label,
            kind: provider_accounts::CODEX_CREDENTIAL_KIND_OAUTH.to_string(),
            email,
            plan_type,
            created_at: Utc::now(),
            last_used_at: Some(Utc::now()),
            secret_ref: None,
            endpoint_profile: provider_accounts::CodexEndpointProfile::default(),
        };
        if provider_accounts::upsert_codex_account(&state.core.data_root, entry)
            .await
            .is_ok()
        {
            let _ = provider_accounts::ingest_codex_account_auth_to_secret_store(
                &state.core.data_root,
                &account_id,
            )
            .await;
            let _ = provider_accounts::set_active_codex_account(
                &state.core.data_root,
                Some(account_id.clone()),
            )
            .await;
            restarts::restart_codex_providers_for_auth_change(&state, "codex auth updated").await;
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

fn spawn_codex_app_server(account_dir: &PathBuf) -> anyhow::Result<tokio::process::Child> {
    let mut cmd = Command::new("codex");
    cmd.args(["-s", "read-only", "-a", "untrusted", "app-server"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    cmd.env("CODEX_HOME", account_dir);
    cmd.spawn().context("spawning codex app-server")
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
