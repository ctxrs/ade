use super::*;

const SSH_CONFIG_OVERRIDE_ENV: &str = "CTX_DESKTOP_SSH_CONFIG_PATH";
const DEFAULT_CTX_HARNESS_IMAGE: &str = "ghcr.io/ctxrs/ctx-harness:ubuntu-24.04";

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

#[derive(Debug, Deserialize)]
pub(super) struct DaemonAuthFile {
    pub(super) token: String,
    #[serde(default)]
    pub(super) daemon_url: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct DaemonHealthCompatibility {
    #[serde(default)]
    desktop_exact_version: String,
    #[serde(default)]
    desktop_dev_instance_id: String,
}

#[derive(Debug, Default, Deserialize)]
struct DaemonHealthSummary {
    #[serde(default)]
    pid: u32,
    #[serde(default)]
    data_root: String,
    #[serde(default)]
    compatibility: DaemonHealthCompatibility,
}

#[derive(Debug, Deserialize)]
pub(super) struct DesktopRestartLocalDaemonReq {
    #[serde(default)]
    confirm: bool,
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
pub(super) async fn desktop_connect_local(
    app: tauri::AppHandle,
) -> Result<DesktopConnectionInfo, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ConnectionManager>();
        let data_dir = daemon_data_dir(&app).map_err(to_err)?;
        let desktop_version = app.package_info().version.to_string();
        let desktop_dev_instance_id = desktop_dev_instance_id();
        // Idempotent: if we're already connected to a healthy local daemon, keep the connection.
        // The workspace wizard calls connect_local as part of its flow; disconnecting here can
        // kill a just-started daemon and introduce flakiness on cold start.
        let info = state.info();
        if matches!(info.kind, DesktopConnectionKind::Local) {
            if let Some(url) = info.base_url.as_deref() {
                if existing_local_daemon_matches_or_absent(
                    url,
                    &data_dir,
                    &desktop_version,
                    desktop_dev_instance_id,
                ) {
                    return Ok(info);
                }
            }
        }
        state.disconnect();
        if let Some((url, token)) = resolve_env_local_daemon(&app).map_err(to_err)? {
            probe_daemon_health(&url).map_err(to_err)?;
            state.set_local_external(url, token);
            return Ok(state.info());
        }
        if let Some((url, token)) =
            resolve_existing_local_daemon(&app, &data_dir).map_err(to_err)?
        {
            state.set_local_external(url, token);
            return Ok(state.info());
        }
        // Block until the daemon is actually reachable before returning. The workspace wizard
        // applies the connection and navigates immediately after `desktop_connect_local` resolves;
        // returning early causes the workbench to briefly render a "daemon unavailable" overlay.
        let spawned = match spawn_and_validate_local_daemon(
            &app,
            &data_dir,
            &desktop_version,
            desktop_dev_instance_id,
        ) {
            Ok(value) => value,
            Err(err) => {
                if let Ok(Some((url, token))) = resolve_existing_local_daemon(&app, &data_dir) {
                    state.set_local_external(url, token);
                    return Ok(state.info());
                }
                return Err(to_err(err));
            }
        };
        state.set_local(
            spawned.url,
            spawned.token,
            spawned.child,
            spawned.systemd_scope,
        );
        Ok(state.info())
    })
    .await
    .map_err(|e| format!("failed to connect to daemon: {e}"))?
}

#[tauri::command]
pub(super) async fn desktop_restart_local_daemon(
    app: tauri::AppHandle,
    req: DesktopRestartLocalDaemonReq,
) -> Result<DesktopConnectionInfo, String> {
    if !req.confirm {
        return Err("confirm required".to_string());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ConnectionManager>();
        let manager: &ConnectionManager = state.inner();
        manager.disconnect();
        let data_dir = daemon_data_dir(&app).map_err(to_err)?;
        let desktop_version = app.package_info().version.to_string();
        let desktop_dev_instance_id = desktop_dev_instance_id();
        let spawned = spawn_and_validate_local_daemon(
            &app,
            &data_dir,
            &desktop_version,
            desktop_dev_instance_id,
        )
        .map_err(to_err)?;
        manager.set_local(
            spawned.url,
            spawned.token,
            spawned.child,
            spawned.systemd_scope,
        );
        Ok(manager.info())
    })
    .await
    .map_err(|e| format!("failed to restart local daemon: {e}"))?
}

pub(super) fn ensure_local_connection(
    app: &tauri::AppHandle,
    state: &ConnectionManager,
) -> Result<()> {
    if !matches!(state.info().kind, DesktopConnectionKind::None) {
        return Ok(());
    }
    // Multiple webview requests can race on cold start (overlay pollers, initial data loads, etc.).
    // Serialize the "connect local" path so we don't concurrently spawn the daemon and trip the
    // daemon's lockfile, which can surface as spurious "daemon unavailable" errors in the UI.
    static LOCAL_CONNECT_MUTEX: std::sync::OnceLock<std::sync::Mutex<()>> =
        std::sync::OnceLock::new();
    let mutex = LOCAL_CONNECT_MUTEX.get_or_init(|| std::sync::Mutex::new(()));
    let _guard = mutex
        .lock()
        .map_err(|err| anyhow!("local connect mutex poisoned: {err}"))?;
    if !matches!(state.info().kind, DesktopConnectionKind::None) {
        return Ok(());
    }
    let data_dir = daemon_data_dir(app)?;
    let desktop_version = app.package_info().version.to_string();
    let desktop_dev_instance_id = desktop_dev_instance_id();
    if let Some((url, token)) = resolve_env_local_daemon(app)? {
        probe_daemon_health(&url)?;
        state.set_local_external(url, token);
        return Ok(());
    }
    if let Some((url, token)) = resolve_existing_local_daemon(app, &data_dir)? {
        state.set_local_external(url, token);
        return Ok(());
    }
    let spawned = match spawn_and_validate_local_daemon(
        app,
        &data_dir,
        &desktop_version,
        desktop_dev_instance_id,
    ) {
        Ok(value) => value,
        Err(err) => {
            // This can happen if another thread already started the daemon but we raced before
            // the auth file became visible or health was reachable. Retry by waiting for the auth
            // file + health and then attaching as an external local connection.
            let auth = read_daemon_auth_with_retry(&data_dir)
                .with_context(|| format!("spawning local daemon failed: {err:#}"))?;
            let Some(url) = auth.daemon_url.as_deref() else {
                return Err(err)
                    .context("spawning local daemon failed (auth file missing daemon_url)");
            };
            probe_local_daemon_health_with_retry(url)?;
            let compatible = existing_local_daemon_matches(
                url,
                &data_dir,
                &desktop_version,
                desktop_dev_instance_id,
            )
                .with_context(|| {
                    format!(
                        "spawning local daemon failed: {err:#}; validating existing local daemon compatibility"
                    )
                })?;
            if !compatible {
                return Err(err).context(format!(
                    "spawning local daemon failed and existing daemon is incompatible (url={url})"
                ));
            }
            state.set_local_external(url.to_string(), auth.token);
            return Ok(());
        }
    };
    state.set_local(
        spawned.url,
        spawned.token,
        spawned.child,
        spawned.systemd_scope,
    );
    Ok(())
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
        ensure_local_connection(&app, manager).map_err(to_err)?;
        manager.daemon_request(req).map_err(to_err)
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

fn read_daemon_auth_with_retry(data_dir: &Path) -> Result<DaemonAuthFile> {
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

fn resolve_env_local_daemon(app: &tauri::AppHandle) -> Result<Option<(String, String)>> {
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

fn resolve_existing_local_daemon(
    app: &tauri::AppHandle,
    data_dir: &Path,
) -> Result<Option<(String, String)>> {
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
        return Ok(Some((url.to_string(), auth.token)));
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
    if wait_until_daemon_reclaimed(base_url, pid, Duration::from_secs(3)) {
        return Ok(());
    }
    let force_revalidated = daemon_reports_expected_pid(base_url, pid);
    let force_err = if force_revalidated {
        terminate_pid(pid, true).err()
    } else {
        None
    };
    if wait_until_daemon_reclaimed(base_url, pid, Duration::from_secs(2)) {
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

fn terminate_pid(pid: u32, force: bool) -> Result<()> {
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
    pub daemons: Vec<DesktopBundledDaemon>,
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
struct DesktopBundledDaemon {
    pub id: String,
    pub os: String,
    pub arch: String,
    pub bin: String,
}

#[derive(Debug, Clone, Deserialize)]
struct DesktopBundledImage {
    pub id: String,
    pub os: String,
    pub arch: String,
    pub tar: String,
    pub image: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RuntimeLockRequiredTargets {
    #[serde(default)]
    provider: Vec<String>,
    #[serde(default)]
    runtime: Vec<String>,
    #[serde(default)]
    image: Vec<String>,
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

fn required_image_has_managed_source(
    lock: &RuntimeLockV2,
    image_id: &str,
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
        component.kind == "image"
            && component.id == image_id
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
    let allowed_image_sources = allowed_source_types_for_profile(&lock);

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
            let managed_source_available =
                required_image_has_managed_source(&lock, image_id, target, &allowed_image_sources);
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

    if !failures.is_empty() {
        anyhow::bail!(
            "desktop parity preflight failed (channel={channel} profile=parity surface={surface}): {}",
            failures.join("; ")
        );
    }
    Ok(())
}

fn normalize_arch_token(raw: &str) -> Option<&'static str> {
    match raw.trim() {
        "x86_64" | "amd64" => Some("x86_64"),
        "aarch64" | "arm64" => Some("aarch64"),
        _ => None,
    }
}

pub(super) fn read_bundled_remote_daemon_binary(
    app: &tauri::AppHandle,
    arch: &str,
) -> Result<PathBuf> {
    let bundle_dir = desktop_bundle_dir(app).ok_or_else(|| anyhow!("bundle dir not found"))?;
    let manifest_path = bundle_dir.join("manifest.json");
    let raw = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("reading {}", manifest_path.display()))?;
    let manifest: DesktopBundledAssetsManifest = serde_json::from_str(&raw)
        .with_context(|| format!("parsing {}", manifest_path.display()))?;
    let entry = manifest
        .daemons
        .iter()
        .find(|daemon| daemon.id == "ctx-daemon" && daemon.os == "linux" && daemon.arch == arch)
        .ok_or_else(|| anyhow!("bundled remote daemon binary not found for linux/{arch}"))?;
    let bin = bundle_dir.join(&entry.bin);
    if !bin.exists() {
        anyhow::bail!("bundled remote daemon binary missing at {}", bin.display());
    }
    Ok(bin)
}

fn read_bundled_ctx_harness_image(
    app: &tauri::AppHandle,
    arch: &str,
) -> Result<Option<(PathBuf, String)>> {
    let bundle_dir = desktop_bundle_dir(app).ok_or_else(|| anyhow!("bundle dir not found"))?;
    let manifest_path = bundle_dir.join("manifest.json");
    let raw = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("reading {}", manifest_path.display()))?;
    let manifest: DesktopBundledAssetsManifest = serde_json::from_str(&raw)
        .with_context(|| format!("parsing {}", manifest_path.display()))?;
    let Some(entry) = manifest
        .images
        .iter()
        .find(|img| img.id == "ctx-harness" && img.os == "linux" && img.arch == arch)
    else {
        return Ok(None);
    };
    let tar = bundle_dir.join(&entry.tar);
    if !tar.exists() {
        anyhow::bail!("bundled ctx-harness image tar missing at {}", tar.display());
    }
    Ok(Some((tar, entry.image.clone())))
}

fn ssh_target(host: &str, user: Option<&str>) -> String {
    match user {
        Some(u) if !u.trim().is_empty() => format!("{}@{}", u.trim(), host),
        _ => host.to_string(),
    }
}

fn ssh_output(target: &str, cmd: &str) -> Result<std::process::Output> {
    let remote_cmd = format!("sh -lc {}", shell_escape(cmd));
    let output = new_ssh_command()
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg("ConnectTimeout=8")
        .arg("-o")
        .arg("ConnectionAttempts=1")
        .arg("-o")
        .arg("ServerAliveInterval=5")
        .arg("-o")
        .arg("ServerAliveCountMax=1")
        .arg(target)
        .arg(remote_cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("ssh: {cmd}"))?;
    Ok(output)
}

pub(super) fn ensure_remote_ctx_harness_image(
    app: &tauri::AppHandle,
    host: &str,
    user: Option<&str>,
    remote_data_dir: Option<&str>,
) -> Result<()> {
    let target = ssh_target(host, user);
    let data_dir = remote_data_dir
        .filter(|d| !d.trim().is_empty())
        .unwrap_or("~/.ctx");
    let podman_xdg_root = format!("{}/podman/xdg", data_dir.trim_end_matches('/'));
    let podman_xdg_config = format!("{}/config", podman_xdg_root);
    let podman_xdg_data = format!("{}/data", podman_xdg_root);
    let podman_xdg_run = format!("{}/run", podman_xdg_root);
    let podman_env_prefix = format!(
        "XDG_CONFIG_HOME={} XDG_DATA_HOME={} XDG_RUNTIME_DIR={}",
        remote_path_expr(&podman_xdg_config),
        remote_path_expr(&podman_xdg_data),
        remote_path_expr(&podman_xdg_run),
    );
    let podman_prepare_cmd = format!(
        "mkdir -p {} {} {} && chmod 700 {} >/dev/null 2>&1 || true",
        remote_path_expr(&podman_xdg_config),
        remote_path_expr(&podman_xdg_data),
        remote_path_expr(&podman_xdg_run),
        remote_path_expr(&podman_xdg_run),
    );

    // Only provision the Linux container image on Linux hosts.
    let os_out = ssh_output(&target, "uname -s")?;
    if !os_out.status.success() {
        anyhow::bail!(
            "ssh uname failed: {}",
            String::from_utf8_lossy(&os_out.stderr).trim()
        );
    }
    let os = String::from_utf8_lossy(&os_out.stdout).trim().to_string();
    if os != "Linux" {
        return Ok(());
    }

    let arch_out = ssh_output(&target, "uname -m")?;
    if !arch_out.status.success() {
        anyhow::bail!(
            "ssh uname -m failed: {}",
            String::from_utf8_lossy(&arch_out.stderr).trim()
        );
    }
    let arch_raw = String::from_utf8_lossy(&arch_out.stdout).trim().to_string();
    let Some(arch) = normalize_arch_token(&arch_raw) else {
        anyhow::bail!("unsupported remote architecture: {arch_raw}");
    };

    // If the remote doesn't have podman, don't block ssh connection (container mode just won't work).
    let podman_out = ssh_output(&target, "command -v podman >/dev/null 2>&1")?;
    if !podman_out.status.success() {
        return Ok(());
    }
    let prep_out = ssh_output(&target, &podman_prepare_cmd)?;
    if !prep_out.status.success() {
        anyhow::bail!(
            "ssh podman xdg setup failed: {}",
            String::from_utf8_lossy(&prep_out.stderr).trim()
        );
    }

    let bundled_image = read_bundled_ctx_harness_image(app, arch)?;
    let (image, bundled_tar) = match bundled_image {
        Some((tar, image)) => (image, Some(tar)),
        None => (DEFAULT_CTX_HARNESS_IMAGE.to_string(), None),
    };

    // Check if the image is already present.
    let exists_out = ssh_output(
        &target,
        &format!(
            "{podman_env_prefix} podman image exists -- {}",
            shell_escape(&image)
        ),
    )?;
    if exists_out.status.success() {
        return Ok(());
    }
    if exists_out.status.code() != Some(1) {
        anyhow::bail!(
            "remote podman image exists failed: {}",
            String::from_utf8_lossy(&exists_out.stderr).trim()
        );
    }

    if let Some(tar) = bundled_tar {
        // Stream tar to podman load over SSH.
        let remote_cmd = format!(
            "sh -lc {}",
            shell_escape(&format!(
                "{podman_prepare_cmd} && {podman_env_prefix} podman load"
            ))
        );
        let mut child = new_ssh_command()
            .arg("-o")
            .arg("BatchMode=yes")
            .arg("-o")
            .arg("ConnectTimeout=8")
            .arg("-o")
            .arg("ConnectionAttempts=1")
            .arg("-o")
            .arg("ServerAliveInterval=5")
            .arg("-o")
            .arg("ServerAliveCountMax=1")
            .arg(&target)
            .arg(remote_cmd)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .context("spawning ssh for podman load")?;

        {
            let mut file =
                std::fs::File::open(&tar).with_context(|| format!("opening {}", tar.display()))?;
            let mut stdin = child
                .stdin
                .take()
                .ok_or_else(|| anyhow!("ssh stdin unavailable"))?;
            std::io::copy(&mut file, &mut stdin).context("streaming image tar to ssh")?;
        }

        let output = child
            .wait_with_output()
            .context("waiting for ssh podman load")?;
        if !output.status.success() {
            anyhow::bail!(
                "remote podman load failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
    } else {
        let pull_out = ssh_output(
            &target,
            &format!(
                "{podman_prepare_cmd} && {podman_env_prefix} podman pull -- {}",
                shell_escape(&image)
            ),
        )?;
        if !pull_out.status.success() {
            anyhow::bail!(
                "remote podman pull failed: {}",
                String::from_utf8_lossy(&pull_out.stderr).trim()
            );
        }
    }
    let exists_after = ssh_output(
        &target,
        &format!(
            "{podman_env_prefix} podman image exists -- {}",
            shell_escape(&image)
        ),
    )?;
    if !exists_after.status.success() {
        anyhow::bail!(
            "remote podman load completed but image is still missing for daemon storage: {}",
            image
        );
    }

    Ok(())
}

fn daemon_health(base_url: &str) -> Result<DaemonHealthSummary> {
    daemon_health_with_timeout(base_url, Duration::from_secs(5))
}

fn daemon_health_with_timeout(base_url: &str, timeout: Duration) -> Result<DaemonHealthSummary> {
    let url = format!("{}/api/health", base_url.trim_end_matches('/'));
    let client = reqwest::blocking::Client::builder()
        .timeout(timeout)
        .build()
        .context("building http client")?;
    let res = client.get(url).send().context("requesting /api/health")?;
    let res = res.error_for_status().context("health status")?;
    res.json::<DaemonHealthSummary>()
        .context("parsing /api/health response")
}

fn normalize_path_for_compare(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| normalize_path(path))
}

fn desktop_dev_instance_id() -> &'static str {
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

fn existing_local_daemon_matches(
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

fn existing_local_daemon_matches_or_absent(
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

fn probe_local_daemon_health_with_retry(base_url: &str) -> Result<()> {
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

pub(super) fn daemon_data_dir(app: &tauri::AppHandle) -> Result<PathBuf> {
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
    let root = app
        .path()
        .app_data_dir()
        .context("resolving app_data_dir")?;
    Ok(root.join("daemon"))
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

fn resolve_primary_bin(app: &tauri::AppHandle, name: &str) -> Result<PathBuf> {
    if cfg!(debug_assertions) {
        return dev_bin(name).with_context(|| {
            format!(
                "missing development binary for `{name}` at expected path (build it first, e.g. `cargo build -p ctx-http --bin ctx`)"
            )
        });
    }
    resource_bin(app, name)
        .with_context(|| format!("missing bundled binary for `{name}` in application resources"))
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
    let ctx_bin = resolve_primary_bin(app, "ctx")?;
    let mcp_bin = resolve_optional_bin(app, "ctx-mcp");

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
        for key in DAEMON_ENV_PASSTHROUGH {
            if let Ok(value) = std::env::var(key) {
                cmd.arg("--setenv").arg(format!("{key}={value}"));
            }
        }
        cmd.arg(&ctx_bin);
        cmd
    } else {
        let mut cmd = Command::new(&ctx_bin);
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

struct SpawnedLocalDaemonReady {
    url: String,
    token: String,
    child: Child,
    systemd_scope: bool,
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

fn spawn_and_validate_local_daemon(
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

    #[test]
    fn ssh_config_override_normalization() {
        assert_eq!(
            normalized_ssh_config_override(" /tmp/ctx-fixture-ssh-config "),
            Some("/tmp/ctx-fixture-ssh-config".to_string())
        );
        assert_eq!(normalized_ssh_config_override("   "), None);
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
