use super::*;
use ctx_core::provider_policy::CODEX_APP_SERVER_ARGS;
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use std::path::PathBuf;
use std::io::Write;
use std::sync::Mutex as StdMutex;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::oneshot;

mod browser;
mod codex;
mod kimi;
mod mistral;

pub(crate) use browser::{
    get_amp_login, get_gemini_login, get_qwen_login, start_amp_login, start_gemini_login,
    start_qwen_login,
};
pub(crate) use codex::{complete_codex_login, get_codex_login, start_codex_login};
pub(crate) use kimi::{get_kimi_login, start_kimi_login};
pub(crate) use mistral::{get_mistral_login, start_mistral_login};

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
pub(crate) struct ClaudeLoginCodeReq {
    login_code: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct ClaudeLoginCodeResp {
    accepted: bool,
}

struct ClaudeLoginProcess {
    line_rx: mpsc::UnboundedReceiver<String>,
    input_rx: mpsc::UnboundedReceiver<String>,
    buffered_lines: Vec<String>,
    auth_url: Option<String>,
    manual_open_required: bool,
    exit_rx: oneshot::Receiver<anyhow::Result<portable_pty::ExitStatus>>,
    killer: Arc<StdMutex<Box<dyn portable_pty::ChildKiller + Send + Sync>>>,
    writer: Arc<StdMutex<Box<dyn Write + Send>>>,
    _browser_open_shim_dir: tempfile::TempDir,
}

struct ClaudeLoginSpawn {
    line_rx: mpsc::UnboundedReceiver<String>,
    exit_rx: oneshot::Receiver<anyhow::Result<portable_pty::ExitStatus>>,
    killer: Arc<StdMutex<Box<dyn portable_pty::ChildKiller + Send + Sync>>>,
    writer: Arc<StdMutex<Box<dyn Write + Send>>>,
    browser_open_shim_dir: tempfile::TempDir,
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
const CLAUDE_BROWSER_OPEN_MARKER: &str = "CTX_CLAUDE_AUTH_URL:";

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

fn claude_login_requires_manual_browser_open(text: &str) -> bool {
    text.to_ascii_lowercase()
        .contains("browser didn't open? use the url below to sign in")
}

pub(crate) async fn start_claude_login(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ClaudeLoginStartReq>,
) -> Result<Json<ClaudeLoginStartResp>, (StatusCode, Json<ApiErrorResp>)> {
    let login_id = uuid::Uuid::new_v4().to_string();
    let label = req.label;
    let mut login = start_claude_login_process(&state).await.map_err(|e| {
        let msg = format!("{e:#}");
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
        status: if login.manual_open_required {
            "manual_open_required".to_string()
        } else {
            "pending".to_string()
        },
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

pub(crate) async fn submit_claude_login_code(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<ClaudeLoginCodeReq>,
) -> Result<Json<ClaudeLoginCodeResp>, (StatusCode, Json<ApiErrorResp>)> {
    let login_code = req.login_code.trim();
    if login_code.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "login_code is required".to_string(),
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
        if status.status != "pending" && status.status != "manual_open_required" {
            return Err((
                StatusCode::CONFLICT,
                Json(ApiErrorResp {
                    error: "login is no longer accepting setup-token prompt codes".to_string(),
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
                error: "login session is not accepting setup-token prompt codes".to_string(),
            }),
        )
    })?;
    tx.send(login_code.to_string()).map_err(|_| {
        (
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "login session is no longer accepting setup-token prompt codes".to_string(),
            }),
        )
    })?;
    Ok(Json(ClaudeLoginCodeResp { accepted: true }))
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

pub(super) async fn resolve_runtime_provider_command_from_config(
    data_root: &std::path::Path,
    provider_id: &str,
) -> anyhow::Result<Option<installer::ProviderRuntimeCommand>> {
    let cfg = installer::load_agent_server_config(data_root)
        .await
        .context("loading agent server config")?;
    installer::resolve_runtime_provider_command(&cfg, provider_id)
        .with_context(|| format!("resolving runtime command for {provider_id}"))
}

pub(super) async fn resolve_provider_login_command_from_config(
    data_root: &std::path::Path,
    provider_id: &str,
) -> anyhow::Result<Option<std::path::PathBuf>> {
    let cfg = installer::load_agent_server_config(data_root)
        .await
        .context("loading agent server config")?;
    installer::resolve_provider_login_command(&cfg, provider_id)
        .with_context(|| format!("resolving prepared login command for {provider_id}"))
}

pub(super) async fn resolve_claude_login_runtime_from_config(
    data_root: &std::path::Path,
) -> anyhow::Result<installer::ProviderRuntimeCommand> {
    if let Some(login_command) = resolve_provider_login_command_from_config(data_root, "claude-cli").await? {
        return Ok(installer::ProviderRuntimeCommand {
            provider_id: "claude-cli".to_string(),
            command_abs_path: login_command.to_string_lossy().to_string(),
            args: Vec::new(),
            dependencies: Vec::new(),
            source: installer::ProviderRuntimeCommandSource::UserOverride,
        });
    }

    let host_claude = which::which("claude").map_err(|_| {
        anyhow::anyhow!(
            "runtime_command_missing: provider=claude-cli (install `claude` on PATH to enable managed Claude setup-token login)"
        )
    })?;
    let command_abs_path = std::fs::canonicalize(&host_claude).unwrap_or(host_claude);
    Ok(installer::ProviderRuntimeCommand {
        provider_id: "claude-cli".to_string(),
        command_abs_path: command_abs_path.to_string_lossy().to_string(),
        args: Vec::new(),
        dependencies: Vec::new(),
        source: installer::ProviderRuntimeCommandSource::UserOverride,
    })
}

async fn resolve_claude_login_runtime(
    state: &Arc<AppState>,
) -> anyhow::Result<installer::ProviderRuntimeCommand> {
    resolve_claude_login_runtime_from_config(&state.core.data_root).await
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

fn matches_auth_url_scheme_from(chars: &[char], start: usize, scheme: &str) -> bool {
    let mut idx = start;
    for expected in scheme.chars() {
        while idx < chars.len() && chars[idx].is_whitespace() {
            idx += 1;
        }
        if idx >= chars.len() || chars[idx].to_ascii_lowercase() != expected {
            return false;
        }
        idx += 1;
    }
    true
}

fn find_auth_url_start(chars: &[char], from_idx: usize) -> Option<usize> {
    let mut idx = from_idx;
    while idx < chars.len() {
        if chars[idx].eq_ignore_ascii_case(&'h')
            && (matches_auth_url_scheme_from(chars, idx, "https://")
                || matches_auth_url_scheme_from(chars, idx, "http://"))
        {
            return Some(idx);
        }
        idx += 1;
    }
    None
}

fn auth_scheme_can_continue_across_break(current: &str, next_fragment: &str) -> bool {
    let compact_current: String = current
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    let compact_next: String = next_fragment
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    if compact_current.is_empty() || compact_next.is_empty() {
        return false;
    }
    let combined = format!("{compact_current}{compact_next}");
    ["https://", "http://"]
        .into_iter()
        .any(|scheme| scheme.starts_with(&combined))
}

fn fragment_starts_full_auth_url(value: &str) -> bool {
    let compact = value
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    compact.starts_with("https://") || compact.starts_with("http://")
}

fn should_continue_auth_url_after_break(current: &str, next_fragment: &str) -> bool {
    if next_fragment.is_empty() {
        return false;
    }
    if fragment_starts_full_auth_url(next_fragment) && Url::parse(current).is_ok() {
        return false;
    }
    if auth_scheme_can_continue_across_break(current, next_fragment) {
        return true;
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
        idx = find_auth_url_start(&chars, idx)?;
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
        if let Ok(parsed) = Url::parse(&trimmed) {
            if matches!(parsed.scheme(), "http" | "https") {
                return Some(trimmed);
            }
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
        return false;
    }
    redirect.port().is_some()
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

fn create_claude_browser_open_shim() -> anyhow::Result<(tempfile::TempDir, PathBuf)> {
    let temp_dir = tempfile::Builder::new()
        .prefix("ctx-claude-browser-open-")
        .tempdir()
        .context("creating Claude browser-open shim tempdir")?;
    let script_path = temp_dir.path().join("open-browser");
    std::fs::write(
        &script_path,
        format!(
            "#!/bin/sh\nurl=\"${{1:-}}\"\nif [ -n \"$url\" ]; then\n  printf '{CLAUDE_BROWSER_OPEN_MARKER}%s\\n' \"$url\"\nfi\nif [ -z \"$url\" ]; then\n  exit 1\nfi\nif command -v open >/dev/null 2>&1; then\n  exec open \"$url\"\nfi\nif command -v xdg-open >/dev/null 2>&1; then\n  exec xdg-open \"$url\"\nfi\nexit 1\n"
        ),
    )
    .with_context(|| format!("writing Claude browser-open shim {}", script_path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o700))
            .with_context(|| {
                format!(
                    "marking Claude browser-open shim executable {}",
                    script_path.display()
                )
            })?;
    }
    Ok((temp_dir, script_path))
}

fn spawn_claude_setup_token_command(
    runtime: &installer::ProviderRuntimeCommand,
) -> anyhow::Result<ClaudeLoginSpawn> {
    let pty = NativePtySystem::default();
    let pair = pty
        .openpty(PtySize {
            rows: 40,
            cols: 400,
            pixel_width: 0,
            pixel_height: 0,
        })
        .context("opening pty for claude setup-token")?;

    let mut cmd = CommandBuilder::new(&runtime.command_abs_path);
    let (browser_open_shim_dir, browser_open_shim_path) = create_claude_browser_open_shim()?;
    for arg in &runtime.args {
        cmd.arg(arg);
    }
    cmd.arg("setup-token");
    cmd.env("NO_COLOR", "1");
    cmd.env("TERM", "xterm-256color");
    cmd.env("BROWSER", browser_open_shim_path);

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
        let result = child.wait().context("waiting for claude setup-token process");
        let _ = exit_tx.send(result);
    });

    Ok(ClaudeLoginSpawn {
        line_rx,
        exit_rx,
        killer,
        writer,
        browser_open_shim_dir,
    })
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
    let runtime = resolve_claude_login_runtime(state).await?;
    let ClaudeLoginSpawn {
        line_rx: mut rx,
        exit_rx,
        killer,
        writer,
        browser_open_shim_dir,
    } = spawn_claude_setup_token_command(&runtime)?;

    let mut buffered_lines = Vec::new();
    let mut auth_url = None;
    let mut manual_open_required = false;
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
                manual_open_required |= claude_login_requires_manual_browser_open(&line);
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

    let (_tx, input_rx) = mpsc::unbounded_channel();
    Ok(ClaudeLoginProcess {
        line_rx: rx,
        input_rx,
        buffered_lines,
        auth_url,
        manual_open_required,
        exit_rx,
        killer,
        writer,
        _browser_open_shim_dir: browser_open_shim_dir,
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
        bail!("setup-token prompt code is required");
    }
    payload.push('\n');
    tokio::task::spawn_blocking(move || {
        let mut guard = writer
            .lock()
            .map_err(|_| anyhow::anyhow!("claude setup-token writer lock poisoned"))?;
        guard
            .write_all(payload.as_bytes())
            .context("writing setup-token prompt code to claude setup-token")?;
        guard
            .flush()
            .context("flushing setup-token prompt code to claude setup-token")
    })
    .await
    .context("joining setup-token input writer task")?
}

async fn monitor_claude_login(
    state: Arc<AppState>,
    login_id: String,
    label: Option<String>,
    mut login: ClaudeLoginProcess,
) {
    let mut transcript = String::new();
    let mut observed_auth_url = login.auth_url.clone();
    let mut observed_manual_open_required = login.manual_open_required;
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
        if !observed_manual_open_required && claude_login_requires_manual_browser_open(&transcript) {
            observed_manual_open_required = true;
            let mut map = state.providers.claude_login_sessions.lock().await;
            if let Some(entry) = map.get_mut(&login_id) {
                if entry.status == "pending" {
                    entry.status = "manual_open_required".to_string();
                }
            }
        }
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
                        let line_requires_manual_open =
                            claude_login_requires_manual_browser_open(&line);
                        append_claude_login_line(
                            &state,
                            &login_id,
                            &mut observed_auth_url,
                            &mut transcript,
                            line,
                        )
                        .await;
                        if line_requires_manual_open && !observed_manual_open_required {
                            observed_manual_open_required = true;
                            let mut map = state.providers.claude_login_sessions.lock().await;
                            if let Some(entry) = map.get_mut(&login_id) {
                                if entry.status == "pending" {
                                    entry.status = "manual_open_required".to_string();
                                }
                            }
                        }
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
                        timeout_error = Some(format!("failed to submit setup-token prompt code to claude setup-token: {err}"));
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
