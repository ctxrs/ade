use super::*;

use crate::desktop_local_daemon::ensure_local_connection;
#[cfg(test)]
use std::cell::Cell;

const SSH_CONFIG_OVERRIDE_ENV: &str = "CTX_DESKTOP_SSH_CONFIG_PATH";
const DESKTOP_DAEMON_BIN_NAME: &str = "ctx-daemon";
const DAEMON_PATH_SENTINEL_BEGIN: &str = "__CTX_DAEMON_PATH_BEGIN__";
const DAEMON_PATH_SENTINEL_END: &str = "__CTX_DAEMON_PATH_END__";

fn normalized_ssh_config_override(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}

fn ssh_config_override_path() -> Option<String> {
    std::env::var(SSH_CONFIG_OVERRIDE_ENV)
        .ok()
        .as_deref()
        .and_then(normalized_ssh_config_override)
}

fn new_ssh_command() -> Command {
    let mut cmd = Command::new("ssh");
    if let Some(path) = ssh_config_override_path() {
        cmd.arg("-F").arg(path);
    }
    cmd
}

fn append_unique_path_entry(
    entries: &mut Vec<PathBuf>,
    seen: &mut std::collections::HashSet<PathBuf>,
    entry: PathBuf,
) {
    if entry.as_os_str().is_empty() {
        return;
    }
    if seen.insert(entry.clone()) {
        entries.push(entry);
    }
}

fn append_path_entries_from_raw(
    entries: &mut Vec<PathBuf>,
    seen: &mut std::collections::HashSet<PathBuf>,
    raw: &std::ffi::OsStr,
) {
    for entry in std::env::split_paths(raw) {
        append_unique_path_entry(entries, seen, entry);
    }
}

fn append_common_tool_dirs(
    entries: &mut Vec<PathBuf>,
    seen: &mut std::collections::HashSet<PathBuf>,
    home_dir: Option<&Path>,
) {
    if let Some(home) = home_dir {
        append_unique_path_entry(entries, seen, home.join(".local").join("bin"));
        append_unique_path_entry(entries, seen, home.join("bin"));
    }
    for raw in [
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/usr/bin",
        "/bin",
        "/usr/sbin",
        "/sbin",
    ] {
        append_unique_path_entry(entries, seen, PathBuf::from(raw));
    }
}

fn build_effective_daemon_path(
    current_path: Option<&std::ffi::OsStr>,
    shell_path: Option<&std::ffi::OsStr>,
    home_dir: Option<&Path>,
) -> Option<std::ffi::OsString> {
    let mut entries = Vec::new();
    let mut seen = std::collections::HashSet::new();
    if let Some(raw) = current_path {
        append_path_entries_from_raw(&mut entries, &mut seen, raw);
    }
    if let Some(raw) = shell_path {
        append_path_entries_from_raw(&mut entries, &mut seen, raw);
    }
    append_common_tool_dirs(&mut entries, &mut seen, home_dir);
    if entries.is_empty() {
        return None;
    }
    std::env::join_paths(entries).ok()
}

fn extract_shell_path(stdout: &[u8]) -> Option<std::ffi::OsString> {
    let output = String::from_utf8_lossy(stdout);
    let start = output.find(DAEMON_PATH_SENTINEL_BEGIN)?;
    let tail = &output[start + DAEMON_PATH_SENTINEL_BEGIN.len()..];
    let end = tail.find(DAEMON_PATH_SENTINEL_END)?;
    let path = tail[..end].trim();
    (!path.is_empty()).then_some(std::ffi::OsString::from(path))
}

fn read_login_shell_path(shell_path: &Path) -> Option<std::ffi::OsString> {
    if !shell_path.is_absolute() || !shell_path.exists() {
        return None;
    }
    let output = Command::new(shell_path)
        .arg("-lc")
        .arg(format!(
            "printf '{DAEMON_PATH_SENTINEL_BEGIN}%s{DAEMON_PATH_SENTINEL_END}' \"$PATH\""
        ))
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    extract_shell_path(&output.stdout)
}

fn candidate_login_shell_paths() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(shell) = std::env::var_os("SHELL").filter(|value| !value.is_empty()) {
        let path = PathBuf::from(shell);
        if path.is_absolute() {
            candidates.push(path);
        }
    }
    for raw in ["/bin/zsh", "/bin/bash", "/bin/sh"] {
        let path = PathBuf::from(raw);
        if !candidates.contains(&path) {
            candidates.push(path);
        }
    }
    candidates
}

fn resolve_daemon_path_env() -> Option<std::ffi::OsString> {
    let shell_path = candidate_login_shell_paths()
        .into_iter()
        .find_map(|shell| read_login_shell_path(&shell));
    let home_dir = std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    build_effective_daemon_path(
        std::env::var_os("PATH").as_deref(),
        shell_path.as_deref(),
        home_dir.as_deref(),
    )
}

#[derive(Debug, Deserialize)]
pub(super) struct DaemonAuthFile {
    pub(super) token: String,
    #[serde(default)]
    pub(super) daemon_url: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct DaemonHealthCompatibility {
    #[serde(default)]
    pub(super) desktop_exact_version: String,
    #[serde(default)]
    pub(super) desktop_dev_instance_id: String,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct DaemonHealthSummary {
    #[serde(default)]
    pub(super) pid: u32,
    #[serde(default)]
    pub(super) data_root: String,
    #[serde(default)]
    pub(super) compatibility: DaemonHealthCompatibility,
}

#[derive(Debug, Deserialize)]
pub(super) struct DesktopDaemonRequest {
    pub(super) method: String,
    pub(super) path: String,
    #[serde(default)]
    pub(super) body: Option<String>,
    #[serde(default)]
    pub(super) headers: Vec<(String, String)>,
}

#[derive(Debug, Deserialize)]
pub(super) struct DesktopCodexLoginRelayReq {
    login_id: String,
    callback_url: String,
    completion_token: String,
}

#[derive(Debug, Serialize)]
pub(super) struct DesktopHttpResponse {
    pub(super) status: u16,
    pub(super) body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) content_type: Option<String>,
}

#[tauri::command]
pub(super) async fn desktop_daemon_request(
    app: tauri::AppHandle,
    req: DesktopDaemonRequest,
) -> Result<DesktopHttpResponse, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ConnectionManager>();
        let manager: &ConnectionManager = state.inner();
        // Many UI paths (including initial app load to a workbench route) can issue daemon requests
        // before explicitly calling `desktop_connect_local`. Auto-connect here to avoid spurious
        // "daemon unavailable" overlays on cold start.
        ensure_local_connection(&app, manager).map_err(|err| format!("{err:#}"))?;
        manager
            .daemon_request(req)
            .map_err(|err| format!("{err:#}"))
    })
    .await
    .map_err(|e| format!("daemon request failed: {e}"))?
}

fn is_loopback_host_name(host: &str) -> bool {
    let value = host.trim().to_ascii_lowercase();
    if value == "localhost" {
        return true;
    }
    value
        .parse::<std::net::IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

fn read_http_request_target(stream: &mut TcpStream) -> Result<String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .context("setting relay read timeout")?;
    let mut buf = [0u8; 16384];
    let read = stream.read(&mut buf).context("reading callback request")?;
    if read == 0 {
        anyhow::bail!("empty callback request");
    }
    let request = String::from_utf8_lossy(&buf[..read]);
    let first_line = request
        .lines()
        .next()
        .ok_or_else(|| anyhow!("callback request missing request line"))?;
    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();
    if method != "GET" {
        anyhow::bail!("unsupported callback method: {method}");
    }
    if target.trim().is_empty() {
        anyhow::bail!("callback request missing target path");
    }
    Ok(target.trim().to_string())
}

fn write_http_response(stream: &mut TcpStream, status: &str, message: &str) {
    let html = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>ctx Codex Login</title></head><body><h2>{status}</h2><p>{message}</p><p>You can now return to ctx.</p></body></html>"
    );
    let payload = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        html.len(),
        html
    );
    let _ = stream.write_all(payload.as_bytes());
    let _ = stream.flush();
}

fn callback_url_from_target(
    target: &str,
    expected_path: &str,
    expected_port: u16,
) -> Result<String> {
    let mut url = if target.starts_with("http://") || target.starts_with("https://") {
        Url::parse(target).context("parsing callback request target URL")?
    } else {
        Url::parse(&format!("http://localhost{target}"))
            .context("parsing callback request target path")?
    };
    if url.path() != expected_path {
        anyhow::bail!("callback path mismatch");
    }
    if url.query().is_none() {
        anyhow::bail!("callback query is missing");
    }
    if !is_loopback_host_name(url.host_str().unwrap_or_default()) {
        anyhow::bail!("callback host must be loopback");
    }
    let _ = url.set_scheme("http");
    let _ = url.set_host(Some("127.0.0.1"));
    let _ = url.set_port(Some(expected_port));
    Ok(url.to_string())
}

fn process_codex_login_relay_connection(
    app: &tauri::AppHandle,
    mut stream: TcpStream,
    login_id: &str,
    completion_token: &str,
    expected_path: &str,
    expected_port: u16,
) -> Result<()> {
    let target = match read_http_request_target(&mut stream) {
        Ok(target) => target,
        Err(err) => {
            write_http_response(
                &mut stream,
                "400 Bad Request",
                "Invalid callback request. Retry from ctx Settings.",
            );
            return Err(err);
        }
    };
    let callback_url = match callback_url_from_target(&target, expected_path, expected_port) {
        Ok(url) => url,
        Err(err) => {
            write_http_response(
                &mut stream,
                "400 Bad Request",
                "Callback URL validation failed. Retry from ctx Settings.",
            );
            return Err(err);
        }
    };

    let state = app.state::<ConnectionManager>();
    let manager: &ConnectionManager = state.inner();
    ensure_local_connection(app, manager).context("ensuring daemon connection")?;
    let body = serde_json::json!({
        "callback_url": callback_url,
        "completion_token": completion_token,
    })
    .to_string();
    let response = manager.daemon_request(DesktopDaemonRequest {
        method: "POST".to_string(),
        path: format!("/api/providers/codex/accounts/login/{login_id}"),
        body: Some(body),
        headers: vec![("Content-Type".to_string(), "application/json".to_string())],
    })?;
    if (200..300).contains(&response.status) {
        write_http_response(
            &mut stream,
            "200 OK",
            "Codex login callback received. Completing sign-in.",
        );
        return Ok(());
    }

    write_http_response(
        &mut stream,
        "502 Bad Gateway",
        "ctx could not complete login relay on the daemon. Use manual callback paste in Settings.",
    );
    anyhow::bail!(
        "daemon callback completion failed with status {}",
        response.status
    )
}

#[tauri::command]
pub(super) async fn desktop_start_codex_login_relay(
    app: tauri::AppHandle,
    req: DesktopCodexLoginRelayReq,
) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let login_id = req.login_id.trim().to_string();
        if login_id.is_empty() {
            return Err(anyhow!("login_id is required"));
        }
        let completion_token = req.completion_token.trim().to_string();
        if completion_token.is_empty() {
            return Err(anyhow!("completion_token is required"));
        }
        let callback_url = Url::parse(req.callback_url.trim())
            .context("invalid callback_url for relay listener")?;
        if callback_url.scheme() != "http" {
            anyhow::bail!("callback_url must use http");
        }
        let host = callback_url
            .host_str()
            .ok_or_else(|| anyhow!("callback_url missing host"))?
            .to_string();
        if !is_loopback_host_name(&host) {
            anyhow::bail!("callback_url host must be loopback");
        }
        let port = callback_url
            .port()
            .ok_or_else(|| anyhow!("callback_url missing explicit port"))?;
        let expected_path = callback_url.path().to_string();
        if !expected_path.starts_with("/auth/callback") {
            anyhow::bail!("callback_url path must start with /auth/callback");
        }

        let listener = TcpListener::bind((host.as_str(), port))
            .with_context(|| format!("binding codex callback relay on {host}:{port}"))?;
        listener
            .set_nonblocking(true)
            .context("setting callback relay nonblocking")?;

        std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5 * 60);
            loop {
                if Instant::now() >= deadline {
                    break;
                }
                match listener.accept() {
                    Ok((stream, _addr)) => {
                        if let Err(err) = process_codex_login_relay_connection(
                            &app,
                            stream,
                            &login_id,
                            &completion_token,
                            &expected_path,
                            port,
                        ) {
                            eprintln!("codex login relay failed: {err:#}");
                        }
                        break;
                    }
                    Err(err) if err.kind() == ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(40));
                    }
                    Err(err) => {
                        eprintln!("codex login relay accept failed: {err}");
                        break;
                    }
                }
            }
        });
        Ok(true)
    })
    .await
    .map_err(|e| format!("starting codex relay failed: {e}"))?
    .map_err(to_err)
}

#[tauri::command]
pub(super) async fn desktop_upload_blob(
    app: tauri::AppHandle,
    bytes: Vec<u8>,
    mime_type: String,
    name: Option<String>,
) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ConnectionManager>();
        state.upload_blob(bytes, mime_type, name).map_err(to_err)
    })
    .await
    .map_err(|e| format!("blob upload failed: {e}"))?
}

fn parse_daemon_auth(bytes: &[u8], path: &Path) -> Result<DaemonAuthFile> {
    let auth: DaemonAuthFile =
        serde_json::from_slice(bytes).with_context(|| format!("parsing {}", path.display()))?;
    if auth.token.trim().is_empty() {
        anyhow::bail!("daemon auth file {} contains empty token", path.display());
    }
    Ok(auth)
}

pub(super) fn read_daemon_auth_with_retry(data_dir: &Path) -> Result<DaemonAuthFile> {
    let path = data_dir.join(DAEMON_AUTH_FILENAME);
    let deadline = Instant::now() + DAEMON_AUTH_READ_TIMEOUT;
    loop {
        let err = match std::fs::read(&path) {
            Ok(bytes) => return parse_daemon_auth(&bytes, &path),
            Err(err) if err.kind() == ErrorKind::NotFound => {
                anyhow!("daemon auth file not found at {}", path.display())
            }
            Err(err) => anyhow::Error::new(err)
                .context(format!("reading daemon auth file {}", path.display())),
        };
        if Instant::now() > deadline {
            return Err(err);
        }
        std::thread::sleep(DAEMON_AUTH_RETRY_DELAY);
    }
}

fn read_daemon_auth_if_present(data_dir: &Path) -> Result<Option<DaemonAuthFile>> {
    let path = data_dir.join(DAEMON_AUTH_FILENAME);
    match std::fs::read(&path) {
        Ok(bytes) => Ok(Some(parse_daemon_auth(&bytes, &path)?)),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => {
            Err(anyhow::Error::new(err)
                .context(format!("reading daemon auth file {}", path.display())))
        }
    }
}

pub(super) fn resolve_env_local_daemon(app: &tauri::AppHandle) -> Result<Option<(String, String)>> {
    let url = match std::env::var("CTX_DESKTOP_DAEMON_URL") {
        Ok(v) => v.trim().to_string(),
        Err(_) => return Ok(None),
    };
    if url.is_empty() {
        return Ok(None);
    }
    let token = match std::env::var("CTX_DESKTOP_DAEMON_TOKEN") {
        Ok(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => {
            let data_dir = daemon_data_dir(app)?;
            match read_daemon_auth_if_present(&data_dir)? {
                Some(auth) if auth.daemon_url.as_deref() == Some(url.as_str()) => auth.token,
                _ => {
                    anyhow::bail!(
                        "CTX_DESKTOP_DAEMON_URL is set but no matching token found (set CTX_DESKTOP_DAEMON_TOKEN)"
                    );
                }
            }
        }
    };
    Ok(Some((url, token)))
}

pub(super) fn resolve_existing_local_daemon(
    app: &tauri::AppHandle,
    data_dir: &Path,
) -> Result<Option<(String, String, Option<u32>)>> {
    let Some(auth) = read_daemon_auth_if_present(data_dir)? else {
        return Ok(None);
    };
    let Some(url) = auth.daemon_url.as_deref() else {
        return Ok(None);
    };
    let desktop_version = app.package_info().version.to_string();
    let desktop_dev_instance_id = desktop_dev_instance_id();
    let Ok(health) = daemon_health(url) else {
        return Ok(None);
    };
    if local_daemon_health_matches_expected(
        &health,
        data_dir,
        &desktop_version,
        desktop_dev_instance_id,
    ) {
        return Ok(Some((
            url.to_string(),
            auth.token,
            normalize_daemon_pid(health.pid),
        )));
    }
    if should_reclaim_incompatible_local_daemon(url, &health, data_dir) {
        if let Err(err) = reclaim_incompatible_local_daemon(url, &health)
            .with_context(|| format!("reclaiming incompatible local daemon at {url}"))
        {
            eprintln!("{err:#}");
        }
    }
    Ok(None)
}

pub(super) fn normalize_daemon_pid(pid: u32) -> Option<u32> {
    if pid == 0 {
        None
    } else {
        Some(pid)
    }
}

fn should_reclaim_incompatible_local_daemon(
    base_url: &str,
    health: &DaemonHealthSummary,
    expected_data_dir: &Path,
) -> bool {
    if health.pid == 0 {
        return false;
    }
    let Ok(parsed) = Url::parse(base_url) else {
        return false;
    };
    let Some(host) = parsed.host_str() else {
        return false;
    };
    if !is_loopback_host_name(host) {
        return false;
    }
    let daemon_data_root = health.data_root.trim();
    if daemon_data_root.is_empty() {
        return false;
    }
    let daemon_root = normalize_path_for_compare(Path::new(daemon_data_root));
    let expected_root = normalize_path_for_compare(expected_data_dir);
    daemon_root == expected_root
}

fn reclaim_incompatible_local_daemon(base_url: &str, health: &DaemonHealthSummary) -> Result<()> {
    if health.pid == 0 {
        anyhow::bail!("incompatible local daemon missing pid");
    }
    let pid = health.pid;
    let graceful_revalidated = daemon_reports_expected_pid(base_url, pid);
    let graceful_err = if graceful_revalidated {
        terminate_pid(pid, false).err()
    } else {
        None
    };
    if wait_for_daemon_reclaim(base_url, pid, Duration::from_secs(3)).is_ok() {
        return Ok(());
    }
    let force_revalidated = daemon_reports_expected_pid(base_url, pid);
    let force_err = if force_revalidated {
        terminate_pid(pid, true).err()
    } else {
        None
    };
    if wait_for_daemon_reclaim(base_url, pid, Duration::from_secs(2)).is_ok() {
        return Ok(());
    }
    let mut details = Vec::new();
    if !graceful_revalidated {
        details.push(
            "skipped graceful terminate (daemon pid could not be revalidated via /api/health)"
                .to_string(),
        );
    }
    if let Some(err) = graceful_err {
        details.push(format!("graceful terminate failed: {err:#}"));
    }
    if !force_revalidated {
        details.push(
            "skipped force terminate (daemon pid could not be revalidated via /api/health)"
                .to_string(),
        );
    }
    if let Some(err) = force_err {
        details.push(format!("force terminate failed: {err:#}"));
    }
    if details.is_empty() {
        anyhow::bail!("incompatible local daemon pid {} did not exit", pid);
    }
    anyhow::bail!(
        "incompatible local daemon pid {} did not exit ({})",
        pid,
        details.join("; ")
    );
}

fn wait_until_daemon_reclaimed(base_url: &str, pid: u32, timeout: Duration) -> bool {
    const RECLAIM_HEALTH_PROBE_MAX_TIMEOUT: Duration = Duration::from_millis(250);
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return false;
        }
        let health_timeout =
            reclaim_health_probe_timeout(remaining, RECLAIM_HEALTH_PROBE_MAX_TIMEOUT);
        let health = daemon_health_with_timeout(base_url, health_timeout).ok();
        let pid_alive = is_pid_alive(pid).unwrap_or(true);
        if reclaim_complete(pid, pid_alive, health.as_ref()) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(120));
    }
}

pub(super) fn wait_for_daemon_reclaim(base_url: &str, pid: u32, timeout: Duration) -> Result<()> {
    if pid == 0 {
        anyhow::bail!("invalid pid 0");
    }
    if wait_until_daemon_reclaimed(base_url, pid, timeout) {
        return Ok(());
    }
    anyhow::bail!("local daemon pid {pid} did not exit within {:?}", timeout);
}

fn reclaim_health_probe_timeout(remaining: Duration, max_probe_timeout: Duration) -> Duration {
    if remaining.is_zero() {
        return Duration::from_millis(1);
    }
    std::cmp::min(remaining, max_probe_timeout)
}

fn reclaim_complete(pid: u32, pid_alive: bool, health: Option<&DaemonHealthSummary>) -> bool {
    let same_pid_serving_health = health.map(|h| h.pid == pid).unwrap_or(false);
    !pid_alive && !same_pid_serving_health
}

fn daemon_reports_expected_pid(base_url: &str, pid: u32) -> bool {
    let health = daemon_health(base_url).ok();
    health_reports_expected_pid(pid, health.as_ref())
}

fn health_reports_expected_pid(pid: u32, health: Option<&DaemonHealthSummary>) -> bool {
    health.map(|h| h.pid == pid).unwrap_or(false)
}

fn is_pid_alive(pid: u32) -> Result<bool> {
    if pid == 0 {
        return Ok(false);
    }

    #[cfg(unix)]
    {
        let output = Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .output()
            .with_context(|| format!("running kill -0 {pid}"))?;
        if output.status.success() {
            return Ok(true);
        }
        if command_reports_missing_process(&output) {
            return Ok(false);
        }
        if command_reports_permission_denied(&output) {
            return Ok(true);
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        anyhow::bail!("kill -0 {pid} failed: {stderr}");
    }

    #[cfg(windows)]
    {
        let output = Command::new("tasklist")
            .arg("/FI")
            .arg(format!("PID eq {pid}"))
            .arg("/FO")
            .arg("CSV")
            .arg("/NH")
            .output()
            .with_context(|| format!("running tasklist for pid {pid}"))?;
        if !output.status.success() {
            if command_reports_missing_process(&output) {
                return Ok(false);
            }
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            anyhow::bail!("tasklist pid {pid} failed: {stderr}");
        }
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let pid_token = format!(",\"{pid}\",");
        if stdout.contains(&pid_token) {
            return Ok(true);
        }
        if command_reports_missing_process(&output) {
            return Ok(false);
        }
        Ok(false)
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        anyhow::bail!("pid liveness checks are unsupported on this platform");
    }
}

pub(super) fn terminate_pid(pid: u32, force: bool) -> Result<()> {
    if pid == 0 {
        anyhow::bail!("invalid pid 0");
    }

    #[cfg(unix)]
    {
        let signal = if force { "-KILL" } else { "-TERM" };
        let output = Command::new("kill")
            .arg(signal)
            .arg(pid.to_string())
            .output()
            .with_context(|| format!("running kill {signal} {pid}"))?;
        if output.status.success() || command_reports_missing_process(&output) {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        anyhow::bail!("kill {signal} {pid} failed: {stderr}");
    }

    #[cfg(windows)]
    {
        let mut cmd = Command::new("taskkill");
        cmd.arg("/PID").arg(pid.to_string()).arg("/T");
        if force {
            cmd.arg("/F");
        }
        let output = cmd
            .output()
            .with_context(|| format!("running taskkill for pid {pid}"))?;
        if output.status.success() || command_reports_missing_process(&output) {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        anyhow::bail!("taskkill pid {pid} failed: {stderr}");
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = (pid, force);
        anyhow::bail!("process termination is unsupported on this platform");
    }
}

fn command_reports_missing_process(output: &std::process::Output) -> bool {
    let stdout = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
    let stderr = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
    stdout.contains("no such process")
        || stderr.contains("no such process")
        || stdout.contains("not found")
        || stderr.contains("not found")
        || stdout.contains("not running")
        || stderr.contains("not running")
        || stdout.contains("no running instance")
        || stderr.contains("no running instance")
}

fn command_reports_permission_denied(output: &std::process::Output) -> bool {
    let stdout = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
    let stderr = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
    stdout.contains("operation not permitted")
        || stderr.contains("operation not permitted")
        || stdout.contains("permission denied")
        || stderr.contains("permission denied")
}

fn read_remote_daemon_auth(
    host: &str,
    user: Option<&str>,
    remote_data_dir: Option<&str>,
) -> Result<DaemonAuthFile> {
    let target = match user {
        Some(u) if !u.trim().is_empty() => format!("{}@{}", u.trim(), host),
        _ => host.to_string(),
    };
    let data_dir = remote_data_dir
        .filter(|d| !d.trim().is_empty())
        .unwrap_or("~/.ctx");
    let auth_path = format!(
        "{}/{}",
        data_dir.trim_end_matches('/'),
        DAEMON_AUTH_FILENAME
    );
    let cmd = format!("cat -- {}", remote_path_expr(&auth_path));
    let remote_cmd = format!("sh -lc {}", shell_escape(&cmd));

    let output = new_ssh_command()
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg("ConnectTimeout=15")
        .arg("-o")
        .arg("ServerAliveInterval=15")
        .arg("-o")
        .arg("ServerAliveCountMax=2")
        .arg(target)
        // NOTE: sshd does not preserve argv boundaries for the remote command; pass as one string.
        .arg(remote_cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .context("reading daemon auth file over ssh")?;

    if !output.status.success() {
        return Err(anyhow!(
            "ssh read failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    parse_daemon_auth(&output.stdout, Path::new(&auth_path))
}

pub(super) fn read_remote_daemon_auth_with_retry(
    host: &str,
    user: Option<&str>,
    remote_data_dir: Option<&str>,
) -> Result<DaemonAuthFile> {
    let deadline = Instant::now() + DAEMON_AUTH_REMOTE_TIMEOUT;
    loop {
        let err = match read_remote_daemon_auth(host, user, remote_data_dir) {
            Ok(auth) => return Ok(auth),
            Err(err) => err,
        };
        if Instant::now() > deadline {
            return Err(err);
        }
        std::thread::sleep(DAEMON_AUTH_RETRY_DELAY);
    }
}

#[derive(Debug, Clone, Deserialize)]
struct DesktopBundledAssetsManifest {
    #[allow(dead_code)]
    pub version: u32,
    #[serde(default)]
    pub providers: Vec<DesktopBundledProvider>,
    #[serde(default)]
    pub runtimes: Vec<DesktopBundledRuntime>,
    #[serde(default)]
    pub images: Vec<DesktopBundledImage>,
}

#[derive(Debug, Clone, Deserialize)]
struct DesktopBundledProvider {
    pub id: String,
    pub os: String,
    pub arch: String,
    pub command: String,
}

#[derive(Debug, Clone, Deserialize)]
struct DesktopBundledRuntime {
    pub id: String,
    pub os: String,
    pub arch: String,
    pub root: String,
    pub bin: String,
}

#[derive(Debug, Clone, Deserialize)]
struct DesktopBundledImage {
    pub id: String,
    pub os: String,
    pub arch: String,
    pub tar: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RuntimeLockRequiredTargets {
    #[serde(default)]
    provider: Vec<String>,
    #[serde(default)]
    runtime: Vec<String>,
    #[serde(default)]
    image: Vec<String>,
    #[serde(default)]
    machine_cache: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RuntimeLockRequired {
    #[serde(default)]
    provider_ids: Vec<String>,
    #[serde(default)]
    runtime_ids: Vec<String>,
    #[serde(default)]
    image_ids: Vec<String>,
    #[serde(default)]
    machine_cache_ids: Vec<String>,
    #[serde(default)]
    targets: RuntimeLockRequiredTargets,
}

#[derive(Debug, Clone, Deserialize)]
struct RuntimeLockV2 {
    version: u32,
    #[serde(default)]
    profiles: std::collections::HashMap<String, RuntimeLockProfile>,
    required: RuntimeLockRequired,
    #[serde(default)]
    components: Vec<RuntimeLockComponent>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RuntimeLockProfile {
    #[serde(default)]
    allowed_source_types: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RuntimeLockComponentSource {
    #[serde(default)]
    source_type: String,
    #[serde(default)]
    uri: Option<String>,
    #[serde(default)]
    sha256: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RuntimeLockComponent {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    id: String,
    #[serde(default)]
    os: String,
    #[serde(default)]
    arch: String,
    #[serde(default)]
    variant: Option<String>,
    #[serde(default)]
    sources: Vec<RuntimeLockComponentSource>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RuntimeTarget {
    os: String,
    arch: String,
}

fn parity_profile_enabled() -> bool {
    matches!(
        std::env::var("CTX_RUNTIME_PROFILE")
            .ok()
            .map(|v| v.trim().to_ascii_lowercase())
            .as_deref(),
        None | Some("") | Some("parity")
    )
}

fn active_runtime_profile() -> &'static str {
    match std::env::var("CTX_RUNTIME_PROFILE")
        .ok()
        .map(|v| v.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("override") => "override",
        Some("source-all") => "source-all",
        _ => "parity",
    }
}

fn allowed_source_types_for_profile(lock: &RuntimeLockV2) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    let profile = active_runtime_profile();
    let cfg = lock
        .profiles
        .get(profile)
        .or_else(|| lock.profiles.get("parity"));
    if let Some(cfg) = cfg {
        for source_type in &cfg.allowed_source_types {
            let trimmed = source_type.trim();
            if trimmed.is_empty() {
                continue;
            }
            out.insert(trimmed.to_string());
        }
    }
    out
}

fn lock_component_has_managed_source(
    component: &RuntimeLockComponent,
    allowed_sources: &std::collections::HashSet<String>,
) -> bool {
    component.sources.iter().any(|source| {
        let source_type = source.source_type.trim();
        if source_type.is_empty() || source_type == "local" {
            return false;
        }
        if !allowed_sources.is_empty() && !allowed_sources.contains(source_type) {
            return false;
        }
        source
            .uri
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_some()
            && source
                .sha256
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_some()
    })
}

fn required_component_has_managed_source(
    lock: &RuntimeLockV2,
    kind: &str,
    id: &str,
    target: &RuntimeTarget,
    allowed_sources: &std::collections::HashSet<String>,
) -> bool {
    lock.components.iter().any(|component| {
        let variant = component
            .variant
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("default");
        component.kind == kind
            && component.id == id
            && component.os == target.os
            && component.arch == target.arch
            && variant == "default"
            && lock_component_has_managed_source(component, allowed_sources)
    })
}

fn normalize_target_token(raw: &str, host_value: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.eq_ignore_ascii_case("host") {
        return Some(host_value.to_string());
    }
    Some(trimmed.to_string())
}

fn parse_target(raw: &str, host_os: &str, host_arch: &str) -> Option<RuntimeTarget> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (os, arch) = trimmed.split_once('/')?;
    let os = normalize_target_token(os, host_os)?;
    let arch = normalize_target_token(arch, host_arch)?;
    Some(RuntimeTarget { os, arch })
}

fn required_targets_or_default(
    configured: &[String],
    fallback: &[RuntimeTarget],
    host_os: &str,
    host_arch: &str,
) -> Vec<RuntimeTarget> {
    if configured.is_empty() {
        return fallback.to_vec();
    }
    let mut out = Vec::<RuntimeTarget>::new();
    for value in configured {
        if let Some(target) = parse_target(value, host_os, host_arch) {
            if !out.contains(&target) {
                out.push(target);
            }
        }
    }
    if out.is_empty() {
        return fallback.to_vec();
    }
    out
}

fn host_default_provider_targets() -> Vec<RuntimeTarget> {
    let host_os = std::env::consts::OS.to_string();
    let host_arch = std::env::consts::ARCH.to_string();
    if host_os == "macos" && host_arch == "aarch64" {
        return vec![
            RuntimeTarget {
                os: "macos".to_string(),
                arch: "aarch64".to_string(),
            },
            RuntimeTarget {
                os: "linux".to_string(),
                arch: "aarch64".to_string(),
            },
            RuntimeTarget {
                os: "linux".to_string(),
                arch: "x86_64".to_string(),
            },
        ];
    }
    let mut out = vec![RuntimeTarget {
        os: host_os.clone(),
        arch: host_arch.clone(),
    }];
    let linux_target = RuntimeTarget {
        os: "linux".to_string(),
        arch: host_arch,
    };
    if !out.contains(&linux_target) {
        out.push(linux_target);
    }
    out
}

fn host_default_runtime_targets() -> Vec<RuntimeTarget> {
    host_default_provider_targets()
}

fn host_default_image_targets() -> Vec<RuntimeTarget> {
    let host_arch = std::env::consts::ARCH.to_string();
    if std::env::consts::OS == "macos" && host_arch == "aarch64" {
        return vec![
            RuntimeTarget {
                os: "linux".to_string(),
                arch: "aarch64".to_string(),
            },
            RuntimeTarget {
                os: "linux".to_string(),
                arch: "x86_64".to_string(),
            },
        ];
    }
    vec![RuntimeTarget {
        os: "linux".to_string(),
        arch: host_arch,
    }]
}

fn host_default_machine_cache_targets() -> Vec<RuntimeTarget> {
    if std::env::consts::OS != "macos" {
        return Vec::new();
    }
    vec![RuntimeTarget {
        os: "macos".to_string(),
        arch: std::env::consts::ARCH.to_string(),
    }]
}

fn host_relevant_targets(
    all_targets: &[RuntimeTarget],
    fallback: &[RuntimeTarget],
) -> Vec<RuntimeTarget> {
    let allowed = fallback;
    let mut out = Vec::<RuntimeTarget>::new();
    for target in all_targets {
        if allowed.contains(target) && !out.contains(target) {
            out.push(target.clone());
        }
    }
    if !out.is_empty() {
        return out;
    }
    fallback.to_vec()
}

fn bundle_manifest_path(bundle_dir: &Path) -> PathBuf {
    if let Ok(raw) = std::env::var("CTX_BUNDLE_MANIFEST") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            let candidate = PathBuf::from(trimmed);
            if candidate.is_absolute() {
                return candidate;
            }
            return bundle_dir.join(candidate);
        }
    }
    bundle_dir.join("manifest.json")
}

pub(super) fn enforce_desktop_parity_bundle_preflight(app: &tauri::AppHandle) -> Result<()> {
    if !parity_profile_enabled() {
        return Ok(());
    }
    let channel = std::env::var("CTX_DESKTOP_CHANNEL")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "dev".to_string());
    let surface = std::env::var("CTX_LAUNCH_SURFACE")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "desktop".to_string());

    let bundle_dir = desktop_bundle_dir(app).ok_or_else(|| anyhow!("bundle dir not found"))?;
    let manifest_path = bundle_manifest_path(&bundle_dir);
    let manifest_parent = manifest_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| bundle_dir.clone());
    let manifest_sibling_lock = manifest_parent.join("runtime_lock.v2.json");
    let lock_path = if manifest_sibling_lock.exists() {
        manifest_sibling_lock
    } else {
        bundle_dir.join("runtime_lock.v2.json")
    };
    let lock_raw = std::fs::read_to_string(&lock_path)
        .with_context(|| format!("reading {}", lock_path.display()))?;
    let lock: RuntimeLockV2 = serde_json::from_str(&lock_raw)
        .with_context(|| format!("parsing {}", lock_path.display()))?;
    if lock.version != 2 {
        anyhow::bail!(
            "unsupported runtime lock version {} at {}",
            lock.version,
            lock_path.display()
        );
    }

    let manifest_raw = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("reading {}", manifest_path.display()))?;
    let manifest: DesktopBundledAssetsManifest = serde_json::from_str(&manifest_raw)
        .with_context(|| format!("parsing {}", manifest_path.display()))?;

    let provider_default_targets = host_default_provider_targets();
    let runtime_default_targets = host_default_runtime_targets();
    let image_default_targets = host_default_image_targets();
    let machine_cache_default_targets = host_default_machine_cache_targets();
    let host_os = std::env::consts::OS;
    let host_arch = std::env::consts::ARCH;
    let provider_targets = host_relevant_targets(
        &required_targets_or_default(
            &lock.required.targets.provider,
            &provider_default_targets,
            host_os,
            host_arch,
        ),
        &provider_default_targets,
    );
    let runtime_targets = host_relevant_targets(
        &required_targets_or_default(
            &lock.required.targets.runtime,
            &runtime_default_targets,
            host_os,
            host_arch,
        ),
        &runtime_default_targets,
    );
    let image_targets = host_relevant_targets(
        &required_targets_or_default(
            &lock.required.targets.image,
            &image_default_targets,
            host_os,
            host_arch,
        ),
        &image_default_targets,
    );
    let machine_cache_targets = host_relevant_targets(
        &required_targets_or_default(
            &lock.required.targets.machine_cache,
            &machine_cache_default_targets,
            host_os,
            host_arch,
        ),
        &machine_cache_default_targets,
    );
    let allowed_managed_sources = allowed_source_types_for_profile(&lock);

    let mut failures = Vec::<String>::new();

    for provider_id in &lock.required.provider_ids {
        for target in &provider_targets {
            let Some(entry) = manifest.providers.iter().find(|entry| {
                entry.id == *provider_id && entry.os == target.os && entry.arch == target.arch
            }) else {
                failures.push(format!(
                    "missing provider entry: {} ({}/{})",
                    provider_id, target.os, target.arch
                ));
                continue;
            };
            let command_path = bundle_dir.join(&entry.command);
            if !command_path.exists() {
                failures.push(format!(
                    "missing provider command file: {} ({}/{}) at {}",
                    provider_id,
                    target.os,
                    target.arch,
                    command_path.display()
                ));
            }
        }
    }

    for runtime_id in &lock.required.runtime_ids {
        for target in &runtime_targets {
            let Some(entry) = manifest.runtimes.iter().find(|entry| {
                entry.id == *runtime_id && entry.os == target.os && entry.arch == target.arch
            }) else {
                failures.push(format!(
                    "missing runtime entry: {} ({}/{})",
                    runtime_id, target.os, target.arch
                ));
                continue;
            };
            let root_path = bundle_dir.join(&entry.root);
            if !root_path.exists() {
                failures.push(format!(
                    "missing runtime root dir: {} ({}/{}) at {}",
                    runtime_id,
                    target.os,
                    target.arch,
                    root_path.display()
                ));
                continue;
            }
            let bin_path = root_path.join(&entry.bin);
            if !bin_path.exists() {
                failures.push(format!(
                    "missing runtime binary file: {} ({}/{}) at {}",
                    runtime_id,
                    target.os,
                    target.arch,
                    bin_path.display()
                ));
            }
        }
    }

    for image_id in &lock.required.image_ids {
        for target in &image_targets {
            let managed_source_available = required_component_has_managed_source(
                &lock,
                "image",
                image_id,
                target,
                &allowed_managed_sources,
            );
            let Some(entry) = manifest.images.iter().find(|entry| {
                entry.id == *image_id && entry.os == target.os && entry.arch == target.arch
            }) else {
                if !managed_source_available {
                    failures.push(format!(
                        "missing image entry: {} ({}/{})",
                        image_id, target.os, target.arch
                    ));
                }
                continue;
            };
            let tar_path = bundle_dir.join(&entry.tar);
            if !tar_path.exists() {
                if !managed_source_available {
                    failures.push(format!(
                        "missing image tar file: {} ({}/{}) at {}",
                        image_id,
                        target.os,
                        target.arch,
                        tar_path.display()
                    ));
                }
            }
        }
    }

    for machine_cache_id in &lock.required.machine_cache_ids {
        for target in &machine_cache_targets {
            if !required_component_has_managed_source(
                &lock,
                "machine_cache",
                machine_cache_id,
                target,
                &allowed_managed_sources,
            ) {
                failures.push(format!(
                    "missing machine-cache managed source: {} ({}/{})",
                    machine_cache_id, target.os, target.arch
                ));
            }
        }
    }

    if !failures.is_empty() {
        anyhow::bail!(
            "desktop parity preflight failed (channel={channel} profile=parity surface={surface}): {}",
            failures.join("; ")
        );
    }
    Ok(())
}

pub(super) fn daemon_health(base_url: &str) -> Result<DaemonHealthSummary> {
    daemon_health_with_timeout(base_url, daemon_health_timeout())
}

#[cfg(test)]
thread_local! {
    static DAEMON_HEALTH_CLIENT_BUILD_COUNT: Cell<usize> = const { Cell::new(0) };
}

#[cfg(test)]
fn reset_daemon_health_client_build_count() {
    DAEMON_HEALTH_CLIENT_BUILD_COUNT.with(|count| count.set(0));
}

#[cfg(test)]
fn daemon_health_client_build_count() -> usize {
    DAEMON_HEALTH_CLIENT_BUILD_COUNT.with(Cell::get)
}

fn daemon_health_client(timeout: Duration) -> Result<reqwest::blocking::Client> {
    static DAEMON_HEALTH_CLIENTS: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<u64, reqwest::blocking::Client>>,
    > = std::sync::OnceLock::new();
    let timeout_key = timeout.as_millis() as u64;
    let clients = DAEMON_HEALTH_CLIENTS
        .get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    let mut guard = clients
        .lock()
        .map_err(|err| anyhow!("daemon health client cache poisoned: {err}"))?;
    if let Some(existing) = guard.get(&timeout_key) {
        return Ok(existing.clone());
    }
    #[cfg(test)]
    DAEMON_HEALTH_CLIENT_BUILD_COUNT.with(|count| count.set(count.get() + 1));
    let client = reqwest::blocking::Client::builder()
        .timeout(timeout)
        .build()
        .context("building http client")?;
    guard.insert(timeout_key, client.clone());
    Ok(client)
}

fn daemon_health_with_timeout(base_url: &str, timeout: Duration) -> Result<DaemonHealthSummary> {
    let url = format!("{}/api/health", base_url.trim_end_matches('/'));
    let client = daemon_health_client(timeout)?;
    let res = client.get(url).send().context("requesting /api/health")?;
    let res = res.error_for_status().context("health status")?;
    res.json::<DaemonHealthSummary>()
        .context("parsing /api/health response")
}

fn daemon_health_timeout() -> Duration {
    const DEFAULT_MS: u64 = 5000;
    const MIN_MS: u64 = 100;
    const MAX_MS: u64 = 30000;
    let raw = std::env::var("CTX_DESKTOP_DAEMON_HEALTH_TIMEOUT_MS").unwrap_or_default();
    let parsed = raw.trim().parse::<u64>().ok().unwrap_or(DEFAULT_MS);
    let bounded = parsed.clamp(MIN_MS, MAX_MS);
    Duration::from_millis(bounded)
}

fn normalize_path_for_compare(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| normalize_path(path))
}

pub(super) fn desktop_dev_instance_id() -> &'static str {
    option_env!("CTX_DEV_INSTANCE_ID").unwrap_or("unknown")
}

fn local_daemon_health_matches_expected(
    health: &DaemonHealthSummary,
    expected_data_dir: &Path,
    expected_desktop_version: &str,
    expected_desktop_dev_instance_id: &str,
) -> bool {
    let daemon_data_root = health.data_root.trim();
    if daemon_data_root.is_empty() {
        return false;
    }
    let daemon_root = normalize_path_for_compare(Path::new(daemon_data_root));
    let expected_root = normalize_path_for_compare(expected_data_dir);
    if daemon_root != expected_root {
        return false;
    }
    let expected_version = expected_desktop_version.trim();
    if expected_version.is_empty() {
        return false;
    }
    if health.compatibility.desktop_exact_version.trim() != expected_version {
        return false;
    }
    if cfg!(debug_assertions) {
        let expected_dev_instance_id = expected_desktop_dev_instance_id.trim();
        if expected_dev_instance_id.is_empty() {
            return false;
        }
        if health.compatibility.desktop_dev_instance_id.trim() != expected_dev_instance_id {
            return false;
        }
    }
    true
}

fn display_nonempty(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        "<empty>".to_string()
    } else {
        trimmed.to_string()
    }
}

fn spawned_local_daemon_incompatibility_message(
    base_url: &str,
    expected_data_dir: &Path,
    expected_desktop_version: &str,
    expected_desktop_dev_instance_id: &str,
    health: &DaemonHealthSummary,
) -> String {
    format!(
        "spawned local daemon is incompatible (expected_version={}, daemon_version={}, expected_dev_instance_id={}, daemon_dev_instance_id={}, expected_data_dir={}, daemon_data_root={}, daemon_pid={}, url={})",
        display_nonempty(expected_desktop_version),
        display_nonempty(&health.compatibility.desktop_exact_version),
        display_nonempty(expected_desktop_dev_instance_id),
        display_nonempty(&health.compatibility.desktop_dev_instance_id),
        expected_data_dir.display(),
        display_nonempty(&health.data_root),
        health.pid,
        base_url,
    )
}

pub(super) fn existing_local_daemon_matches(
    base_url: &str,
    expected_data_dir: &Path,
    expected_desktop_version: &str,
    expected_desktop_dev_instance_id: &str,
) -> Result<bool> {
    let health = daemon_health(base_url)?;
    Ok(local_daemon_health_matches_expected(
        &health,
        expected_data_dir,
        expected_desktop_version,
        expected_desktop_dev_instance_id,
    ))
}

pub(super) fn existing_local_daemon_matches_or_absent(
    base_url: &str,
    expected_data_dir: &Path,
    expected_desktop_version: &str,
    expected_desktop_dev_instance_id: &str,
) -> bool {
    existing_local_daemon_matches(
        base_url,
        expected_data_dir,
        expected_desktop_version,
        expected_desktop_dev_instance_id,
    )
    .unwrap_or(false)
}

pub(super) fn probe_daemon_health(base_url: &str) -> Result<()> {
    let _ = daemon_health(base_url)?;
    Ok(())
}

pub(super) fn probe_local_daemon_health_with_retry(base_url: &str) -> Result<()> {
    let mut last_err: Option<anyhow::Error> = None;
    for attempt in 0..LOCAL_DAEMON_HEALTH_RETRIES {
        match probe_daemon_health(base_url) {
            Ok(()) => return Ok(()),
            Err(err) => last_err = Some(err),
        }
        let delay = LOCAL_DAEMON_HEALTH_BASE_DELAY_MS.saturating_mul((attempt + 1) as u64);
        std::thread::sleep(Duration::from_millis(delay));
    }
    Err(last_err.unwrap_or_else(|| anyhow!("requesting /api/health failed")))
}

pub(super) fn probe_daemon_health_with_retry(
    base_url: &str,
    local_port: u16,
    tunnel: &mut Child,
    stderr_log: &std::sync::Arc<std::sync::Mutex<String>>,
) -> Result<()> {
    let mut last_err: Option<anyhow::Error> = None;
    for attempt in 0..SSH_TUNNEL_HEALTH_RETRIES {
        match probe_daemon_health(base_url) {
            Ok(()) => return Ok(()),
            Err(err) => {
                last_err = Some(err);
                if let Ok(Some(status)) = tunnel.try_wait() {
                    let stderr = ssh_log_snippet(stderr_log);
                    if stderr.is_empty() {
                        return Err(anyhow!("ssh tunnel exited ({status})"));
                    }
                    return Err(anyhow!("ssh tunnel exited ({status}): {stderr}"));
                }
            }
        }
        let delay = SSH_TUNNEL_HEALTH_BASE_DELAY_MS.saturating_mul((attempt + 1) as u64);
        std::thread::sleep(Duration::from_millis(delay));
    }
    let err = last_err.unwrap_or_else(|| anyhow!("requesting /api/health failed"));
    let stderr = ssh_log_snippet(stderr_log);
    let tunnel_state = match tunnel.try_wait() {
        Ok(Some(status)) => format!("ssh tunnel exited ({status})"),
        Ok(None) => "ssh tunnel still running".to_string(),
        Err(e) => format!("ssh tunnel state unknown ({e})"),
    };
    let mut details = format!("{tunnel_state}; local port {local_port}");
    if !stderr.is_empty() {
        details.push_str(&format!("; ssh stderr: {stderr}"));
    }
    Err(anyhow!("{err:#}; {details}"))
}

fn ssh_log_snippet(stderr_log: &std::sync::Arc<std::sync::Mutex<String>>) -> String {
    let log = stderr_log.lock().ok();
    let Some(log) = log.as_ref() else {
        return String::new();
    };
    let snippet = log.trim();
    if snippet.is_empty() {
        String::new()
    } else {
        snippet.to_string()
    }
}

fn truncate_tail_chars(value: &str, max_chars: usize) -> String {
    let total = value.chars().count();
    if total <= max_chars {
        return value.to_string();
    }
    let skip = total - max_chars;
    let mut idx = 0;
    let mut seen = 0;
    for (i, _) in value.char_indices() {
        if seen == skip {
            idx = i;
            break;
        }
        seen += 1;
    }
    value[idx..].to_string()
}

fn daemon_stderr_snippet(path: Option<&Path>) -> String {
    let Some(path) = path else {
        return String::new();
    };
    let Ok(bytes) = std::fs::read(path) else {
        return String::new();
    };
    let text = String::from_utf8_lossy(&bytes);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        String::new()
    } else {
        truncate_tail_chars(trimmed, 1200)
    }
}

pub(super) fn daemon_data_dir(_app: &tauri::AppHandle) -> Result<PathBuf> {
    if let Ok(raw) = std::env::var(DESKTOP_DAEMON_DATA_DIR_ENV) {
        let raw = raw.trim();
        if !raw.is_empty() {
            let p = PathBuf::from(raw);
            if !p.is_absolute() {
                anyhow::bail!("{DESKTOP_DAEMON_DATA_DIR_ENV} must be an absolute path");
            }
            return Ok(p);
        }
    }
    let root = ctx_fs::paths::default_ctx_home().context("resolving ctx home")?;
    std::fs::create_dir_all(&root).context("creating ctx home dir")?;
    Ok(root)
}

fn current_arch_token() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else {
        std::env::consts::ARCH
    }
}

fn resource_bin(app: &tauri::AppHandle, name: &str) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(res) = app.path().resource_dir().ok() {
        let bin_ext = if cfg!(target_os = "windows") {
            ".exe"
        } else {
            ""
        };
        let arch = current_arch_token();
        let prefix = format!("{name}-{arch}");
        for base in [res.join("bin"), res.clone()] {
            let Ok(entries) = std::fs::read_dir(&base) else {
                continue;
            };
            let mut paths: Vec<PathBuf> =
                entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
            paths.sort();
            for p in paths {
                if !p.is_file() {
                    continue;
                }
                let Some(file_name) = p.file_name().and_then(|s| s.to_str()) else {
                    continue;
                };
                if !file_name.starts_with(&prefix) {
                    continue;
                }
                if !bin_ext.is_empty() && !file_name.ends_with(bin_ext) {
                    continue;
                }
                candidates.push(p);
            }
        }

        // Fall back to generic names only after we try arch-specific candidates.
        candidates.push(res.join("bin").join(format!("{name}{bin_ext}")));
        candidates.push(res.join(format!("{name}{bin_ext}")));
    }

    for c in candidates {
        if c.exists() && path_matches_current_platform_binary(&c) {
            return Some(c);
        }
    }
    None
}

fn dev_bin(name: &str) -> Option<PathBuf> {
    let bin_ext = if cfg!(target_os = "windows") {
        ".exe"
    } else {
        ""
    };
    if let Ok(raw) = std::env::var("CTX_DESKTOP_DEV_BIN_DIR") {
        let raw = raw.trim();
        if !raw.is_empty() {
            let candidate = PathBuf::from(raw).join(format!("{name}{bin_ext}"));
            if candidate.exists() && path_matches_current_platform_binary(&candidate) {
                return Some(candidate);
            }
        }
    }
    if let Ok(target_dir) = std::env::var("CARGO_TARGET_DIR") {
        let candidate = PathBuf::from(target_dir)
            .join("debug")
            .join(format!("{name}{bin_ext}"));
        if candidate.exists() && path_matches_current_platform_binary(&candidate) {
            return Some(candidate);
        }
    }
    // Desktop prep syncs host binaries into src-tauri/bin; prefer this before generic core/target.
    let synced_bin = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("bin")
        .join(format!("{name}{bin_ext}"));
    if synced_bin.exists() && path_matches_current_platform_binary(&synced_bin) {
        return Some(synced_bin);
    }

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())?
        .to_path_buf(); // core/
    let candidate = root
        .join("target")
        .join("debug")
        .join(format!("{name}{bin_ext}"));
    if candidate.exists() && path_matches_current_platform_binary(&candidate) {
        return Some(candidate);
    }
    None
}

fn resolve_daemon_bin(app: &tauri::AppHandle) -> Result<PathBuf> {
    if cfg!(debug_assertions) {
        return dev_bin(DESKTOP_DAEMON_BIN_NAME).with_context(|| format!(
            "missing development binary for `{DESKTOP_DAEMON_BIN_NAME}` at expected path (run `pnpm -C core desktop:prep` or rerun desktop_sync_resources after building `ctx-http`)"
        ));
    }
    resource_bin(app, DESKTOP_DAEMON_BIN_NAME).with_context(|| {
        format!("missing bundled binary for `{DESKTOP_DAEMON_BIN_NAME}` in application resources")
    })
}

fn resolve_optional_bin(app: &tauri::AppHandle, name: &str) -> Option<PathBuf> {
    if cfg!(debug_assertions) {
        return dev_bin(name);
    }
    resource_bin(app, name)
}

fn path_matches_current_platform_binary(path: &Path) -> bool {
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut header = [0u8; 4];
    if file.read_exact(&mut header).is_err() {
        return false;
    }

    if cfg!(target_os = "linux") {
        header == [0x7f, b'E', b'L', b'F']
    } else if cfg!(target_os = "windows") {
        header[0..2] == [b'M', b'Z']
    } else if cfg!(target_os = "macos") {
        matches!(
            header,
            // Mach-O 32-bit / 64-bit
            [0xFE, 0xED, 0xFA, 0xCE]
                | [0xCE, 0xFA, 0xED, 0xFE]
                | [0xFE, 0xED, 0xFA, 0xCF]
                | [0xCF, 0xFA, 0xED, 0xFE]
                // Fat (universal) binaries
                | [0xCA, 0xFE, 0xBA, 0xBE]
                | [0xBE, 0xBA, 0xFE, 0xCA]
        )
    } else {
        true
    }
}

fn dev_web_dist() -> Option<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())?
        .join("web")
        .join("dist"); // core/apps/web/dist
    if dir.exists() {
        Some(dir)
    } else {
        None
    }
}

fn dev_bundle_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("bundles");
    if dir.exists() {
        Some(dir)
    } else {
        None
    }
}

fn desktop_bundle_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path()
        .resource_dir()
        .ok()
        .map(|p| p.join("bundles"))
        .filter(|p| p.exists())
        .or_else(dev_bundle_dir)
}

#[cfg(target_os = "linux")]
fn systemd_run_available() -> bool {
    match Command::new("systemd-run").arg("--version").status() {
        Ok(status) => status.success(),
        Err(_) => false,
    }
}

#[cfg(target_os = "linux")]
fn systemd_user_available() -> bool {
    match Command::new("systemctl")
        .arg("--user")
        .arg("show-environment")
        .status()
    {
        Ok(status) => status.success(),
        Err(_) => false,
    }
}

#[cfg(target_os = "linux")]
fn should_use_systemd_scope() -> bool {
    systemd_run_available() && systemd_user_available()
}

#[cfg(not(target_os = "linux"))]
fn should_use_systemd_scope() -> bool {
    false
}

pub(super) fn env_bool(name: &str, default: bool) -> bool {
    let Ok(raw) = std::env::var(name) else {
        return default;
    };
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => true,
        "0" | "false" | "no" | "off" => false,
        _ => default,
    }
}

pub(super) fn stop_systemd_scope(_unit: &str) {
    #[cfg(target_os = "linux")]
    {
        let scope = if _unit.ends_with(".scope") {
            _unit.to_string()
        } else {
            format!("{_unit}.scope")
        };
        let _ = Command::new("systemctl")
            .arg("--user")
            .arg("stop")
            .arg(&scope)
            .status();
        let _ = Command::new("systemctl")
            .arg("--user")
            .arg("reset-failed")
            .arg(&scope)
            .status();
    }
}

pub(super) fn systemd_scope_for_local_daemon_url(base_url: &str) -> Option<String> {
    let url = Url::parse(base_url).ok()?;
    let port = url.port()?;
    Some(format!("ctx-daemon-{port}"))
}

const LOCAL_DAEMON_PATH_PROBE_START: &str = "__CTX_DAEMON_PATH_START__";
const LOCAL_DAEMON_PATH_PROBE_END: &str = "__CTX_DAEMON_PATH_END__";

fn trim_non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn local_daemon_login_shell_path() -> Option<PathBuf> {
    let shell_from_env = std::env::var_os("SHELL")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute() && path.exists());
    shell_from_env.or_else(|| {
        if cfg!(target_os = "macos") {
            let fallback = PathBuf::from("/bin/zsh");
            if fallback.exists() {
                return Some(fallback);
            }
        }
        None
    })
}

fn parse_local_daemon_path_probe_output(stdout: &str) -> Option<String> {
    let start = stdout.find(LOCAL_DAEMON_PATH_PROBE_START)?;
    let after_start = start + LOCAL_DAEMON_PATH_PROBE_START.len();
    let end_rel = stdout[after_start..].find(LOCAL_DAEMON_PATH_PROBE_END)?;
    trim_non_empty(&stdout[after_start..after_start + end_rel])
}

fn probe_local_daemon_path_via_shell(shell_path: &Path) -> Option<String> {
    let probe_command = format!(
        "printf '%s' '{start}'; printenv PATH; printf '%s' '{end}'",
        start = LOCAL_DAEMON_PATH_PROBE_START,
        end = LOCAL_DAEMON_PATH_PROBE_END,
    );
    let output = Command::new(shell_path)
        .arg("-l")
        .arg("-c")
        .arg(&probe_command)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_local_daemon_path_probe_output(&String::from_utf8_lossy(&output.stdout))
}

fn resolve_local_daemon_path_env() -> Option<String> {
    // Finder-launched macOS apps often miss user shell PATH entries like ~/.local/bin. Probe the
    // login shell once at desktop daemon spawn so downstream CLI discovery resolves the same tools
    // a user expects in Terminal, then pass that PATH explicitly to the daemon process.
    if cfg!(target_os = "macos") {
        if let Some(shell_path) = local_daemon_login_shell_path() {
            if let Some(shell_path_env) = probe_local_daemon_path_via_shell(&shell_path) {
                return Some(shell_path_env);
            }
        }
    }
    std::env::var("PATH")
        .ok()
        .and_then(|value| trim_non_empty(&value))
}

pub(super) fn spawn_daemon(
    app: &tauri::AppHandle,
    data_dir: &Path,
    wait_for_health: bool,
) -> Result<(String, Child, bool)> {
    let prefer_systemd_scope = should_use_systemd_scope();
    if prefer_systemd_scope {
        match spawn_daemon_with_mode(app, data_dir, true, wait_for_health) {
            Ok(v) => return Ok(v),
            Err(e) => {
                // Fall back to a direct child process when systemd user services are unavailable
                // (common in headless/dev environments).
                return spawn_daemon_with_mode(app, data_dir, false, wait_for_health)
                    .with_context(|| format!("spawning ctx daemon via systemd-run failed: {e:#}"));
            }
        }
    }
    spawn_daemon_with_mode(app, data_dir, false, wait_for_health)
}

fn spawn_daemon_with_mode(
    app: &tauri::AppHandle,
    data_dir: &Path,
    use_systemd_scope: bool,
    wait_for_health: bool,
) -> Result<(String, Child, bool)> {
    let ctx_bin = resolve_daemon_bin(app)?;
    let mcp_bin = resolve_optional_bin(app, "ctx-mcp");
    let daemon_path_env = resolve_daemon_path_env();

    let web_dist = app
        .path()
        .resource_dir()
        .ok()
        .and_then(|p| {
            let candidates = [
                p.join("web").join("dist"),
                p.join("web-dist"),
                p.join("dist"),
            ];
            candidates.into_iter().find(|c| c.exists())
        })
        .or_else(dev_web_dist);
    let bundle_dir = app
        .path()
        .resource_dir()
        .ok()
        .map(|p| p.join("bundles"))
        .filter(|p| p.exists())
        .or_else(dev_bundle_dir);
    let resolved_path_env = resolve_local_daemon_path_env();

    // Container-mode Codex sessions need a CODEX_HOME available inside the Linux harness.
    // The daemon can seed `~/.codex/auth.json` into a daemon-managed location, but only if the
    // host auth file actually exists (otherwise enabling seeding would create hard errors).
    let seed_codex_auth = std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".codex").join("auth.json").exists())
        .unwrap_or(false);

    let local_port = pick_unused_local_port()?;
    let base_url = format!("http://127.0.0.1:{local_port}");
    let systemd_unit = format!("ctx-daemon-{local_port}");

    if use_systemd_scope {
        // Best-effort cleanup in case a prior run left stale units around.
        stop_systemd_scope("ctx-daemon");
        stop_systemd_scope(&systemd_unit);
    }
    let mut cmd = if use_systemd_scope {
        let mut cmd = Command::new("systemd-run");
        cmd.arg("--user")
            .arg("--scope")
            .arg("--unit")
            .arg(&systemd_unit)
            .arg("--same-dir");
        if let Some(path_env) = resolved_path_env.as_ref() {
            cmd.arg("--setenv").arg(format!("PATH={path_env}"));
        }
        if let Some(dist) = web_dist.as_ref() {
            cmd.arg("--setenv")
                .arg(format!("CTX_WEB_DIST={}", dist.to_string_lossy()));
        }
        if let Some(mcp) = mcp_bin.as_ref() {
            cmd.arg("--setenv")
                .arg(format!("CTX_MCP_COMMAND={}", mcp.to_string_lossy()));
        }
        if let Some(bundle) = bundle_dir.as_ref() {
            cmd.arg("--setenv")
                .arg(format!("CTX_BUNDLE_DIR={}", bundle.to_string_lossy()));
        }
        if seed_codex_auth {
            cmd.arg("--setenv").arg("CTX_SEED_CODEX_AUTH_FROM_HOST=1");
        }
        if let Ok(appimage) = std::env::var("APPIMAGE") {
            cmd.arg("--setenv")
                .arg(format!("CTX_APPIMAGE_PATH={appimage}"));
        }
        if let Some(path_value) = daemon_path_env.as_deref() {
            cmd.arg("--setenv")
                .arg(format!("PATH={}", path_value.to_string_lossy()));
        }
        for key in DAEMON_ENV_PASSTHROUGH {
            if let Ok(value) = std::env::var(key) {
                cmd.arg("--setenv").arg(format!("{key}={value}"));
            }
        }
        cmd.arg(&ctx_bin);
        cmd
    } else {
        let mut cmd = Command::new(&ctx_bin);
        if let Some(path_env) = resolved_path_env.as_ref() {
            cmd.env("PATH", path_env);
        }
        if let Some(dist) = web_dist.as_ref() {
            cmd.env("CTX_WEB_DIST", dist.to_string_lossy().to_string());
        }
        if let Some(mcp) = mcp_bin.as_ref() {
            cmd.env("CTX_MCP_COMMAND", mcp.to_string_lossy().to_string());
        }
        if let Some(bundle) = bundle_dir.as_ref() {
            cmd.env("CTX_BUNDLE_DIR", bundle.to_string_lossy().to_string());
        }
        if seed_codex_auth {
            cmd.env("CTX_SEED_CODEX_AUTH_FROM_HOST", "1");
        }
        if let Ok(appimage) = std::env::var("APPIMAGE") {
            cmd.env("CTX_APPIMAGE_PATH", appimage.clone());
        }
        if let Some(path_value) = daemon_path_env.as_deref() {
            cmd.env("PATH", path_value);
        }
        for key in DAEMON_ENV_PASSTHROUGH {
            if let Ok(value) = std::env::var(key) {
                cmd.env(key, value);
            }
        }
        cmd
    };

    cmd.arg("serve")
        .arg("--bind")
        .arg(format!("127.0.0.1:{local_port}"))
        .arg("--data-dir")
        .arg(data_dir.to_string_lossy().to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null());

    let mut stderr_path: Option<PathBuf> = None;
    if use_systemd_scope {
        cmd.stderr(Stdio::inherit());
    } else {
        let log_dir = data_dir.join("logs");
        if let Err(err) = std::fs::create_dir_all(&log_dir) {
            eprintln!(
                "failed to create daemon log dir {}: {err}",
                log_dir.display()
            );
            cmd.stderr(Stdio::inherit());
        } else {
            let path = log_dir.join("desktop-daemon-stderr.log");
            match OpenOptions::new().create(true).append(true).open(&path) {
                Ok(file) => {
                    stderr_path = Some(path);
                    cmd.stderr(file);
                }
                Err(err) => {
                    eprintln!("failed to open daemon stderr log: {err}");
                    cmd.stderr(Stdio::inherit());
                }
            }
        }
    }

    let mut child = cmd.spawn().context("spawning ctx daemon")?;
    if wait_for_health {
        if let Err(err) = probe_local_daemon_health_with_retry(&base_url) {
            if let Ok(Some(status)) = child.try_wait() {
                let stderr = daemon_stderr_snippet(stderr_path.as_deref());
                let mut msg = format!("{err:#}; daemon exited ({status})");
                if !stderr.is_empty() {
                    msg.push_str(&format!("; stderr: {stderr}"));
                }
                return Err(anyhow!(msg));
            }
            let stderr = daemon_stderr_snippet(stderr_path.as_deref());
            if !stderr.is_empty() {
                return Err(anyhow!("{err:#}; stderr: {stderr}"));
            }
            return Err(err).context("waiting for daemon health");
        }
    } else {
        let base_url = base_url.clone();
        let stderr_path = stderr_path.clone();
        std::thread::spawn(move || {
            if let Err(err) = probe_local_daemon_health_with_retry(&base_url) {
                let stderr = daemon_stderr_snippet(stderr_path.as_deref());
                if stderr.is_empty() {
                    eprintln!("ctx daemon health check failed after spawn: {err:#}");
                } else {
                    eprintln!(
                        "ctx daemon health check failed after spawn: {err:#}; stderr: {stderr}"
                    );
                }
            }
        });
    }
    Ok((base_url, child, use_systemd_scope))
}

#[allow(dead_code)]
fn is_executable(path: &Path) -> bool {
    path.exists()
}

pub(super) fn try_kill_child(mut child: Child) -> Result<()> {
    let _ = child.kill();
    let _ = child.wait();
    Ok(())
}

pub(super) struct SpawnedLocalDaemonReady {
    pub(super) url: String,
    pub(super) token: String,
    pub(super) child: Child,
    pub(super) systemd_scope: bool,
}

struct PendingSpawnedLocalDaemon {
    url: String,
    child: Option<Child>,
    systemd_scope: bool,
}

impl PendingSpawnedLocalDaemon {
    fn new(url: String, child: Child, systemd_scope: bool) -> Self {
        Self {
            url,
            child: Some(child),
            systemd_scope,
        }
    }

    fn url(&self) -> &str {
        &self.url
    }

    fn disarm(mut self) -> Result<(String, Child, bool)> {
        let child = self
            .child
            .take()
            .ok_or_else(|| anyhow!("spawned daemon child missing"))?;
        let url = std::mem::take(&mut self.url);
        Ok((url, child, self.systemd_scope))
    }
}

impl Drop for PendingSpawnedLocalDaemon {
    fn drop(&mut self) {
        if let Some(child) = self.child.take() {
            cleanup_rejected_spawned_local_daemon(child, self.systemd_scope, &self.url);
        }
    }
}

pub(super) fn spawn_and_validate_local_daemon(
    app: &tauri::AppHandle,
    data_dir: &Path,
    desktop_version: &str,
    desktop_dev_instance_id: &str,
) -> Result<SpawnedLocalDaemonReady> {
    let (url, child, systemd_scope) = spawn_daemon(app, data_dir, true)?;
    let pending = PendingSpawnedLocalDaemon::new(url, child, systemd_scope);
    let health = daemon_health(pending.url())
        .context("requesting /api/health for spawned local daemon compatibility")?;
    let compatible = local_daemon_health_matches_expected(
        &health,
        data_dir,
        desktop_version,
        desktop_dev_instance_id,
    );
    if !compatible {
        anyhow::bail!(
            "{}",
            spawned_local_daemon_incompatibility_message(
                pending.url(),
                data_dir,
                desktop_version,
                desktop_dev_instance_id,
                &health
            )
        );
    }
    let auth = read_daemon_auth_with_retry(data_dir)?;
    let (url, child, systemd_scope) = pending.disarm()?;
    Ok(SpawnedLocalDaemonReady {
        url,
        token: auth.token,
        child,
        systemd_scope,
    })
}

fn cleanup_rejected_spawned_local_daemon(child: Child, systemd_scope: bool, base_url: &str) {
    let _ = try_kill_child(child);
    if systemd_scope {
        if let Some(unit) = systemd_scope_for_local_daemon_url(base_url) {
            stop_systemd_scope(&unit);
        }
    }
}

#[cfg(test)]
mod desktop_daemon_tests {
    use super::*;
    use crate::desktop_local_daemon::{
        apply_validated_local_connection, connect_local_with_sources, lock_local_connect_gate,
    };

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<std::ffi::OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(previous) = self.previous.as_ref() {
                std::env::set_var(self.key, previous);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    #[test]
    fn ssh_config_override_normalization() {
        assert_eq!(
            normalized_ssh_config_override(" /tmp/ctx-fixture-ssh-config "),
            Some("/tmp/ctx-fixture-ssh-config".to_string())
        );
        assert_eq!(normalized_ssh_config_override("   "), None);
    }

    #[test]
    fn parse_local_daemon_path_probe_output_extracts_marker_payload() {
        let output = format!(
            "noise before\n{start}/Users/test/.local/bin:/usr/bin{end}\nnoise after\n",
            start = LOCAL_DAEMON_PATH_PROBE_START,
            end = LOCAL_DAEMON_PATH_PROBE_END,
        );
        assert_eq!(
            parse_local_daemon_path_probe_output(&output),
            Some("/Users/test/.local/bin:/usr/bin".to_string())
        );
        assert_eq!(
            parse_local_daemon_path_probe_output("missing markers"),
            None
        );
    }

    #[cfg(unix)]
    fn write_shell_probe_script(contents: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let path = std::env::temp_dir().join(format!("ctx-shell-probe-{}", uuid::Uuid::new_v4()));
        std::fs::write(&path, contents).expect("write shell probe script");
        let mut perms = std::fs::metadata(&path)
            .expect("stat shell probe script")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).expect("chmod shell probe script");
        path
    }

    #[test]
    #[cfg(unix)]
    fn probe_local_daemon_path_via_shell_reads_marker_payload() {
        let shell_path = write_shell_probe_script(&format!(
            "#!/bin/sh\nprintf 'boot noise\\n'\nprintf '%s/tmp/ctx-user-bin:%s%s\\n' '{start}' '/usr/bin' '{end}'\n",
            start = LOCAL_DAEMON_PATH_PROBE_START,
            end = LOCAL_DAEMON_PATH_PROBE_END,
        ));
        let parsed = probe_local_daemon_path_via_shell(&shell_path);
        assert_eq!(parsed, Some("/tmp/ctx-user-bin:/usr/bin".to_string()));
        std::fs::remove_file(shell_path).ok();
    }

    #[test]
    #[cfg(unix)]
    fn probe_local_daemon_path_via_shell_returns_none_without_marker() {
        let shell_path = write_shell_probe_script("#!/bin/sh\nprintf '/usr/bin\\n'\n");
        let parsed = probe_local_daemon_path_via_shell(&shell_path);
        assert_eq!(parsed, None);
        std::fs::remove_file(shell_path).ok();
    }

    #[test]
    fn local_daemon_health_match_requires_expected_data_root_and_mode_specific_identity() {
        let expected_dir =
            std::env::temp_dir().join(format!("ctx-daemon-health-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&expected_dir).expect("create expected dir");
        let other_dir = expected_dir.join("other");
        std::fs::create_dir_all(&other_dir).expect("create other dir");

        let matching = DaemonHealthSummary {
            pid: 42,
            data_root: expected_dir.to_string_lossy().to_string(),
            compatibility: DaemonHealthCompatibility {
                desktop_exact_version: "1.2.3".to_string(),
                desktop_dev_instance_id: "dev-wt-a".to_string(),
            },
        };
        assert!(local_daemon_health_matches_expected(
            &matching,
            &expected_dir,
            "1.2.3",
            "dev-wt-a",
        ));
        if cfg!(debug_assertions) {
            assert!(!local_daemon_health_matches_expected(
                &matching,
                &expected_dir,
                "9.9.9",
                "dev-wt-a",
            ));
            assert!(!local_daemon_health_matches_expected(
                &matching,
                &expected_dir,
                "1.2.3",
                "dev-wt-b",
            ));
        }

        let wrong_root = DaemonHealthSummary {
            pid: 42,
            data_root: other_dir.to_string_lossy().to_string(),
            compatibility: DaemonHealthCompatibility {
                desktop_exact_version: "1.2.3".to_string(),
                desktop_dev_instance_id: "dev-wt-a".to_string(),
            },
        };
        assert!(!local_daemon_health_matches_expected(
            &wrong_root,
            &expected_dir,
            "1.2.3",
            "dev-wt-a",
        ));

        std::fs::remove_dir_all(&expected_dir).ok();
    }

    #[test]
    fn existing_local_daemon_match_errors_are_treated_as_absent() {
        let expected_dir = std::env::temp_dir().join(format!(
            "ctx-daemon-existing-match-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&expected_dir).expect("create expected dir");

        assert!(!existing_local_daemon_matches_or_absent(
            "not-a-url",
            &expected_dir,
            "1.2.3",
            "dev-wt-a",
        ));

        std::fs::remove_dir_all(&expected_dir).ok();
    }

    #[test]
    fn spawned_daemon_incompatibility_message_reports_expected_and_actual_values() {
        let expected_dir = std::env::temp_dir().join(format!(
            "ctx-daemon-spawn-incompatible-{}",
            uuid::Uuid::new_v4()
        ));
        let health = DaemonHealthSummary {
            pid: 4242,
            data_root: "/tmp/ctx-daemon-other".to_string(),
            compatibility: DaemonHealthCompatibility {
                desktop_exact_version: "0.1.1".to_string(),
                desktop_dev_instance_id: "dev-other".to_string(),
            },
        };

        let msg = spawned_local_daemon_incompatibility_message(
            "http://127.0.0.1:4123",
            &expected_dir,
            "0.2.20",
            "dev-main",
            &health,
        );

        assert!(msg.contains("expected_version=0.2.20"));
        assert!(msg.contains("daemon_version=0.1.1"));
        assert!(msg.contains("expected_dev_instance_id=dev-main"));
        assert!(msg.contains("daemon_dev_instance_id=dev-other"));
        assert!(msg.contains("daemon_data_root=/tmp/ctx-daemon-other"));
        assert!(msg.contains("daemon_pid=4242"));
        assert!(msg.contains("url=http://127.0.0.1:4123"));
    }

    #[test]
    fn build_effective_daemon_path_merges_current_shell_and_common_dirs() {
        let home_dir =
            std::env::temp_dir().join(format!("ctx-daemon-path-home-{}", uuid::Uuid::new_v4()));
        let current = std::ffi::OsString::from("/usr/bin:/bin");
        let shell = std::ffi::OsString::from("/tmp/custom/bin:/usr/bin");

        let merged = build_effective_daemon_path(
            Some(current.as_os_str()),
            Some(shell.as_os_str()),
            Some(home_dir.as_path()),
        )
        .expect("merged path");
        let parts = std::env::split_paths(&merged).collect::<Vec<_>>();

        assert_eq!(parts[0], PathBuf::from("/usr/bin"));
        assert_eq!(parts[1], PathBuf::from("/bin"));
        assert!(parts.contains(&PathBuf::from("/tmp/custom/bin")));
        assert!(parts.contains(&home_dir.join(".local").join("bin")));
        assert!(parts.contains(&PathBuf::from("/opt/homebrew/bin")));
        assert_eq!(
            parts
                .iter()
                .filter(|entry| **entry == PathBuf::from("/usr/bin"))
                .count(),
            1
        );
    }

    #[test]
    fn extract_shell_path_reads_sentinel_payload() {
        let raw = b"noise before __CTX_DAEMON_PATH_BEGIN__/tmp/alpha:/tmp/beta__CTX_DAEMON_PATH_END__ trailing";
        let parsed = extract_shell_path(raw).expect("parsed path");
        assert_eq!(parsed, std::ffi::OsString::from("/tmp/alpha:/tmp/beta"));
    }

    #[test]
    #[cfg(unix)]
    fn read_login_shell_path_uses_shell_output_markers() {
        use std::os::unix::fs::PermissionsExt;

        let temp =
            std::env::temp_dir().join(format!("ctx-daemon-shell-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp).expect("create temp dir");
        let shell_path = temp.join("fake-shell");
        std::fs::write(
            &shell_path,
            format!(
                "#!/bin/sh\nprintf 'prefix {DAEMON_PATH_SENTINEL_BEGIN}/tmp/fake-cursor:/usr/bin{DAEMON_PATH_SENTINEL_END} suffix'\n"
            ),
        )
        .expect("write fake shell");
        let mut perms = std::fs::metadata(&shell_path)
            .expect("stat fake shell")
            .permissions();
        perms.set_mode(perms.mode() | 0o111);
        std::fs::set_permissions(&shell_path, perms).expect("chmod fake shell");

        let resolved = read_login_shell_path(&shell_path).expect("resolved shell path");
        assert_eq!(resolved, std::ffi::OsString::from("/tmp/fake-cursor:/usr/bin"));
        std::fs::remove_dir_all(&temp).ok();
    }

    #[test]
    #[cfg(unix)]
    fn resolve_daemon_path_env_prefers_current_path_and_shell_discovery() {
        use std::os::unix::fs::PermissionsExt;

        let temp =
            std::env::temp_dir().join(format!("ctx-daemon-shell-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp).expect("create temp dir");
        let shell_path = temp.join("fake-shell");
        std::fs::write(
            &shell_path,
            format!(
                "#!/bin/sh\nprintf '{DAEMON_PATH_SENTINEL_BEGIN}/tmp/fake-cursor:/usr/bin{DAEMON_PATH_SENTINEL_END}'\n"
            ),
        )
        .expect("write fake shell");
        let mut perms = std::fs::metadata(&shell_path)
            .expect("stat fake shell")
            .permissions();
        perms.set_mode(perms.mode() | 0o111);
        std::fs::set_permissions(&shell_path, perms).expect("chmod fake shell");

        let _shell = EnvVarGuard::set("SHELL", shell_path.as_os_str());
        let _path = EnvVarGuard::set("PATH", "/usr/bin:/bin");
        let _home = EnvVarGuard::set("HOME", temp.as_os_str());

        let resolved = resolve_daemon_path_env().expect("resolved daemon path");
        let parts = std::env::split_paths(&resolved).collect::<Vec<_>>();
        assert_eq!(parts[0], PathBuf::from("/usr/bin"));
        assert_eq!(parts[1], PathBuf::from("/bin"));
        assert!(parts.contains(&PathBuf::from("/tmp/fake-cursor")));
        std::fs::remove_dir_all(&temp).ok();
    }

    #[test]
    fn daemon_health_reuses_cached_client_for_same_timeout() {
        reset_daemon_health_client_build_count();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind test listener");
        let addr = listener.local_addr().expect("listener addr");
        let server = std::thread::spawn(move || {
            let body =
                "{\"pid\":1,\"data_root\":\"/tmp/test\",\"compatibility\":{\"desktop_exact_version\":\"1.0.0\",\"desktop_dev_instance_id\":\"dev\"}}";
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().expect("accept request");
                let mut buf = [0_u8; 1024];
                let _ = std::io::Read::read(&mut stream, &mut buf);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body,
                );
                std::io::Write::write_all(&mut stream, response.as_bytes())
                    .expect("write response");
            }
        });

        let base_url = format!("http://{}", addr);
        for _ in 0..2 {
            let health = daemon_health_with_timeout(&base_url, Duration::from_secs(5))
                .expect("daemon health succeeds");
            assert_eq!(health.pid, 1);
        }

        server.join().expect("join test server");
        assert_eq!(daemon_health_client_build_count(), 1);
    }

    #[test]
    fn replacement_validation_failure_keeps_existing_active_connection() {
        let state = ConnectionManager::default();
        state.set_local_attached(
            "http://127.0.0.1:4399".to_string(),
            "existing-token".to_string(),
            None,
            LocalConnectionSource::ExistingCompatibleDaemon,
        );

        let err =
            apply_validated_local_connection(&state, Err(anyhow!("spawn validation failed")), None)
                .expect_err("spawn failure should not replace an existing healthy connection");
        assert!(format!("{err:#}").contains("spawn validation failed"));

        let info = state.info();
        assert!(matches!(info.kind, DesktopConnectionKind::Local));
        assert_eq!(info.base_url.as_deref(), Some("http://127.0.0.1:4399"));
        assert_eq!(info.token.as_deref(), Some("existing-token"));
    }

    #[test]
    fn local_connect_gate_serializes_callers() {
        let first_guard = lock_local_connect_gate().expect("lock first gate holder");
        let ready = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let entered = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let ready_for_thread = std::sync::Arc::clone(&ready);
        let entered_for_thread = std::sync::Arc::clone(&entered);
        let handle = std::thread::spawn(move || {
            ready_for_thread.store(true, std::sync::atomic::Ordering::SeqCst);
            let _second_guard = lock_local_connect_gate().expect("lock second gate holder");
            entered_for_thread.store(true, std::sync::atomic::Ordering::SeqCst);
        });

        while !ready.load(std::sync::atomic::Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(10));
        }
        std::thread::sleep(Duration::from_millis(80));
        assert!(
            !entered.load(std::sync::atomic::Ordering::SeqCst),
            "second caller should block while the shared local-connect gate is held"
        );

        drop(first_guard);
        handle.join().expect("join gate waiter");
        assert!(
            entered.load(std::sync::atomic::Ordering::SeqCst),
            "second caller should proceed once the shared local-connect gate is released"
        );
    }

    #[cfg(unix)]
    fn pid_is_alive(pid: u32) -> bool {
        Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    #[cfg(unix)]
    fn wait_for_pid_exit(pid: u32, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if !pid_is_alive(pid) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(80));
        }
        !pid_is_alive(pid)
    }

    #[test]
    #[cfg(unix)]
    fn desktop_connect_local_spawn_failure_preserves_existing_owned_connection() {
        let state = ConnectionManager::default();
        let child = Command::new("sh")
            .arg("-c")
            .arg("sleep 60")
            .spawn()
            .expect("spawn sleep child");
        let child_pid = child.id();
        assert!(
            pid_is_alive(child_pid),
            "owned child should be alive before replacement attempt"
        );
        state.set_local(
            "http://127.0.0.1:4399".to_string(),
            "existing-token".to_string(),
            child,
            false,
        );

        let resolve_existing_calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let err = connect_local_with_sources(
            &state,
            |_| false,
            || Ok(None),
            |_| Ok(()),
            {
                let resolve_existing_calls = std::sync::Arc::clone(&resolve_existing_calls);
                move || {
                    resolve_existing_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Ok(None)
                }
            },
            || Err(anyhow!("spawn validation failed")),
        )
        .expect_err("spawn failure should preserve the existing owned connection");

        assert!(format!("{err:#}").contains("spawn validation failed"));
        assert_eq!(
            resolve_existing_calls.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "spawn failure path should retry existing-daemon resolution before returning"
        );
        assert!(
            pid_is_alive(child_pid),
            "existing owned daemon child should remain alive after replacement failure"
        );

        let info = state.info();
        assert!(matches!(info.kind, DesktopConnectionKind::Local));
        assert_eq!(info.base_url.as_deref(), Some("http://127.0.0.1:4399"));
        assert_eq!(info.token.as_deref(), Some("existing-token"));

        state.disconnect();
        assert!(
            wait_for_pid_exit(child_pid, Duration::from_secs(3)),
            "owned child should be cleaned up during test teardown"
        );
    }

    #[test]
    #[cfg(unix)]
    fn desktop_connect_local_spawn_race_reattaches_same_owned_local_daemon_without_killing_it() {
        let state = ConnectionManager::default();
        let child = Command::new("sh")
            .arg("-c")
            .arg("sleep 60")
            .spawn()
            .expect("spawn local child placeholder");
        let child_pid = child.id();
        assert!(
            pid_is_alive(child_pid),
            "existing local child should start alive"
        );
        state.set_local(
            "http://127.0.0.1:4301".to_string(),
            "same-token".to_string(),
            child,
            false,
        );

        let resolve_existing_calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let info = connect_local_with_sources(
            &state,
            |_| false,
            || Ok(None),
            |_| Ok(()),
            {
                let resolve_existing_calls = std::sync::Arc::clone(&resolve_existing_calls);
                move || {
                    let call_index =
                        resolve_existing_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    if call_index == 0 {
                        return Ok(None);
                    }
                    Ok(Some((
                        "http://127.0.0.1:4301".to_string(),
                        "same-token".to_string(),
                        Some(child_pid),
                    )))
                }
            },
            || Err(anyhow!("spawn lost race")),
        )
        .expect("same-daemon handoff should preserve the owned local daemon");

        assert_eq!(
            resolve_existing_calls.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "spawn failure path should retry existing-daemon resolution before reattaching"
        );
        assert!(
            pid_is_alive(child_pid),
            "same-daemon handoff must not kill the process being reattached"
        );
        assert!(matches!(info.kind, DesktopConnectionKind::Local));
        assert_eq!(info.base_url.as_deref(), Some("http://127.0.0.1:4301"));
        assert_eq!(info.token.as_deref(), Some("same-token"));

        state.disconnect();
        assert!(
            wait_for_pid_exit(child_pid, Duration::from_secs(3)),
            "reattached owned local daemon should still be terminated on disconnect"
        );
    }

    #[test]
    #[cfg(unix)]
    fn desktop_connect_local_spawn_race_switches_to_validated_local_daemon_over_existing_local_connection(
    ) {
        let state = ConnectionManager::default();
        let child = Command::new("sh")
            .arg("-c")
            .arg("sleep 60")
            .spawn()
            .expect("spawn local child placeholder");
        let child_pid = child.id();
        assert!(
            pid_is_alive(child_pid),
            "existing local child should start alive"
        );
        state.set_local(
            "http://127.0.0.1:4301".to_string(),
            "stale-token".to_string(),
            child,
            false,
        );

        let resolve_existing_calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let info = connect_local_with_sources(
            &state,
            |_| false,
            || Ok(None),
            |_| Ok(()),
            {
                let resolve_existing_calls = std::sync::Arc::clone(&resolve_existing_calls);
                move || {
                    let call_index =
                        resolve_existing_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    if call_index == 0 {
                        return Ok(None);
                    }
                    Ok(Some((
                        "http://127.0.0.1:4399".to_string(),
                        "replacement-token".to_string(),
                        None,
                    )))
                }
            },
            || Err(anyhow!("spawn lost race")),
        )
        .expect("validated local fallback should replace the stale local connection");

        assert_eq!(
            resolve_existing_calls.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "spawn failure path should retry existing-daemon resolution before reattaching"
        );
        assert!(
            wait_for_pid_exit(child_pid, Duration::from_secs(3)),
            "stale local child should be cleaned up once the validated local fallback replaces it"
        );
        assert!(matches!(info.kind, DesktopConnectionKind::Local));
        assert_eq!(info.base_url.as_deref(), Some("http://127.0.0.1:4399"));
        assert_eq!(info.token.as_deref(), Some("replacement-token"));
    }

    #[test]
    #[cfg(unix)]
    fn desktop_connect_local_spawn_race_switches_to_validated_local_daemon_over_ssh_connection() {
        let state = ConnectionManager::default();
        let tunnel = Command::new("sh")
            .arg("-c")
            .arg("sleep 60")
            .spawn()
            .expect("spawn ssh tunnel placeholder");
        let tunnel_pid = tunnel.id();
        assert!(
            pid_is_alive(tunnel_pid),
            "ssh tunnel placeholder should start alive"
        );
        state.set_ssh(
            "http://127.0.0.1:5401".to_string(),
            Some("ssh-token".to_string()),
            tunnel,
            "example.test".to_string(),
            Some("dev".to_string()),
            2222,
            None,
            SshRuntimeMetadata {
                managed_ctx_bin: "~/.ctx/bin/ctx".to_string(),
                active_ctx_bin: Some("~/.ctx/bin/ctx".to_string()),
            },
        );

        let resolve_existing_calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let info = connect_local_with_sources(
            &state,
            |_| false,
            || Ok(None),
            |_| Ok(()),
            {
                let resolve_existing_calls = std::sync::Arc::clone(&resolve_existing_calls);
                move || {
                    let call_index =
                        resolve_existing_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    if call_index == 0 {
                        return Ok(None);
                    }
                    Ok(Some((
                        "http://127.0.0.1:4399".to_string(),
                        "local-token".to_string(),
                        None,
                    )))
                }
            },
            || Err(anyhow!("spawn lost race")),
        )
        .expect("validated local fallback should replace an active non-local connection");

        assert_eq!(
            resolve_existing_calls.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "spawn failure path should retry existing-daemon resolution before attaching"
        );
        assert!(
            wait_for_pid_exit(tunnel_pid, Duration::from_secs(3)),
            "ssh tunnel placeholder should be cleaned up when the validated local daemon replaces it"
        );
        assert!(matches!(info.kind, DesktopConnectionKind::Local));
        assert_eq!(info.base_url.as_deref(), Some("http://127.0.0.1:4399"));
        assert_eq!(info.token.as_deref(), Some("local-token"));
    }

    #[test]
    fn reclaim_predicate_requires_loopback_same_data_dir_and_pid() {
        let expected_dir =
            std::env::temp_dir().join(format!("ctx-daemon-reclaim-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&expected_dir).expect("create expected dir");
        let other_dir = expected_dir.join("other");
        std::fs::create_dir_all(&other_dir).expect("create other dir");

        let compatible_root = DaemonHealthSummary {
            pid: 100,
            data_root: expected_dir.to_string_lossy().to_string(),
            compatibility: DaemonHealthCompatibility::default(),
        };
        assert!(should_reclaim_incompatible_local_daemon(
            "http://127.0.0.1:4123",
            &compatible_root,
            &expected_dir,
        ));

        let non_loopback = DaemonHealthSummary {
            pid: 100,
            data_root: expected_dir.to_string_lossy().to_string(),
            compatibility: DaemonHealthCompatibility::default(),
        };
        assert!(!should_reclaim_incompatible_local_daemon(
            "http://192.168.1.30:4123",
            &non_loopback,
            &expected_dir,
        ));

        let wrong_root = DaemonHealthSummary {
            pid: 100,
            data_root: other_dir.to_string_lossy().to_string(),
            compatibility: DaemonHealthCompatibility::default(),
        };
        assert!(!should_reclaim_incompatible_local_daemon(
            "http://127.0.0.1:4123",
            &wrong_root,
            &expected_dir,
        ));

        let missing_pid = DaemonHealthSummary {
            pid: 0,
            data_root: expected_dir.to_string_lossy().to_string(),
            compatibility: DaemonHealthCompatibility::default(),
        };
        assert!(!should_reclaim_incompatible_local_daemon(
            "http://127.0.0.1:4123",
            &missing_pid,
            &expected_dir,
        ));

        std::fs::remove_dir_all(&expected_dir).ok();
    }

    #[test]
    fn reclaim_complete_requires_pid_exit_and_no_same_pid_health() {
        let pid = 4242u32;
        let same_pid_health = DaemonHealthSummary {
            pid,
            data_root: "/tmp/ctx".to_string(),
            compatibility: DaemonHealthCompatibility::default(),
        };
        let other_pid_health = DaemonHealthSummary {
            pid: pid + 1,
            data_root: "/tmp/ctx".to_string(),
            compatibility: DaemonHealthCompatibility::default(),
        };

        assert!(!reclaim_complete(pid, true, None));
        assert!(!reclaim_complete(pid, true, Some(&same_pid_health)));
        assert!(!reclaim_complete(pid, false, Some(&same_pid_health)));
        assert!(reclaim_complete(pid, false, None));
        assert!(reclaim_complete(pid, false, Some(&other_pid_health)));
    }

    #[test]
    fn health_reports_expected_pid_only_when_health_matches_pid() {
        let pid = 5151u32;
        let matching = DaemonHealthSummary {
            pid,
            data_root: "/tmp/ctx".to_string(),
            compatibility: DaemonHealthCompatibility::default(),
        };
        let other = DaemonHealthSummary {
            pid: pid + 1,
            data_root: "/tmp/ctx".to_string(),
            compatibility: DaemonHealthCompatibility::default(),
        };

        assert!(health_reports_expected_pid(pid, Some(&matching)));
        assert!(!health_reports_expected_pid(pid, Some(&other)));
        assert!(!health_reports_expected_pid(pid, None));
    }

    #[test]
    fn reclaim_health_probe_timeout_respects_remaining_budget() {
        let max_probe = Duration::from_millis(250);
        assert_eq!(
            reclaim_health_probe_timeout(Duration::from_millis(900), max_probe),
            max_probe
        );
        assert_eq!(
            reclaim_health_probe_timeout(Duration::from_millis(40), max_probe),
            Duration::from_millis(40)
        );
        assert_eq!(
            reclaim_health_probe_timeout(Duration::ZERO, max_probe),
            Duration::from_millis(1)
        );
    }

    #[test]
    fn parse_target_requires_os_arch_pair() {
        assert_eq!(
            parse_target("linux/x86_64", "macos", "aarch64"),
            Some(RuntimeTarget {
                os: "linux".to_string(),
                arch: "x86_64".to_string(),
            })
        );
        assert_eq!(parse_target("linux", "macos", "aarch64"), None);
        assert_eq!(parse_target("", "macos", "aarch64"), None);
        assert_eq!(parse_target("linux/", "macos", "aarch64"), None);
    }

    #[test]
    fn parse_target_normalizes_host_tokens() {
        assert_eq!(
            parse_target("host/host", "macos", "aarch64"),
            Some(RuntimeTarget {
                os: "macos".to_string(),
                arch: "aarch64".to_string(),
            })
        );
    }

    #[test]
    fn required_targets_falls_back_when_configured_is_empty_or_invalid() {
        let fallback = vec![RuntimeTarget {
            os: "macos".to_string(),
            arch: "aarch64".to_string(),
        }];
        assert_eq!(
            required_targets_or_default(&[], &fallback, "macos", "aarch64"),
            fallback
        );
        assert_eq!(
            required_targets_or_default(&["invalid".to_string()], &fallback, "macos", "aarch64"),
            fallback
        );
    }

    #[test]
    fn required_targets_normalize_host_and_preserve_concrete_targets() {
        let fallback = vec![
            RuntimeTarget {
                os: "macos".to_string(),
                arch: "aarch64".to_string(),
            },
            RuntimeTarget {
                os: "linux".to_string(),
                arch: "x86_64".to_string(),
            },
        ];
        assert_eq!(
            required_targets_or_default(
                &["host/host".to_string(), "linux/x86_64".to_string()],
                &fallback,
                "macos",
                "aarch64",
            ),
            fallback
        );
    }

    #[test]
    fn host_relevant_targets_filters_to_allowed_subset() {
        let configured = vec![
            RuntimeTarget {
                os: "macos".to_string(),
                arch: "aarch64".to_string(),
            },
            RuntimeTarget {
                os: "linux".to_string(),
                arch: "x86_64".to_string(),
            },
        ];
        let fallback = vec![RuntimeTarget {
            os: "linux".to_string(),
            arch: "x86_64".to_string(),
        }];
        assert_eq!(host_relevant_targets(&configured, &fallback), fallback);
    }

    #[test]
    fn host_relevant_targets_uses_fallback_when_none_match() {
        let configured = vec![RuntimeTarget {
            os: "windows".to_string(),
            arch: "x86_64".to_string(),
        }];
        let fallback = vec![RuntimeTarget {
            os: "linux".to_string(),
            arch: "aarch64".to_string(),
        }];
        assert_eq!(host_relevant_targets(&configured, &fallback), fallback);
    }
}
