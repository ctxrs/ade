use super::*;

#[derive(Debug, Deserialize)]
pub(super) struct SshConnectReq {
    host: String,
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    remote_port: Option<u16>,
    #[serde(default = "default_true")]
    start_remote: bool,
    #[serde(default)]
    remote_data_dir: Option<String>,
    #[serde(default)]
    remote_ctx_bin: Option<String>,
}

const fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize)]
pub(super) struct DesktopSshTestReq {
    host: String,
    #[serde(default)]
    user: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct DesktopRemotePrewarmReq {
    host: String,
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    remote_port: Option<u16>,
    #[serde(default)]
    remote_data_dir: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct DesktopSshPathReq {
    host: String,
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct DesktopSshPathEntry {
    name: String,
    path: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct DesktopGitBranchReq {
    path: String,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct DesktopSshHost {
    host: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    host_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    port: Option<u16>,
}

#[derive(Debug, Deserialize)]
pub(super) struct DesktopRemoteDaemonUpdateReq {
    #[serde(default)]
    confirm: bool,
    #[serde(default)]
    channel: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct DesktopRemoteDaemonUpdateResp {
    updated: bool,
    message: String,
}

#[tauri::command]
pub(super) fn desktop_list_ssh_hosts() -> Result<Vec<DesktopSshHost>, String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for path in ssh_config_paths() {
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(_) => continue,
        };
        for entry in parse_ssh_config(&text) {
            if seen.insert(entry.host.clone()) {
                out.push(entry);
            }
        }
    }
    Ok(out)
}

#[tauri::command]
pub(super) async fn desktop_list_ssh_paths(
    req: DesktopSshPathReq,
) -> Result<Vec<DesktopSshPathEntry>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let host = req.host.trim().to_string();
        if host.is_empty() {
            return Err("host is required".to_string());
        }
        let target = match req.user.as_deref() {
            Some(u) if !u.trim().is_empty() => format!("{}@{}", u.trim(), host),
            _ => host,
        };
        let raw = req.path.unwrap_or_default();
        let (parent, prefix) = split_remote_path(&raw);
        let cmd = format!("ls -a1 -p -- {}", remote_path_expr(&parent));
        let remote_cmd = format!("sh -lc {}", shell_escape(&cmd));
        let output = Command::new("ssh")
            .arg("-o")
            .arg("BatchMode=yes")
            .arg("-o")
            .arg("ConnectTimeout=8")
            .arg(target)
            // NOTE: sshd does not preserve argv boundaries for the remote command; pass as one string.
            .arg(remote_cmd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| format!("failed to spawn ssh: {e}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            if stderr.is_empty() {
                return Err("ssh failed to list paths".to_string());
            }
            return Err(format!("ssh failed: {stderr}"));
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut entries = Vec::new();
        for line in stdout.lines() {
            let name = line.trim();
            if name.is_empty() {
                continue;
            }
            if !name.ends_with('/') {
                continue;
            }
            let name = name.trim_end_matches('/');
            if name == "." {
                continue;
            }
            if !prefix.is_empty() && !name.starts_with(&prefix) {
                continue;
            }
            let full_path = join_remote_path(&parent, name);
            entries.push(DesktopSshPathEntry {
                name: name.to_string(),
                path: full_path,
            });
        }
        Ok(entries)
    })
    .await
    .map_err(|e| format!("ssh list failed: {e}"))?
}

#[tauri::command]
pub(super) async fn desktop_test_ssh(req: DesktopSshTestReq) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let host = req.host.trim().to_string();
        if host.is_empty() {
            return Err("host is required".to_string());
        }
        let target = match req.user.as_deref() {
            Some(u) if !u.trim().is_empty() => format!("{}@{}", u.trim(), host),
            _ => host,
        };
        let output = Command::new("ssh")
            .arg("-o")
            .arg("BatchMode=yes")
            .arg("-o")
            .arg("ConnectTimeout=8")
            .arg(target)
            .arg("true")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| format!("failed to spawn ssh: {e}"))?;
        if output.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            return Err("ssh failed to connect".to_string());
        }
        Err(format!("ssh failed: {stderr}"))
    })
    .await
    .map_err(|e| format!("ssh check failed: {e}"))?
}

#[tauri::command]
pub(super) async fn desktop_get_git_branch(
    req: DesktopGitBranchReq,
) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let raw = req.path.trim();
        if raw.is_empty() {
            return Ok(None);
        }
        let mut path = expand_tilde(raw).unwrap_or_else(|| PathBuf::from(raw));
        if !path.is_absolute() {
            return Ok(None);
        }
        path = normalize_path(&path);
        let output = Command::new("git")
            .arg("-C")
            .arg(&path)
            .arg("rev-parse")
            .arg("--abbrev-ref")
            .arg("HEAD")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output();
        let Ok(output) = output else {
            return Ok(None);
        };
        if !output.status.success() {
            return Ok(None);
        }
        let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if value.is_empty() || value == "HEAD" {
            return Ok(None);
        }
        Ok(Some(value))
    })
    .await
    .map_err(|e| format!("git branch lookup failed: {e}"))?
}

#[tauri::command]
pub(super) async fn desktop_connect_ssh(
    state: tauri::State<'_, ConnectionManager>,
    req: SshConnectReq,
) -> Result<DesktopConnectionInfo, String> {
    state.disconnect();

    let host = req.host.trim().to_string();
    if host.is_empty() {
        return Err("host is required".to_string());
    }
    let remote_port = req.remote_port.unwrap_or(4399);

    let user = req.user.clone();
    let host_for_connect = host.clone();
    let user_for_connect = user.clone();
    let remote_data_dir_for_connect = req.remote_data_dir.clone();
    let remote_ctx_bin = normalize_remote_ctx_bin(req.remote_ctx_bin.as_deref())
        .map(|path| validate_remote_ctx_bin(&path))
        .transpose()
        .map_err(to_err)?;
    let remote_ctx_bin_for_connect = remote_ctx_bin.clone();
    let start_remote = req.start_remote;
    let (base_url, token, tunnel) = tauri::async_runtime::spawn_blocking(move || {
        let no_start_remote = env_bool("CTX_DESKTOP_SSH_NO_START_REMOTE", false);

        // Prefer connecting to an already-running daemon. This avoids restarting/touching
        // the remote daemon when users (or tests) already have it running on the target port.
        let mut local_port = pick_unused_local_port()?;
        let (mut tunnel, tunnel_stderr) =
            start_ssh_tunnel(&host_for_connect, user_for_connect.as_deref(), local_port, remote_port)?;
        let mut base_url = format!("http://127.0.0.1:{local_port}");

        let mut health =
            probe_daemon_health_with_retry(&base_url, local_port, &mut tunnel, &tunnel_stderr);
        if health.is_err() && start_remote && !no_start_remote {
            let _ = try_kill_child(tunnel);
            let ctx_bin = remote_ctx_bin_for_connect.as_deref().ok_or_else(|| {
                anyhow!(
                    "remote daemon start requires `remote_ctx_bin` (absolute path, e.g. /opt/ctx/bin/ctx)"
                )
            })?;
            start_remote_daemon_over_ssh(
                &host_for_connect,
                user_for_connect.as_deref(),
                remote_port,
                remote_data_dir_for_connect.as_deref(),
                ctx_bin,
            )?;

            local_port = pick_unused_local_port()?;
            let (mut tunnel2, tunnel_stderr2) =
                start_ssh_tunnel(&host_for_connect, user_for_connect.as_deref(), local_port, remote_port)?;
            base_url = format!("http://127.0.0.1:{local_port}");
            health = probe_daemon_health_with_retry(
                &base_url,
                local_port,
                &mut tunnel2,
                &tunnel_stderr2,
            );
            if let Err(e) = health {
                let _ = try_kill_child(tunnel2);
                return Err(e);
            }
            tunnel = tunnel2;
        } else if let Err(e) = health {
            let _ = try_kill_child(tunnel);
            return Err(anyhow!(
                "{e:#}; remote start skipped (start_remote={start_remote}, no_start_remote={no_start_remote})"
            ));
        }

        let auth = read_remote_daemon_auth_with_retry(
            &host_for_connect,
            user_for_connect.as_deref(),
            remote_data_dir_for_connect.as_deref(),
        )?;
        Ok((base_url, auth.token, tunnel))
    })
    .await
    .map_err(|e| format!("failed to reach remote daemon: {e}"))?
    .map_err(|e| format!("failed to reach remote daemon: {e:#}"))?;

    state.set_ssh(
        base_url,
        Some(token),
        tunnel,
        host,
        req.user.clone(),
        remote_port,
        req.remote_data_dir.clone(),
        remote_ctx_bin,
    );
    Ok(state.info())
}

#[tauri::command]
pub(super) async fn desktop_update_remote_daemon(
    state: tauri::State<'_, ConnectionManager>,
    req: DesktopRemoteDaemonUpdateReq,
) -> Result<DesktopRemoteDaemonUpdateResp, String> {
    if !req.confirm {
        return Err("confirm required".to_string());
    }
    let channel = normalize_update_channel(req.channel.as_deref())?;
    let target = state.ssh_target().map_err(to_err)?;
    let remote_ctx_bin = target
        .remote_ctx_bin
        .ok_or_else(|| {
            "remote daemon update requires `remote_ctx_bin`; reconnect in launcher/workspace setup with an absolute path".to_string()
        })?;
    let host = target.host;
    let user = target.user;
    let remote_port = target.remote_port;
    let remote_data_dir = target.remote_data_dir;
    let channel_for_update = channel.clone();

    let new_token = tauri::async_runtime::spawn_blocking(move || {
        run_remote_daemon_self_update(
            &host,
            user.as_deref(),
            remote_port,
            remote_data_dir.as_deref(),
            &remote_ctx_bin,
            &channel_for_update,
        )?;
        let auth =
            read_remote_daemon_auth_with_retry(&host, user.as_deref(), remote_data_dir.as_deref())?;
        Ok::<String, anyhow::Error>(auth.token)
    })
    .await
    .map_err(|e| format!("remote daemon update task failed: {e}"))?
    .map_err(to_err)?;

    state.update_ssh_token(new_token).map_err(to_err)?;
    Ok(DesktopRemoteDaemonUpdateResp {
        updated: true,
        message: format!("Remote daemon updated on channel `{channel}` and restarted."),
    })
}

#[tauri::command]
pub(super) async fn desktop_kickoff_remote_prewarm(
    app: tauri::AppHandle,
    req: DesktopRemotePrewarmReq,
) -> Result<(), String> {
    let host = req.host.trim().to_string();
    if host.is_empty() {
        return Err("host is required".to_string());
    }
    let user = normalize_optional_text(req.user.as_deref());
    let remote_port = req.remote_port.unwrap_or(4399);
    let remote_data_dir = normalize_optional_text(req.remote_data_dir.as_deref());
    let key = remote_prewarm_dedupe_key(
        &host,
        user.as_deref(),
        remote_port,
        remote_data_dir.as_deref(),
    );
    let should_spawn = {
        let inflight = remote_prewarm_inflight();
        let mut guard = inflight
            .lock()
            .map_err(|_| "remote prewarm lock poisoned".to_string())?;
        guard.insert(key.clone())
    };
    if !should_spawn {
        return Ok(());
    }

    tauri::async_runtime::spawn(async move {
        let result = tauri::async_runtime::spawn_blocking(move || {
            ensure_remote_ctx_harness_image(
                &app,
                &host,
                user.as_deref(),
                remote_data_dir.as_deref(),
            )
        })
        .await;
        match result {
            Ok(Ok(())) => {}
            Ok(Err(err)) => eprintln!("remote prewarm failed: {err:#}"),
            Err(join_err) => eprintln!("remote prewarm task join failed: {join_err:#}"),
        }
        if let Ok(mut guard) = remote_prewarm_inflight().lock() {
            guard.remove(&key);
        }
    });
    Ok(())
}

fn ssh_config_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(path) = expand_tilde("~/.ssh/config") {
        paths.push(path);
    }
    let system_config = PathBuf::from("/etc/ssh/ssh_config");
    paths.push(system_config.clone());
    let system_dir = PathBuf::from("/etc/ssh/ssh_config.d");
    if let Ok(entries) = std::fs::read_dir(system_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                paths.push(path);
            }
        }
    }
    paths
}

fn parse_ssh_config(text: &str) -> Vec<DesktopSshHost> {
    let mut out = Vec::new();
    let mut current_hosts: Vec<String> = Vec::new();
    let mut current_user: Option<String> = None;
    let mut current_host_name: Option<String> = None;
    let mut current_port: Option<u16> = None;

    let flush = |hosts: &Vec<String>,
                 user: &Option<String>,
                 host_name: &Option<String>,
                 port: &Option<u16>,
                 out: &mut Vec<DesktopSshHost>| {
        if hosts.is_empty() {
            return;
        }
        for host in hosts {
            if is_ssh_pattern(host) {
                continue;
            }
            out.push(DesktopSshHost {
                host: host.to_string(),
                user: user.clone(),
                host_name: host_name.clone(),
                port: *port,
            });
        }
    };

    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let key = parts.next().unwrap_or("");
        let rest: Vec<&str> = parts.collect();
        if key.eq_ignore_ascii_case("host") {
            flush(
                &current_hosts,
                &current_user,
                &current_host_name,
                &current_port,
                &mut out,
            );
            current_hosts = rest.iter().map(|v| v.to_string()).collect();
            current_user = None;
            current_host_name = None;
            current_port = None;
            continue;
        }
        if current_hosts.is_empty() {
            continue;
        }
        if key.eq_ignore_ascii_case("user") {
            current_user = rest.first().map(|v| v.to_string());
        } else if key.eq_ignore_ascii_case("hostname") {
            current_host_name = rest.first().map(|v| v.to_string());
        } else if key.eq_ignore_ascii_case("port") {
            current_port = rest.first().and_then(|v| v.parse::<u16>().ok());
        }
    }

    flush(
        &current_hosts,
        &current_user,
        &current_host_name,
        &current_port,
        &mut out,
    );
    out
}

fn is_ssh_pattern(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.is_empty()
        || trimmed.starts_with('!')
        || trimmed.contains('*')
        || trimmed.contains('?')
        || trimmed.contains('[')
        || trimmed.contains(']')
}

fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
}

fn normalize_remote_ctx_bin(value: Option<&str>) -> Option<String> {
    normalize_optional_text(value)
}

fn validate_remote_ctx_bin(value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        anyhow::bail!("remote_ctx_bin is required");
    }
    if !trimmed.starts_with('/') {
        anyhow::bail!("remote_ctx_bin must be an absolute path (for example /opt/ctx/bin/ctx)");
    }
    Ok(trimmed.to_string())
}

fn remote_prewarm_dedupe_key(
    host: &str,
    user: Option<&str>,
    remote_port: u16,
    remote_data_dir: Option<&str>,
) -> String {
    let host = host.trim().to_ascii_lowercase();
    let user = user.unwrap_or("").trim().to_string();
    let data_dir = remote_data_dir
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("~/.ctx")
        .to_string();
    format!("{user}@{host}:{remote_port}:{data_dir}")
}

fn remote_prewarm_inflight() -> &'static std::sync::Mutex<HashSet<String>> {
    static REMOTE_PREWARM_INFLIGHT: std::sync::OnceLock<std::sync::Mutex<HashSet<String>>> =
        std::sync::OnceLock::new();
    REMOTE_PREWARM_INFLIGHT.get_or_init(|| std::sync::Mutex::new(HashSet::new()))
}

pub(super) fn normalize_update_channel(raw: Option<&str>) -> Result<String, String> {
    let channel = raw
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("stable");
    let valid = channel
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.');
    if !valid {
        return Err("invalid channel (expected [A-Za-z0-9._-])".to_string());
    }
    Ok(channel.to_string())
}

fn run_remote_daemon_self_update(
    host: &str,
    user: Option<&str>,
    remote_port: u16,
    remote_data_dir: Option<&str>,
    remote_ctx_bin: &str,
    channel: &str,
) -> Result<()> {
    let ctx_bin = validate_remote_ctx_bin(remote_ctx_bin)?;
    let update_cmd = format!(
        "if [ -x {ctx_bin} ]; then {ctx_bin} self-update --yes --channel {channel}; else echo 'ctx not executable at configured remote path' >&2; exit 127; fi",
        ctx_bin = remote_path_expr(&ctx_bin),
        channel = shell_escape(channel),
    );
    let output =
        run_remote_ssh_shell(host, user, &update_cmd).context("running remote self-update")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if !stderr.is_empty() { stderr } else { stdout };
        anyhow::bail!("remote self-update failed: {detail}");
    }

    // Always restart to ensure the running daemon process picks up the new binary.
    stop_remote_daemon_over_ssh(host, user, remote_port, &ctx_bin)
        .context("stopping remote daemon after self-update")?;
    start_remote_daemon_over_ssh(host, user, remote_port, remote_data_dir, &ctx_bin)
        .context("starting remote daemon after self-update")?;
    Ok(())
}

fn stop_remote_daemon_over_ssh(
    host: &str,
    user: Option<&str>,
    remote_port: u16,
    remote_ctx_bin: &str,
) -> Result<()> {
    let cmd = remote_stop_daemon_cmd(remote_port, remote_ctx_bin);
    let output = run_remote_ssh_shell(host, user, &cmd).context("stopping remote daemon")?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let detail = if !stderr.is_empty() { stderr } else { stdout };
    anyhow::bail!("remote stop command failed: {detail}");
}

fn remote_stop_daemon_cmd(remote_port: u16, remote_ctx_bin: &str) -> String {
    let ctx_serve_pattern = format!("{remote_ctx_bin} serve");
    let bind_local_pattern = format!("{ctx_serve_pattern} --bind 127.0.0.1:{remote_port}");
    let bind_any_pattern = format!("{ctx_serve_pattern} --bind 0.0.0.0:{remote_port}");
    let port_pattern = format!("{ctx_serve_pattern} --port {remote_port}");
    format!(
        "if command -v lsof >/dev/null 2>&1; then \
  pids=\"$(lsof -tiTCP:{port} -sTCP:LISTEN || true)\"; \
  if [[ -n \"$pids\" ]]; then \
    kill $pids >/dev/null 2>&1 || {{ echo \"remote daemon stop failed (kill on port {port})\" >&2; exit 1; }}; \
    sleep 1; \
    exit 0; \
  fi; \
fi; \
if ! command -v pkill >/dev/null 2>&1; then echo 'pkill unavailable on remote host' >&2; exit 127; fi; \
pkill -f -- {bind_local} >/dev/null 2>&1 || \
pkill -f -- {bind_any} >/dev/null 2>&1 || \
pkill -f -- {port_pattern} >/dev/null 2>&1 || \
pkill -f -- {ctx_serve} >/dev/null 2>&1; \
status=$?; \
if [ $status -ne 0 ]; then echo \"remote daemon stop failed (pkill exit $status)\" >&2; exit $status; fi; \
sleep 1",
        port = remote_port,
        bind_local = shell_escape(&bind_local_pattern),
        bind_any = shell_escape(&bind_any_pattern),
        port_pattern = shell_escape(&port_pattern),
        ctx_serve = shell_escape(&ctx_serve_pattern),
    )
}

fn run_remote_ssh_shell(host: &str, user: Option<&str>, cmd: &str) -> Result<std::process::Output> {
    let target = match user {
        Some(u) if !u.trim().is_empty() => format!("{}@{}", u.trim(), host),
        _ => host.to_string(),
    };
    let remote_cmd = format!("sh -lc {}", shell_escape(cmd));
    Command::new("ssh")
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg("ConnectTimeout=15")
        .arg("-o")
        .arg("ServerAliveInterval=15")
        .arg("-o")
        .arg("ServerAliveCountMax=2")
        .arg(target)
        .arg(remote_cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .context("running ssh remote command")
}

fn start_ssh_tunnel(
    host: &str,
    user: Option<&str>,
    local_port: u16,
    remote_port: u16,
) -> Result<(Child, std::sync::Arc<std::sync::Mutex<String>>)> {
    let target = match user {
        Some(u) if !u.trim().is_empty() => format!("{}@{}", u.trim(), host),
        _ => host.to_string(),
    };

    let mut cmd = Command::new("ssh");
    cmd.arg("-N")
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg("ConnectTimeout=15")
        .arg("-o")
        .arg("ExitOnForwardFailure=yes")
        .arg("-o")
        .arg("ServerAliveInterval=30")
        .arg("-o")
        .arg("ServerAliveCountMax=3")
        .arg("-L")
        .arg(format!("{local_port}:127.0.0.1:{remote_port}"))
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().context("spawning ssh tunnel")?;
    let stderr_log = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    if let Some(stderr) = child.stderr.take() {
        let stderr_log = std::sync::Arc::clone(&stderr_log);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stderr);
            let mut buf = String::new();
            loop {
                buf.clear();
                let read = match reader.read_line(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(_) => break,
                };
                if read == 0 {
                    break;
                }
                let mut log = match stderr_log.lock() {
                    Ok(guard) => guard,
                    Err(_) => return,
                };
                if log.len() + buf.len() > SSH_TUNNEL_LOG_BYTES {
                    let excess = (log.len() + buf.len()) - SSH_TUNNEL_LOG_BYTES;
                    log.drain(..excess);
                }
                log.push_str(&buf);
            }
        });
    }
    Ok((child, stderr_log))
}

fn start_remote_daemon_over_ssh(
    host: &str,
    user: Option<&str>,
    remote_port: u16,
    remote_data_dir: Option<&str>,
    remote_ctx_bin: &str,
) -> Result<()> {
    let target = match user {
        Some(u) if !u.trim().is_empty() => format!("{}@{}", u.trim(), host),
        _ => host.to_string(),
    };

    let data_dir = remote_data_dir
        .filter(|d| !d.trim().is_empty())
        .unwrap_or("~/.ctx");
    let log_dir = format!("{}/logs", data_dir.trim_end_matches('/'));
    let log_dir_expr = remote_path_expr(&log_dir);
    let log_file = format!("{}/daemon.log", log_dir.trim_end_matches('/'));
    let log_file_expr = remote_path_expr(&log_file);
    let ctx_bin = validate_remote_ctx_bin(remote_ctx_bin)?;
    let ctx_bin_expr = remote_path_expr(&ctx_bin);
    // Remote daemon should be able to use host Podman when available (same behavior expected in
    // launcher container modes on Linux remotes). Pass through optional podman tuning envs.
    let mut daemon_env = vec!["CTX_ALLOW_SYSTEM_PODMAN=1".to_string()];
    if let Ok(v) = std::env::var("CTX_PODMAN_PATH") {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            daemon_env.push(format!("CTX_PODMAN_PATH={}", shell_escape(trimmed)));
        }
    }
    if let Ok(v) = std::env::var("CTX_PODMAN_MACHINE_PREFETCH") {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            daemon_env.push(format!(
                "CTX_PODMAN_MACHINE_PREFETCH={}",
                shell_escape(trimmed)
            ));
        }
    }
    let daemon_env_prefix = if daemon_env.is_empty() {
        String::new()
    } else {
        format!("env {} ", daemon_env.join(" "))
    };
    let exec_cmd = format!(
        "if [ -x {ctx_bin} ]; then {env}{ctx_bin} serve --bind 127.0.0.1:{remote_port} --data-dir {dir}; else echo 'ctx not executable at configured remote path' >&2; exit 127; fi",
        env = daemon_env_prefix,
        ctx_bin = ctx_bin_expr,
        dir = remote_path_expr(data_dir),
    );
    let log_cmd = format!(
        "mkdir -p {log_dir} && {exec_cmd} > {log_file} 2>&1",
        log_dir = log_dir_expr,
        log_file = log_file_expr,
    );
    // Always start the remote daemon via nohup so the SSH command returns immediately.
    // systemd-run --scope for a long-lived daemon can keep the SSH session open indefinitely.
    let remote_cmd = format!(
        "mkdir -p {log_dir} && nohup /bin/sh -lc {cmd} >/dev/null 2>&1 < /dev/null &",
        log_dir = log_dir_expr,
        cmd = shell_escape(&log_cmd),
    );

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
        .arg(format!("sh -lc {}", shell_escape(&remote_cmd)))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .context("starting remote daemon over ssh")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!(
            "ssh start failed: {stderr}; remote_cmd={remote_cmd}"
        ));
    }
    Ok(())
}

pub(super) fn shell_escape(s: &str) -> String {
    // Minimal POSIX shell escaping: wrap in single quotes and escape inner single quotes.
    let inner = s.replace('\'', "'\"'\"'");
    format!("'{}'", inner)
}

fn escape_for_double_quotes(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "\\$")
        .replace('`', "\\`")
}

pub(super) fn remote_path_expr(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed == "~" {
        return "\"$HOME\"".to_string();
    }
    if let Some(rest) = trimmed.strip_prefix("~/") {
        let escaped = escape_for_double_quotes(rest);
        if escaped.is_empty() {
            return "\"$HOME\"".to_string();
        }
        return format!("\"$HOME/{}\"", escaped);
    }
    shell_escape(trimmed)
}

fn split_remote_path(raw: &str) -> (String, String) {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return ("~".to_string(), String::new());
    }
    if trimmed == "~" || trimmed.ends_with('/') {
        return (trimmed.to_string(), String::new());
    }
    if let Some((parent, suffix)) = trimmed.rsplit_once('/') {
        if parent.is_empty() {
            return ("/".to_string(), suffix.to_string());
        }
        return (parent.to_string(), suffix.to_string());
    }
    ("~".to_string(), trimmed.to_string())
}

fn join_remote_path(parent: &str, name: &str) -> String {
    if parent == "~" || parent == "~/" {
        return format!("~/{name}");
    }
    if parent == "/" {
        return format!("/{name}");
    }
    format!("{}/{}", parent.trim_end_matches('/'), name)
}

#[cfg(test)]
mod remote_path_validation_tests {
    use super::*;

    #[test]
    fn normalize_and_validate_remote_ctx_bin_values() {
        assert_eq!(normalize_remote_ctx_bin(None), None);
        assert_eq!(normalize_remote_ctx_bin(Some("   ")), None);
        assert_eq!(
            normalize_remote_ctx_bin(Some(" /opt/ctx/bin/ctx ")),
            Some("/opt/ctx/bin/ctx".to_string())
        );

        let valid = validate_remote_ctx_bin("/opt/ctx/bin/ctx").expect("absolute path is valid");
        assert_eq!(valid, "/opt/ctx/bin/ctx");

        let err_empty = validate_remote_ctx_bin(" ").expect_err("empty path must fail");
        assert!(
            err_empty.to_string().contains("remote_ctx_bin is required"),
            "unexpected error: {err_empty:#}"
        );
        let err_rel = validate_remote_ctx_bin("ctx").expect_err("relative path must fail");
        assert!(
            err_rel.to_string().contains("must be an absolute path"),
            "unexpected error: {err_rel:#}"
        );
    }

    #[test]
    fn remote_prewarm_dedupe_key_normalization() {
        let key = remote_prewarm_dedupe_key(
            "EXAMPLE.HOST ",
            Some(" devuser "),
            44199,
            Some(" /tmp/ctx-daemon "),
        );
        assert_eq!(key, "devuser@example.host:44199:/tmp/ctx-daemon");

        let key_default_dir = remote_prewarm_dedupe_key("example.host", None, 44199, None);
        assert_eq!(key_default_dir, "@example.host:44199:~/.ctx");
    }

    #[test]
    fn update_channel_validation() {
        assert_eq!(
            normalize_update_channel(None).expect("default channel should be accepted"),
            "stable"
        );
        assert_eq!(
            normalize_update_channel(Some(" stable ")).expect("trimmed stable should be valid"),
            "stable"
        );
        assert_eq!(
            normalize_update_channel(Some("rc-2026.02.17"))
                .expect("alphanumeric + dot + dash should be valid"),
            "rc-2026.02.17"
        );
        assert!(
            normalize_update_channel(Some("bad channel")).is_err(),
            "spaces should be rejected"
        );
        assert!(
            normalize_update_channel(Some("bad/channel")).is_err(),
            "slashes should be rejected"
        );
    }

    #[test]
    fn remote_stop_command_requires_pkill_success() {
        let cmd = remote_stop_daemon_cmd(44199, "/opt/ctx/bin/ctx");
        assert!(
            cmd.contains("lsof -tiTCP:44199 -sTCP:LISTEN"),
            "expected lsof listener probe in stop command: {cmd}"
        );
        assert!(
            cmd.contains("command -v pkill"),
            "expected pkill preflight in stop command: {cmd}"
        );
        assert!(
            cmd.contains("pkill -f --"),
            "expected pkill invocation in stop command: {cmd}"
        );
        assert!(
            cmd.contains("status=$?"),
            "expected status handling in stop command: {cmd}"
        );
        assert!(
            cmd.contains("exit $status"),
            "expected explicit failure on stop error: {cmd}"
        );
        assert!(
            !cmd.contains("pkill -f -- '/opt/ctx/bin/ctx serve --bind 127.0.0.1:44199' >/dev/null 2>&1 || true"),
            "pkill stop command must not blanket-ignore failures: {cmd}"
        );
        assert!(
            cmd.contains("127.0.0.1:44199"),
            "expected port-specific match pattern in stop command: {cmd}"
        );
        assert!(
            cmd.contains("0.0.0.0:44199"),
            "expected wildcard bind fallback in stop command: {cmd}"
        );
        assert!(
            cmd.contains("/opt/ctx/bin/ctx serve"),
            "expected ctx-serve fallback pattern in stop command: {cmd}"
        );
    }
}
