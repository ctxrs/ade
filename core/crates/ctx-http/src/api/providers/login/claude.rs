use super::*;
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use std::sync::Mutex as StdMutex;
use tokio::sync::oneshot;

const CLAUDE_LOGIN_NO_AUTH_URL_TIMEOUT: Duration = Duration::from_secs(8);
const CLAUDE_LOGIN_URL_SETTLE_WAIT: Duration = Duration::from_millis(500);
const CLAUDE_LOGIN_COMPLETION_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const CLAUDE_LOGIN_EXIT_GRACE_WAIT: Duration = Duration::from_millis(400);
const CLAUDE_BROWSER_OPEN_MARKER: &str = "CTX_CLAUDE_AUTH_URL:";
const CLAUDE_BROWSER_AUTH_TIER: &str = "provider-browser-auth";
const CLAUDE_UNSUPPORTED_MANUAL_FALLBACK_ERROR: &str =
    "Claude setup-token fell back to manual code entry, which ctx does not support. Browser launch likely failed before Claude could receive the localhost callback.";

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

struct ClaudeLoginProcess {
    line_rx: mpsc::UnboundedReceiver<String>,
    buffered_lines: Vec<String>,
    auth_url: Option<String>,
    browser_open_capture_path: PathBuf,
    exit_rx: oneshot::Receiver<anyhow::Result<portable_pty::ExitStatus>>,
    killer: Arc<StdMutex<Box<dyn portable_pty::ChildKiller + Send + Sync>>>,
    _browser_open_shim_dir: tempfile::TempDir,
}

struct ClaudeLoginSpawn {
    line_rx: mpsc::UnboundedReceiver<String>,
    exit_rx: oneshot::Receiver<anyhow::Result<portable_pty::ExitStatus>>,
    killer: Arc<StdMutex<Box<dyn portable_pty::ChildKiller + Send + Sync>>>,
    browser_open_capture_path: PathBuf,
    browser_open_shim_dir: tempfile::TempDir,
}

fn claude_login_hit_unsupported_manual_fallback(text: &str) -> bool {
    text.to_ascii_lowercase()
        .contains("browser didn't open? use the url below to sign in")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ClaudeAuthUrlSource {
    BrowserOpenCapture,
    BrowserOpenMarker,
    Transcript,
}

fn extract_claude_browser_open_marker_url(text: &str) -> Option<String> {
    for raw_line in text.lines() {
        let line = raw_line.trim();
        let Some(marker_idx) = line.find(CLAUDE_BROWSER_OPEN_MARKER) else {
            continue;
        };
        let candidate = line[marker_idx + CLAUDE_BROWSER_OPEN_MARKER.len()..].trim();
        if candidate.is_empty() {
            continue;
        }
        if Url::parse(candidate).is_ok() {
            return Some(candidate.to_string());
        }
        if let Some(parsed) = extract_auth_url(candidate) {
            return Some(parsed);
        }
    }
    None
}

fn extract_preferred_claude_auth_url(text: &str) -> Option<(String, ClaudeAuthUrlSource)> {
    if let Some(marker_url) = extract_claude_browser_open_marker_url(text) {
        return Some((marker_url, ClaudeAuthUrlSource::BrowserOpenMarker));
    }
    extract_auth_url(text).map(|value| (value, ClaudeAuthUrlSource::Transcript))
}

fn should_replace_observed_claude_auth_url(
    current: Option<&str>,
    candidate: &str,
    source: ClaudeAuthUrlSource,
) -> bool {
    match source {
        ClaudeAuthUrlSource::BrowserOpenCapture | ClaudeAuthUrlSource::BrowserOpenMarker => {
            current != Some(candidate)
        }
        ClaudeAuthUrlSource::Transcript => match current {
            None => true,
            Some(existing) => {
                !auth_url_looks_complete(existing) && candidate.len() >= existing.len()
            }
        },
    }
}

fn read_claude_browser_open_capture_url(path: &std::path::Path) -> Option<String> {
    let raw = std::fs::read_to_string(path).ok()?;
    let candidate = raw.trim();
    if candidate.is_empty() {
        return None;
    }
    if Url::parse(candidate).is_ok() {
        return Some(candidate.to_string());
    }
    extract_auth_url(candidate)
}

fn upgrade_claude_auth_url_from_capture_path(
    observed_auth_url: &mut Option<String>,
    capture_path: &std::path::Path,
) {
    let Some(candidate) = read_claude_browser_open_capture_url(capture_path) else {
        return;
    };
    if should_replace_observed_claude_auth_url(
        observed_auth_url.as_deref(),
        &candidate,
        ClaudeAuthUrlSource::BrowserOpenCapture,
    ) {
        *observed_auth_url = Some(candidate);
    }
}

pub(crate) async fn start_claude_login(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ClaudeLoginStartReq>,
) -> Result<Json<ClaudeLoginStartResp>, (StatusCode, Json<ApiErrorResp>)> {
    let login_id = uuid::Uuid::new_v4().to_string();
    let label = req.label;
    let login = start_claude_login_process(&state).await.map_err(|e| {
        let msg = format!("{e:#}");
        let status = if msg.contains("runtime_command_") {
            StatusCode::BAD_REQUEST
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };
        (status, Json(ApiErrorResp { error: msg }))
    })?;
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

pub(crate) async fn resolve_claude_login_runtime_from_config(
    data_root: &std::path::Path,
) -> anyhow::Result<installer::ProviderRuntimeCommand> {
    if let Some(login_command) =
        resolve_provider_login_command_from_config(data_root, "claude-cli").await?
    {
        return Ok(installer::ProviderRuntimeCommand {
            provider_id: "claude-cli".to_string(),
            command_abs_path: login_command.to_string_lossy().to_string(),
            args: Vec::new(),
            dependencies: Vec::new(),
            source: installer::ProviderRuntimeCommandSource::UserOverride,
        });
    }

    if let Some(runtime_command) =
        resolve_runtime_provider_command_from_config(data_root, "claude-cli").await?
    {
        return Ok(runtime_command);
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

fn claude_login_should_skip_browser_open(raw_tier: Option<&str>) -> bool {
    matches!(
        raw_tier.map(str::trim),
        Some(tier) if tier.eq_ignore_ascii_case(CLAUDE_BROWSER_AUTH_TIER)
    )
}

fn claude_browser_open_shim_script(skip_browser_open: bool) -> &'static str {
    if skip_browser_open {
        r#"#!/bin/sh
url="${1:-}"
capture_path="${CTX_CLAUDE_AUTH_URL_CAPTURE_PATH:-}"
if [ -n "$url" ] && [ -n "$capture_path" ]; then
  printf '%s\n' "$url" > "$capture_path"
fi
exit 0
"#
    } else {
        r#"#!/bin/sh
url="${1:-}"
capture_path="${CTX_CLAUDE_AUTH_URL_CAPTURE_PATH:-}"
if [ -n "$url" ] && [ -n "$capture_path" ]; then
  printf '%s\n' "$url" > "$capture_path"
fi
if [ -z "$url" ]; then
  exit 1
fi
if command -v open >/dev/null 2>&1; then
  exec open "$url"
fi
if command -v xdg-open >/dev/null 2>&1; then
  exec xdg-open "$url"
fi
exit 1
"#
    }
}

fn create_claude_browser_open_shim() -> anyhow::Result<(tempfile::TempDir, PathBuf, PathBuf)> {
    let temp_dir = tempfile::Builder::new()
        .prefix("ctx-claude-browser-open-")
        .tempdir()
        .context("creating Claude browser-open shim tempdir")?;
    let script_path = temp_dir.path().join("open-browser");
    let capture_path = temp_dir.path().join("auth-url");
    let skip_browser_open =
        claude_login_should_skip_browser_open(std::env::var("CTX_E2E_TIER").ok().as_deref());
    // This script is the browser-open golden path for Claude setup-token, so
    // it must stay POSIX `sh` compatible.
    let script_body = claude_browser_open_shim_script(skip_browser_open);
    std::fs::write(&script_path, script_body)
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
    Ok((temp_dir, script_path, capture_path))
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
    let (browser_open_shim_dir, browser_open_shim_path, browser_open_capture_path) =
        create_claude_browser_open_shim()?;
    for arg in &runtime.args {
        cmd.arg(arg);
    }
    cmd.arg("setup-token");
    cmd.env("NO_COLOR", "1");
    cmd.env("TERM", "xterm-256color");
    cmd.env("BROWSER", browser_open_shim_path);
    cmd.env(
        "CTX_CLAUDE_AUTH_URL_CAPTURE_PATH",
        &browser_open_capture_path,
    );

    let mut child = pair.slave.spawn_command(cmd).with_context(|| {
        format!(
            "spawning claude setup-token via {}",
            runtime.command_abs_path
        )
    })?;
    let killer = Arc::new(StdMutex::new(child.clone_killer()));
    drop(pair.slave);

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
        browser_open_capture_path,
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
        mut exit_rx,
        killer,
        browser_open_capture_path,
        browser_open_shim_dir,
    } = spawn_claude_setup_token_command(&runtime)?;

    let mut buffered_lines = Vec::new();
    let mut auth_url = None;
    let mut hit_unsupported_manual_fallback = false;
    let mut transcript = String::new();
    let hard_deadline = Instant::now() + CLAUDE_LOGIN_URL_WAIT;
    let mut settle_deadline: Option<Instant> = None;
    loop {
        upgrade_claude_auth_url_from_capture_path(&mut auth_url, &browser_open_capture_path);
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
                hit_unsupported_manual_fallback |=
                    claude_login_hit_unsupported_manual_fallback(&line);
                buffered_lines.push(line);
                upgrade_claude_auth_url_from_capture_path(
                    &mut auth_url,
                    &browser_open_capture_path,
                );
                if let Some((candidate, source)) = extract_preferred_claude_auth_url(&transcript) {
                    if should_replace_observed_claude_auth_url(
                        auth_url.as_deref(),
                        &candidate,
                        source,
                    ) {
                        auth_url = Some(candidate);
                    }
                    settle_deadline = Some(Instant::now() + CLAUDE_LOGIN_URL_SETTLE_WAIT);
                }
            }
            Ok(None) => break,
            Err(_) => break,
        }
    }
    upgrade_claude_auth_url_from_capture_path(&mut auth_url, &browser_open_capture_path);

    if hit_unsupported_manual_fallback {
        let _ = kill_claude_login_process(Arc::clone(&killer)).await;
        let _ = tokio::time::timeout(CLAUDE_LOGIN_EXIT_GRACE_WAIT, &mut exit_rx).await;
        bail!(CLAUDE_UNSUPPORTED_MANUAL_FALLBACK_ERROR);
    }

    Ok(ClaudeLoginProcess {
        line_rx: rx,
        buffered_lines,
        auth_url,
        browser_open_capture_path,
        exit_rx,
        killer,
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
        if let Some((candidate, source)) = extract_preferred_claude_auth_url(&line) {
            if should_replace_observed_claude_auth_url(
                observed_auth_url.as_deref(),
                &candidate,
                source,
            ) {
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
        if let Some((candidate, source)) = extract_preferred_claude_auth_url(transcript) {
            if should_replace_observed_claude_auth_url(
                observed_auth_url.as_deref(),
                &candidate,
                source,
            ) {
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
    let mut exit_result: Option<anyhow::Result<portable_pty::ExitStatus>> = None;
    let mut terminal_error: Option<String> = None;

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
        upgrade_claude_auth_url_from_capture_path(
            &mut observed_auth_url,
            &login.browser_open_capture_path,
        );
        if claude_login_hit_unsupported_manual_fallback(&transcript) {
            terminal_error = Some(CLAUDE_UNSUPPORTED_MANUAL_FALLBACK_ERROR.to_string());
            break;
        }
        if !had_auth_url && observed_auth_url.is_some() {
            completion_deadline = Some(Instant::now() + CLAUDE_LOGIN_COMPLETION_TIMEOUT);
        }
    }

    while terminal_error.is_none() {
        upgrade_claude_auth_url_from_capture_path(
            &mut observed_auth_url,
            &login.browser_open_capture_path,
        );
        let deadline = completion_deadline.unwrap_or(auth_url_deadline);
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            terminal_error = Some(if observed_auth_url.is_some() {
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
                        upgrade_claude_auth_url_from_capture_path(
                            &mut observed_auth_url,
                            &login.browser_open_capture_path,
                        );
                        if claude_login_hit_unsupported_manual_fallback(&transcript) {
                            terminal_error = Some(CLAUDE_UNSUPPORTED_MANUAL_FALLBACK_ERROR.to_string());
                            break;
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
            _ = &mut timeout_future => {
                terminal_error = Some(if observed_auth_url.is_some() {
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
            upgrade_claude_auth_url_from_capture_path(
                &mut observed_auth_url,
                &login.browser_open_capture_path,
            );
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
            upgrade_claude_auth_url_from_capture_path(
                &mut observed_auth_url,
                &login.browser_open_capture_path,
            );
        }
    }

    if terminal_error.is_some() {
        if let Err(err) = kill_claude_login_process(Arc::clone(&login.killer)).await {
            let suffix = format!("; failed to terminate setup-token process cleanly: {err}");
            terminal_error = Some(match terminal_error.take() {
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
    let mut final_error: Option<String> = terminal_error;
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn preferred_claude_auth_url_uses_browser_open_marker_over_scraped_url() {
        let expected = "https://claude.ai/oauth/authorize?code=true&client_id=cid&response_type=code&redirect_uri=http%3A%2F%2Flocalhost%3A58215%2Fcallback&scope=user%3Ainference&code_challenge=abc&code_challenge_method=S256&state=good-state";
        let corrupted = "https://claude.ai/oauth/authorize?code=true&client_id=cid&response_type=code&redirect_uri=https:/platform.claude.com/oauth/code/callback&scope=user:inference&code_challenge=abc&code_challenge_method=S256&state=bad-statePastecodehereifprompted%3E";
        let transcript = format!(
            "{CLAUDE_BROWSER_OPEN_MARKER}{expected}\nBrowser didn't open? Use the URL below to sign in\n{corrupted}\nPaste code here if prompted >"
        );

        assert_eq!(
            extract_preferred_claude_auth_url(&transcript)
                .map(|(value, _)| value)
                .as_deref(),
            Some(expected)
        );
    }

    #[test]
    fn browser_open_marker_replaces_longer_incomplete_transcript_url() {
        let current = "https://claude.ai/oauth/authorize?code=true&client_id=cid&response_type=code&redirect_uri=https:/platform.claude.com/oauth/code/callback&scope=user:inference&code_challenge=abc&code_challenge_method=S256&state=bad-state";
        let candidate = "https://claude.ai/oauth/authorize?code=true&client_id=cid&response_type=code&redirect_uri=http%3A%2F%2Flocalhost%3A58215%2Fcallback&scope=user%3Ainference&code_challenge=abc&code_challenge_method=S256&state=good-state";

        assert!(should_replace_observed_claude_auth_url(
            Some(current),
            candidate,
            ClaudeAuthUrlSource::BrowserOpenMarker,
        ));
    }

    #[test]
    fn provider_browser_auth_tier_skips_os_browser_launch() {
        assert!(claude_login_should_skip_browser_open(Some(
            CLAUDE_BROWSER_AUTH_TIER
        )));
        assert!(claude_login_should_skip_browser_open(Some(
            " Provider-Browser-Auth "
        )));
        assert!(!claude_login_should_skip_browser_open(Some(
            "provider-api-auth"
        )));
        assert!(!claude_login_should_skip_browser_open(None));
    }

    #[test]
    fn reads_captured_browser_open_url_from_side_channel_file() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let capture_path = temp_dir.path().join("auth-url");
        let expected = "https://claude.ai/oauth/authorize?code=true&client_id=cid&response_type=code&redirect_uri=http%3A%2F%2Flocalhost%3A58215%2Fcallback&scope=user%3Ainference&code_challenge=abc&code_challenge_method=S256&state=good-state";
        std::fs::write(&capture_path, format!("{expected}\n")).expect("write capture file");

        assert_eq!(
            read_claude_browser_open_capture_url(&capture_path).as_deref(),
            Some(expected)
        );
    }

    fn write_executable_script(path: &std::path::Path, body: &str) {
        std::fs::write(path, body).expect("write script");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
                .expect("chmod script");
        }
    }

    #[test]
    fn browser_open_shim_capture_only_writes_auth_url() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let script_path = temp_dir.path().join("open-browser");
        write_executable_script(&script_path, claude_browser_open_shim_script(true));
        let capture_path = temp_dir.path().join("auth-url");
        let auth_url = "https://claude.ai/oauth/authorize?redirect_uri=http%3A%2F%2Flocalhost%3A5999%2Fcallback&state=test";

        let status = Command::new("/bin/sh")
            .arg(&script_path)
            .arg(auth_url)
            .env("CTX_CLAUDE_AUTH_URL_CAPTURE_PATH", &capture_path)
            .status()
            .expect("run capture-only shim");

        assert!(status.success());
        assert_eq!(
            std::fs::read_to_string(&capture_path).expect("read capture path"),
            format!("{auth_url}\n")
        );
    }

    #[test]
    fn browser_open_shim_invokes_open_and_captures_auth_url() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let script_path = temp_dir.path().join("open-browser");
        write_executable_script(&script_path, claude_browser_open_shim_script(false));
        let capture_path = temp_dir.path().join("auth-url");
        let open_log_path = temp_dir.path().join("open.log");
        let open_path = temp_dir.path().join("open");
        write_executable_script(
            &open_path,
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$1\" > \"{}\"\nexit 0\n",
                open_log_path.display()
            ),
        );
        let auth_url = "https://claude.ai/oauth/authorize?redirect_uri=http%3A%2F%2Flocalhost%3A6001%2Fcallback&state=test";
        let path_env = format!("{}:/usr/bin:/bin", temp_dir.path().display());

        let status = Command::new("/bin/sh")
            .arg(&script_path)
            .arg(auth_url)
            .env("CTX_CLAUDE_AUTH_URL_CAPTURE_PATH", &capture_path)
            .env("PATH", path_env)
            .status()
            .expect("run browser-open shim");

        assert!(status.success());
        assert_eq!(
            std::fs::read_to_string(&capture_path).expect("read capture path"),
            format!("{auth_url}\n")
        );
        assert_eq!(
            std::fs::read_to_string(&open_log_path).expect("read open log"),
            format!("{auth_url}\n")
        );
    }
}
