use super::*;

#[derive(Debug, Deserialize)]
pub(super) struct DaemonAuthFile {
    pub(super) token: String,
    #[serde(default)]
    pub(super) daemon_url: Option<String>,
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
        // Idempotent: if we're already connected to a healthy local daemon, keep the connection.
        // The workspace wizard calls connect_local as part of its flow; disconnecting here can
        // kill a just-started daemon and introduce flakiness on cold start.
        let info = state.info();
        if matches!(info.kind, DesktopConnectionKind::Local) {
            if let Some(url) = info.base_url.as_deref() {
                if probe_daemon_health(url).is_ok() {
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
        let data_dir = daemon_data_dir(&app).map_err(to_err)?;
        if let Some((url, token)) = resolve_existing_local_daemon(&data_dir).map_err(to_err)? {
            state.set_local_external(url, token);
            return Ok(state.info());
        }
        // Block until the daemon is actually reachable before returning. The workspace wizard
        // applies the connection and navigates immediately after `desktop_connect_local` resolves;
        // returning early causes the workbench to briefly render a "daemon unavailable" overlay.
        let (url, child, systemd_scope) = match spawn_daemon(&app, &data_dir, true) {
            Ok(value) => value,
            Err(err) => {
                if let Ok(Some((url, token))) = resolve_existing_local_daemon(&data_dir) {
                    state.set_local_external(url, token);
                    return Ok(state.info());
                }
                return Err(to_err(err));
            }
        };
        let auth = read_daemon_auth_with_retry(&data_dir).map_err(to_err)?;
        state.set_local(url.clone(), auth.token.clone(), child, systemd_scope);
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
        let (url, child, systemd_scope) = spawn_daemon(&app, &data_dir, true).map_err(to_err)?;
        let auth = read_daemon_auth_with_retry(&data_dir).map_err(to_err)?;
        manager.set_local(url, auth.token, child, systemd_scope);
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
    if let Some((url, token)) = resolve_env_local_daemon(app)? {
        probe_daemon_health(&url)?;
        state.set_local_external(url, token);
        return Ok(());
    }
    if let Some((url, token)) = resolve_existing_local_daemon(&data_dir)? {
        state.set_local_external(url, token);
        return Ok(());
    }
    let (url, child, systemd_scope) = match spawn_daemon(app, &data_dir, true) {
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
            state.set_local_external(url.to_string(), auth.token);
            return Ok(());
        }
    };
    let auth = read_daemon_auth_with_retry(&data_dir)?;
    state.set_local(url, auth.token, child, systemd_scope);
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

fn resolve_existing_local_daemon(data_dir: &Path) -> Result<Option<(String, String)>> {
    let Some(auth) = read_daemon_auth_if_present(data_dir)? else {
        return Ok(None);
    };
    let Some(url) = auth.daemon_url.as_deref() else {
        return Ok(None);
    };
    match probe_daemon_health(url) {
        Ok(()) => Ok(Some((url.to_string(), auth.token))),
        Err(_) => Ok(None),
    }
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

    let output = Command::new("ssh")
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
    pub images: Vec<DesktopBundledImage>,
}

#[derive(Debug, Clone, Deserialize)]
struct DesktopBundledImage {
    pub id: String,
    pub os: String,
    pub arch: String,
    pub tar: String,
    pub image: String,
}

fn normalize_arch_token(raw: &str) -> Option<&'static str> {
    match raw.trim() {
        "x86_64" | "amd64" => Some("x86_64"),
        "aarch64" | "arm64" => Some("aarch64"),
        _ => None,
    }
}

fn read_bundled_ctx_harness_image(app: &tauri::AppHandle, arch: &str) -> Result<(PathBuf, String)> {
    let bundle_dir = desktop_bundle_dir(app).ok_or_else(|| anyhow!("bundle dir not found"))?;
    let manifest_path = bundle_dir.join("manifest.json");
    let raw = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("reading {}", manifest_path.display()))?;
    let manifest: DesktopBundledAssetsManifest = serde_json::from_str(&raw)
        .with_context(|| format!("parsing {}", manifest_path.display()))?;
    let entry = manifest
        .images
        .iter()
        .find(|img| img.id == "ctx-harness" && img.os == "linux" && img.arch == arch)
        .ok_or_else(|| anyhow!("bundled ctx-harness image tar not found for linux/{arch}"))?;
    let tar = bundle_dir.join(&entry.tar);
    if !tar.exists() {
        anyhow::bail!("bundled ctx-harness image tar missing at {}", tar.display());
    }
    Ok((tar, entry.image.clone()))
}

fn ssh_target(host: &str, user: Option<&str>) -> String {
    match user {
        Some(u) if !u.trim().is_empty() => format!("{}@{}", u.trim(), host),
        _ => host.to_string(),
    }
}

fn ssh_output(target: &str, cmd: &str) -> Result<std::process::Output> {
    let remote_cmd = format!("sh -lc {}", shell_escape(cmd));
    let output = Command::new("ssh")
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

    let (tar, image) = read_bundled_ctx_harness_image(app, arch)?;

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

    // Stream tar to podman load over SSH.
    let remote_cmd = format!(
        "sh -lc {}",
        shell_escape(&format!(
            "{podman_prepare_cmd} && {podman_env_prefix} podman load"
        ))
    );
    let mut child = Command::new("ssh")
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

pub(super) fn probe_daemon_health(base_url: &str) -> Result<()> {
    let url = format!("{}/api/health", base_url.trim_end_matches('/'));
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .context("building http client")?;
    let res = client.get(url).send().context("requesting /api/health")?;
    res.error_for_status().context("health status")?;
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

fn manifest_bin(name: &str) -> Option<PathBuf> {
    let bin_ext = if cfg!(target_os = "windows") {
        ".exe"
    } else {
        ""
    };
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("bin");
    for candidate in [
        base.join(format!(
            "{name}-{arch}{bin_ext}",
            arch = current_arch_token()
        )),
        base.join(format!("{name}{bin_ext}")),
    ] {
        if candidate.exists() && path_matches_current_platform_binary(&candidate) {
            return Some(candidate);
        }
    }
    None
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
    let ctx_bin = resource_bin(app, "ctx")
        .or_else(|| manifest_bin("ctx"))
        .or_else(|| dev_bin("ctx"))
        .unwrap_or_else(|| PathBuf::from("ctx"));

    let mcp_bin = resource_bin(app, "ctx-mcp")
        .or_else(|| manifest_bin("ctx-mcp"))
        .or_else(|| dev_bin("ctx-mcp"));

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
