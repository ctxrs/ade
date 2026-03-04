use super::*;
use sha2::Digest;

#[derive(Debug, Deserialize)]
pub(super) struct SshConnectReq {
    host: String,
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    password_once: Option<String>,
    #[serde(default)]
    remote_port: Option<u16>,
    #[serde(default = "default_true")]
    start_remote: bool,
    #[serde(default)]
    remote_data_dir: Option<String>,
}

const fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize)]
pub(super) struct DesktopSshTestReq {
    host: String,
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    password_once: Option<String>,
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

const MANAGED_REMOTE_CTX_BIN: &str = "~/.ctx/bin/ctx";
const WINDOWS_REMOTE_UNSUPPORTED_MSG: &str =
    "Remote Windows hosts are not supported yet. Use a Linux host (x86_64 or arm64).";
const REMOTE_BOOTSTRAP_CAPABILITY_MSG: &str =
    "Remote daemon bootstrap failed while retrieving managed daemon artifact. Check network connectivity and release metadata.";
const PLATFORM_PROBE_OS_MARKER: &str = "__CTX_PLATFORM_OS__";
const PLATFORM_PROBE_ARCH_MARKER: &str = "__CTX_PLATFORM_ARCH__";
const SSH_CONFIG_OVERRIDE_ENV: &str = "CTX_DESKTOP_SSH_CONFIG_PATH";
const SSH_TUNNEL_BOOTSTRAP_HEALTH_RETRIES: usize = 12;
const SSH_TUNNEL_BOOTSTRAP_HEALTH_BASE_DELAY_MS: u64 = 150;
const DEFAULT_DOWNLOAD_BASE_URL: &str = "https://api.ctx.rs/functions/v1";
const REMOTE_DAEMON_DOWNLOAD_TIMEOUT_SECS: u64 = 180;

#[derive(Debug, Clone, Copy)]
struct RemoteLinuxPlatform {
    arch: &'static str,
}

#[derive(Debug, Clone, Deserialize)]
struct ReleaseDaemonArtifact {
    url_path: String,
    sha256: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ReleasePlatformEntry {
    #[serde(default)]
    daemon: Option<ReleaseDaemonArtifact>,
}

#[derive(Debug, Clone, Deserialize)]
struct ReleaseManifest {
    #[serde(default)]
    platforms: std::collections::HashMap<String, ReleasePlatformEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RemoteCtxBootstrapPlan {
    UseManaged,
    InstallManaged,
}

fn plan_remote_ctx_bootstrap(managed_exists: bool) -> RemoteCtxBootstrapPlan {
    if managed_exists {
        return RemoteCtxBootstrapPlan::UseManaged;
    }
    RemoteCtxBootstrapPlan::InstallManaged
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
        let output = new_ssh_command()
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
        let user = normalize_optional_text(req.user.as_deref());
        let password_once = normalize_optional_text(req.password_once.as_deref());
        probe_remote_linux_platform_with_optional_password(
            &host,
            user.as_deref(),
            password_once.as_deref(),
        )
        .map(|_| ())
        .map_err(to_err)
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
    app: tauri::AppHandle,
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
    let password_once = normalize_optional_text(req.password_once.as_deref());
    let host_for_connect = host.clone();
    let user_for_connect = user.clone();
    let password_once_for_connect = password_once.clone();
    let remote_data_dir_for_connect = req.remote_data_dir.clone();
    let app_for_connect = app.clone();
    let remote_ctx_bin = MANAGED_REMOTE_CTX_BIN.to_string();
    let remote_ctx_bin_for_connect = remote_ctx_bin.clone();
    let start_remote = req.start_remote;
    let (base_url, token, tunnel, effective_remote_ctx_bin) =
        tauri::async_runtime::spawn_blocking(move || {
        let remote_platform = probe_remote_linux_platform_with_optional_password(
            &host_for_connect,
            user_for_connect.as_deref(),
            password_once_for_connect.as_deref(),
        )?;
        let no_start_remote = env_bool("CTX_DESKTOP_SSH_NO_START_REMOTE", false);
        let mut effective_remote_ctx_bin: Option<String> = None;

        // Prefer connecting to an already-running daemon. This avoids restarting/touching
        // the remote daemon when users (or tests) already have it running on the target port.
        let mut local_port = pick_unused_local_port()?;
        let (mut tunnel, tunnel_stderr) =
            start_ssh_tunnel(&host_for_connect, user_for_connect.as_deref(), local_port, remote_port)?;
        let mut base_url = format!("http://127.0.0.1:{local_port}");

        let mut health = if start_remote && !no_start_remote {
            probe_daemon_health_quick_for_bootstrap(&base_url, &mut tunnel, &tunnel_stderr)
        } else {
            probe_daemon_health_with_retry(&base_url, local_port, &mut tunnel, &tunnel_stderr)
        };
        if health.is_err() && start_remote && !no_start_remote {
            let _ = try_kill_child(tunnel);
            // Bootstrap contract:
            // 1) use managed binary when present,
            // 2) otherwise download/install managed binary for this release channel.
            let managed_exists = remote_ctx_bin_exists_over_ssh(
                &host_for_connect,
                user_for_connect.as_deref(),
                &remote_ctx_bin_for_connect,
            )?;
            let remote_start_ctx_bin = match plan_remote_ctx_bootstrap(managed_exists) {
                RemoteCtxBootstrapPlan::UseManaged => remote_ctx_bin_for_connect.clone(),
                RemoteCtxBootstrapPlan::InstallManaged => {
                    install_remote_daemon_over_ssh(
                        &app_for_connect,
                        &host_for_connect,
                        user_for_connect.as_deref(),
                        remote_platform,
                        &remote_ctx_bin_for_connect,
                    )
                    .map_err(|install_err| install_err.context(REMOTE_BOOTSTRAP_CAPABILITY_MSG))?;
                    remote_ctx_bin_for_connect.clone()
                }
            };
            start_remote_daemon_over_ssh(
                &host_for_connect,
                user_for_connect.as_deref(),
                remote_port,
                remote_data_dir_for_connect.as_deref(),
                &remote_start_ctx_bin,
            )?;
            effective_remote_ctx_bin = Some(remote_start_ctx_bin);

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

        if effective_remote_ctx_bin.is_none() {
            if remote_ctx_bin_exists_over_ssh(
                &host_for_connect,
                user_for_connect.as_deref(),
                &remote_ctx_bin_for_connect,
            )
            .unwrap_or(false)
            {
                effective_remote_ctx_bin = Some(remote_ctx_bin_for_connect);
            }
        }

        let auth = read_remote_daemon_auth_with_retry(
            &host_for_connect,
            user_for_connect.as_deref(),
            remote_data_dir_for_connect.as_deref(),
        )?;
        Ok((base_url, auth.token, tunnel, effective_remote_ctx_bin))
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
        effective_remote_ctx_bin,
    );
    Ok(state.info())
}

#[tauri::command]
pub(super) async fn desktop_update_remote_daemon(
    app: tauri::AppHandle,
    state: tauri::State<'_, ConnectionManager>,
    req: DesktopRemoteDaemonUpdateReq,
) -> Result<DesktopRemoteDaemonUpdateResp, String> {
    if !req.confirm {
        return Err("confirm required".to_string());
    }
    let channel = normalize_update_channel(req.channel.as_deref())?;
    let target = state.ssh_target().map_err(to_err)?;
    let managed_remote_ctx_bin = MANAGED_REMOTE_CTX_BIN.to_string();
    let mut remote_ctx_bin = target
        .remote_ctx_bin
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| managed_remote_ctx_bin.clone());
    let host = target.host;
    let user = target.user;
    let remote_port = target.remote_port;
    let remote_data_dir = target.remote_data_dir;
    let channel_for_update = channel.clone();
    let app_for_update = app.clone();

    let new_token = tauri::async_runtime::spawn_blocking(move || {
        if !remote_ctx_bin_exists_over_ssh(&host, user.as_deref(), &remote_ctx_bin)? {
            if remote_ctx_bin != managed_remote_ctx_bin
                && remote_ctx_bin_exists_over_ssh(&host, user.as_deref(), &managed_remote_ctx_bin)?
            {
                remote_ctx_bin = managed_remote_ctx_bin.clone();
            } else {
                let remote_platform = probe_remote_linux_platform(&host, user.as_deref())?;
                install_remote_daemon_over_ssh(
                    &app_for_update,
                    &host,
                    user.as_deref(),
                    remote_platform,
                    &managed_remote_ctx_bin,
                )
                .map_err(|install_err| install_err.context(REMOTE_BOOTSTRAP_CAPABILITY_MSG))?;
                remote_ctx_bin = managed_remote_ctx_bin.clone();
            }
        }
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

fn normalized_ssh_config_override(value: Option<&str>) -> Option<String> {
    normalize_optional_text(value)
}

fn ssh_config_override_path() -> Option<String> {
    std::env::var(SSH_CONFIG_OVERRIDE_ENV)
        .ok()
        .as_deref()
        .and_then(|raw| normalized_ssh_config_override(Some(raw)))
}

fn new_ssh_command() -> Command {
    let mut cmd = Command::new("ssh");
    if let Some(path) = ssh_config_override_path() {
        cmd.arg("-F").arg(path);
    }
    cmd
}

fn looks_like_ssh_auth_failure(message: &str) -> bool {
    let lowered = message.to_ascii_lowercase();
    lowered.contains("permission denied")
        || lowered.contains("publickey")
        || lowered.contains("authentication failed")
        || lowered.contains("too many authentication failures")
}

fn probe_remote_linux_platform_with_optional_password(
    host: &str,
    user: Option<&str>,
    password_once: Option<&str>,
) -> Result<RemoteLinuxPlatform> {
    match probe_remote_linux_platform(host, user) {
        Ok(platform) => Ok(platform),
        Err(err) => {
            let Some(password_once) = password_once else {
                return Err(err);
            };
            if !looks_like_ssh_auth_failure(&err.to_string()) {
                return Err(err);
            }
            bootstrap_ssh_key_auth_with_password(host, user, password_once)?;
            probe_remote_linux_platform(host, user)
        }
    }
}

fn bootstrap_ssh_key_auth_with_password(
    host: &str,
    user: Option<&str>,
    password_once: &str,
) -> Result<()> {
    let public_key = ensure_default_ssh_public_key()?;
    // Stream the key through stdin to avoid brittle shell quoting for key payloads.
    let install_cmd = ssh_authorized_keys_install_command();
    let key_payload = format!("{public_key}\n");
    let output = run_ssh_shell_with_password_once(
        host,
        user,
        password_once,
        install_cmd,
        Some(key_payload.as_bytes()),
    )
    .context("running password-once SSH bootstrap")?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let detail = if !stderr.is_empty() { stderr } else { stdout };
    if detail.is_empty() {
        anyhow::bail!("password-once SSH bootstrap failed");
    }
    anyhow::bail!("password-once SSH bootstrap failed: {detail}");
}

fn ssh_authorized_keys_install_command() -> &'static str {
    "umask 077; \
mkdir -p \"$HOME/.ssh\"; \
chmod 700 \"$HOME/.ssh\"; \
touch \"$HOME/.ssh/authorized_keys\"; \
chmod 600 \"$HOME/.ssh/authorized_keys\"; \
key=\"$(cat)\"; \
grep -qxF \"$key\" \"$HOME/.ssh/authorized_keys\" || printf '%s\\n' \"$key\" >> \"$HOME/.ssh/authorized_keys\""
}

fn ensure_default_ssh_public_key() -> Result<String> {
    let ssh_dir = expand_tilde("~/.ssh")
        .ok_or_else(|| anyhow!("unable to resolve ~/.ssh for SSH password bootstrap"))?;
    std::fs::create_dir_all(&ssh_dir).context("creating local ~/.ssh directory")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&ssh_dir, std::fs::Permissions::from_mode(0o700))
            .context("setting local ~/.ssh permissions")?;
    }

    let private_key_path = ssh_dir.join("id_ed25519");
    let public_key_path = ssh_dir.join("id_ed25519.pub");
    if !public_key_path.exists() {
        if private_key_path.exists() {
            let derive_output = Command::new("ssh-keygen")
                .arg("-y")
                .arg("-f")
                .arg(&private_key_path)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .context("deriving public key from existing ~/.ssh/id_ed25519")?;
            if !derive_output.status.success() {
                let stderr = String::from_utf8_lossy(&derive_output.stderr)
                    .trim()
                    .to_string();
                if stderr.is_empty() {
                    anyhow::bail!("unable to derive ~/.ssh/id_ed25519.pub");
                }
                anyhow::bail!("unable to derive ~/.ssh/id_ed25519.pub: {stderr}");
            }
            let derived = String::from_utf8_lossy(&derive_output.stdout)
                .trim()
                .to_string();
            if derived.is_empty() {
                anyhow::bail!("derived ~/.ssh/id_ed25519.pub is empty");
            }
            std::fs::write(&public_key_path, format!("{derived}\n"))
                .context("writing ~/.ssh/id_ed25519.pub")?;
        } else {
            let generate_output = Command::new("ssh-keygen")
                .arg("-t")
                .arg("ed25519")
                .arg("-N")
                .arg("")
                .arg("-f")
                .arg(&private_key_path)
                .arg("-C")
                .arg("ctx-desktop")
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .context("generating ~/.ssh/id_ed25519 for SSH password bootstrap")?;
            if !generate_output.status.success() {
                let stderr = String::from_utf8_lossy(&generate_output.stderr)
                    .trim()
                    .to_string();
                if stderr.is_empty() {
                    anyhow::bail!("unable to generate ~/.ssh/id_ed25519");
                }
                anyhow::bail!("unable to generate ~/.ssh/id_ed25519: {stderr}");
            }
        }
    }

    let public_key = std::fs::read_to_string(&public_key_path)
        .with_context(|| format!("reading {}", public_key_path.display()))?;
    let trimmed = public_key.trim();
    if trimmed.is_empty() {
        anyhow::bail!(
            "local SSH public key is empty at {}",
            public_key_path.display()
        );
    }
    Ok(trimmed.to_string())
}

fn write_ssh_askpass_script() -> Result<PathBuf> {
    let path = std::env::temp_dir().join(format!("ctx-ssh-askpass-{}.sh", uuid::Uuid::new_v4()));
    std::fs::write(
        &path,
        "#!/bin/sh\nprintf '%s\\n' \"$CTX_SSH_PASSWORD_ONCE\"\n",
    )
    .with_context(|| format!("writing SSH askpass helper at {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
            .with_context(|| format!("setting SSH askpass helper mode at {}", path.display()))?;
    }
    Ok(path)
}

fn run_ssh_shell_with_password_once(
    host: &str,
    user: Option<&str>,
    password_once: &str,
    cmd: &str,
    stdin_payload: Option<&[u8]>,
) -> Result<std::process::Output> {
    let target = match user {
        Some(u) if !u.trim().is_empty() => format!("{}@{}", u.trim(), host),
        _ => host.to_string(),
    };
    let remote_cmd = format!("sh -lc {}", shell_escape(cmd));
    let askpass_script = write_ssh_askpass_script()?;
    let mut command = new_ssh_command();
    command
        .arg("-o")
        .arg("BatchMode=no")
        .arg("-o")
        .arg("ConnectTimeout=15")
        .arg("-o")
        .arg("ConnectionAttempts=1")
        .arg("-o")
        .arg("NumberOfPasswordPrompts=1")
        .arg("-o")
        .arg("PreferredAuthentications=password,keyboard-interactive")
        .arg("-o")
        .arg("PasswordAuthentication=yes")
        .arg("-o")
        .arg("KbdInteractiveAuthentication=yes")
        .arg("-o")
        .arg("PubkeyAuthentication=no")
        .arg("-o")
        .arg("StrictHostKeyChecking=accept-new")
        .arg(target)
        .arg(remote_cmd)
        .env("SSH_ASKPASS", &askpass_script)
        .env("SSH_ASKPASS_REQUIRE", "force")
        .env("DISPLAY", "ctx-desktop")
        .env("CTX_SSH_PASSWORD_ONCE", password_once)
        .stdin(if stdin_payload.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let output = (|| -> Result<std::process::Output> {
        let mut child = command
            .spawn()
            .context("running ssh with password-once credentials")?;
        if let Some(payload) = stdin_payload {
            let Some(mut stdin) = child.stdin.take() else {
                return Err(reap_password_once_child_after_input_error(
                    child,
                    "ssh stdin unavailable for password-once command",
                ));
            };
            if let Err(err) = stdin.write_all(payload) {
                drop(stdin);
                return Err(reap_password_once_child_after_input_error(
                    child,
                    &format!("writing password-once SSH stdin payload: {err}"),
                ));
            }
        }
        child
            .wait_with_output()
            .context("waiting for password-once SSH command")
    })();
    let _ = std::fs::remove_file(&askpass_script);
    output
}

fn format_password_once_output_detail(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stderr.is_empty() {
        return stderr;
    }
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn reap_password_once_child_after_input_error(child: Child, reason: &str) -> anyhow::Error {
    match child.wait_with_output() {
        Ok(output) => {
            let detail = format_password_once_output_detail(&output);
            if detail.is_empty() {
                anyhow!(
                    "{reason}; password-once ssh exited with status {}",
                    output.status
                )
            } else {
                anyhow!(
                    "{reason}; password-once ssh exited with status {}: {detail}",
                    output.status
                )
            }
        }
        Err(wait_err) => anyhow!(
            "{reason}; additionally failed waiting for password-once SSH command: {wait_err}"
        ),
    }
}

fn ssh_stderr_snippet(stderr_log: &std::sync::Arc<std::sync::Mutex<String>>) -> String {
    let guard = match stderr_log.lock() {
        Ok(guard) => guard,
        Err(_) => return String::new(),
    };
    let trimmed = guard.trim();
    if trimmed.is_empty() {
        String::new()
    } else {
        trimmed.to_string()
    }
}

fn probe_daemon_health_quick_for_bootstrap(
    base_url: &str,
    tunnel: &mut Child,
    tunnel_stderr: &std::sync::Arc<std::sync::Mutex<String>>,
) -> Result<()> {
    let mut last_err: Option<anyhow::Error> = None;
    // Fast-path when remote bootstrap is enabled: if daemon isn't already up,
    // fail quickly and proceed to start_remote logic.
    //
    // Note: we still give tunnel bring-up a short retry budget. Some environments
    // need >200ms before /api/health is reachable over a newly-started SSH tunnel.
    for attempt in 0..SSH_TUNNEL_BOOTSTRAP_HEALTH_RETRIES {
        match probe_daemon_health(base_url) {
            Ok(()) => return Ok(()),
            Err(err) => last_err = Some(err),
        }
        if let Ok(Some(status)) = tunnel.try_wait() {
            let stderr = ssh_stderr_snippet(tunnel_stderr);
            if stderr.is_empty() {
                return Err(anyhow!("ssh tunnel exited ({status})"));
            }
            return Err(anyhow!("ssh tunnel exited ({status}): {stderr}"));
        }
        if attempt + 1 < SSH_TUNNEL_BOOTSTRAP_HEALTH_RETRIES {
            let delay =
                SSH_TUNNEL_BOOTSTRAP_HEALTH_BASE_DELAY_MS.saturating_mul((attempt + 1) as u64);
            std::thread::sleep(Duration::from_millis(delay));
        }
    }
    let err = last_err.unwrap_or_else(|| anyhow!("requesting /api/health failed"));
    let stderr = ssh_stderr_snippet(tunnel_stderr);
    let tunnel_state = match tunnel.try_wait() {
        Ok(Some(status)) => format!("ssh tunnel exited ({status})"),
        Ok(None) => "ssh tunnel still running".to_string(),
        Err(e) => format!("ssh tunnel state unknown ({e})"),
    };
    let mut details = format!("{tunnel_state}; quick bootstrap health probe exhausted");
    if !stderr.is_empty() {
        details.push_str(&format!("; ssh stderr: {stderr}"));
    }
    Err(anyhow!("{err:#}; {details}"))
}

fn validate_remote_ctx_bin(value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        anyhow::bail!("remote_ctx_bin is required");
    }
    let valid_absolute = trimmed.starts_with('/');
    let valid_home_relative = trimmed == "~" || trimmed.starts_with("~/");
    if !valid_absolute && !valid_home_relative {
        anyhow::bail!(
            "remote_ctx_bin must be an absolute path or ~/ path (for example ~/.ctx/bin/ctx)"
        );
    }
    Ok(trimmed.to_string())
}

fn normalize_remote_arch_token(raw: &str) -> Option<&'static str> {
    match raw.trim() {
        "x86_64" | "amd64" => Some("x86_64"),
        "aarch64" | "arm64" => Some("aarch64"),
        _ => None,
    }
}

fn is_windows_os_token(raw: &str) -> bool {
    let lowered = raw.trim().to_ascii_lowercase();
    lowered.contains("windows")
        || lowered.contains("mingw")
        || lowered.contains("msys")
        || lowered.contains("cygwin")
}

fn looks_like_windows_shell_error(raw: &str) -> bool {
    let lowered = raw.to_ascii_lowercase();
    lowered.contains("is not recognized as an internal or external command")
        || lowered.contains("'sh' is not recognized")
        || lowered.contains("'uname' is not recognized")
        || lowered.contains("cmd.exe")
        || lowered.contains("powershell")
}

fn probe_remote_linux_platform(host: &str, user: Option<&str>) -> Result<RemoteLinuxPlatform> {
    let target = match user {
        Some(u) if !u.trim().is_empty() => format!("{}@{}", u.trim(), host),
        _ => host.to_string(),
    };

    let probe_cmd = format!(
        "printf '{}%s\\n' \"$(uname -s 2>/dev/null || true)\"; printf '{}%s\\n' \"$(uname -m 2>/dev/null || true)\"",
        PLATFORM_PROBE_OS_MARKER, PLATFORM_PROBE_ARCH_MARKER,
    );
    let remote_probe_cmd = format!("sh -lc {}", shell_escape(&probe_cmd));
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
        .arg(&target)
        // NOTE: sshd does not preserve argv boundaries for the remote command; pass as one string.
        .arg(remote_probe_cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .context("probing remote platform over ssh")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if looks_like_windows_shell_error(&stderr) || detect_windows_remote_with_cmd(&target) {
            anyhow::bail!(WINDOWS_REMOTE_UNSUPPORTED_MSG);
        }
        if stderr.is_empty() {
            anyhow::bail!("ssh failed to probe remote platform");
        }
        anyhow::bail!("ssh failed to probe remote platform: {stderr}");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let Some((os, arch_raw)) = parse_remote_platform_probe_stdout(&stdout) else {
        anyhow::bail!("ssh probe returned incomplete platform details");
    };
    if is_windows_os_token(&os) {
        anyhow::bail!(WINDOWS_REMOTE_UNSUPPORTED_MSG);
    }
    if os != "Linux" {
        anyhow::bail!("Unsupported remote OS `{os}`. Use a Linux host (x86_64 or arm64).");
    }
    let Some(arch) = normalize_remote_arch_token(&arch_raw) else {
        anyhow::bail!("Unsupported remote architecture `{arch_raw}`. Use Linux x86_64 or arm64.");
    };
    Ok(RemoteLinuxPlatform { arch })
}

fn parse_remote_platform_probe_stdout(stdout: &str) -> Option<(String, String)> {
    let mut os: Option<String> = None;
    let mut arch: Option<String> = None;
    for line in stdout.lines() {
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix(PLATFORM_PROBE_OS_MARKER) {
            let value = value.trim();
            if !value.is_empty() {
                os = Some(value.to_string());
            }
            continue;
        }
        if let Some(value) = trimmed.strip_prefix(PLATFORM_PROBE_ARCH_MARKER) {
            let value = value.trim();
            if !value.is_empty() {
                arch = Some(value.to_string());
            }
        }
    }
    match (os, arch) {
        (Some(os), Some(arch)) => Some((os, arch)),
        _ => None,
    }
}

fn detect_windows_remote_with_cmd(target: &str) -> bool {
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
        .arg("cmd /c ver")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();
    let Ok(output) = output else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let combined = format!(
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    combined.to_ascii_lowercase().contains("windows")
}

fn remote_ctx_bin_parent_dir(remote_ctx_bin: &str) -> Result<String> {
    let path = validate_remote_ctx_bin(remote_ctx_bin)?;
    if path == "~" {
        return Ok("~".to_string());
    }
    let Some((parent, _name)) = path.rsplit_once('/') else {
        anyhow::bail!("remote_ctx_bin must include a file name");
    };
    if parent.is_empty() {
        return Ok("/".to_string());
    }
    Ok(parent.to_string())
}

fn bootstrap_download_base_url() -> String {
    std::env::var("CTX_DOWNLOAD_BASE_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_DOWNLOAD_BASE_URL.to_string())
}

fn remote_daemon_platform_key_for_arch(arch: &str) -> Result<&'static str> {
    match arch {
        "x86_64" => Ok("linux-x64"),
        "aarch64" => Ok("linux-arm64"),
        other => anyhow::bail!("unsupported remote daemon arch for bootstrap: {other}"),
    }
}

fn normalize_sha256_hex(raw: &str) -> Result<String> {
    let normalized = raw.trim().to_ascii_lowercase();
    if normalized.len() != 64 || !normalized.chars().all(|ch| ch.is_ascii_hexdigit()) {
        anyhow::bail!("invalid sha256 digest format");
    }
    Ok(normalized)
}

fn join_url(base: &str, path_or_url: &str) -> String {
    let trimmed = path_or_url.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return trimmed.to_string();
    }
    format!("{}{}", base.trim_end_matches('/'), trimmed)
}

fn fetch_release_manifest_for_channel(channel: &str, base_url: &str) -> Result<ReleaseManifest> {
    let url = format!(
        "{}/releases/{}/latest.json",
        base_url.trim_end_matches('/'),
        channel
    );
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(REMOTE_DAEMON_DOWNLOAD_TIMEOUT_SECS))
        .build()
        .context("building release manifest http client")?;
    let resp = client
        .get(&url)
        .send()
        .with_context(|| format!("fetching release manifest: {url}"))?
        .error_for_status()
        .with_context(|| format!("release manifest http error: {url}"))?;
    resp.json::<ReleaseManifest>()
        .context("parsing release manifest JSON")
}

fn sha256_hex_file(path: &std::path::Path) -> Result<String> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let mut hasher = sha2::Sha256::new();
    hasher.update(&bytes);
    Ok(format!("{:x}", hasher.finalize()))
}

fn managed_remote_daemon_download_path(
    app: &tauri::AppHandle,
    channel: &str,
    platform_key: &str,
    sha256: &str,
) -> Result<PathBuf> {
    let channel = channel.trim();
    if channel.is_empty() {
        anyhow::bail!("channel is required");
    }
    let data_dir = daemon_data_dir(app)?;
    Ok(data_dir
        .join("updates")
        .join("remote-daemon")
        .join(channel)
        .join(platform_key)
        .join(format!("sha256-{sha256}"))
        .join("ctx"))
}

fn ensure_managed_remote_daemon_binary(
    app: &tauri::AppHandle,
    remote_arch: &str,
    channel: &str,
) -> Result<PathBuf> {
    let platform_key = remote_daemon_platform_key_for_arch(remote_arch)?;
    let base_url = bootstrap_download_base_url();
    let manifest = fetch_release_manifest_for_channel(channel, &base_url)?;
    let platform_entry = manifest
        .platforms
        .get(platform_key)
        .ok_or_else(|| anyhow!("release manifest missing platform entry: {platform_key}"))?;
    let daemon_artifact = platform_entry
        .daemon
        .as_ref()
        .ok_or_else(|| anyhow!("release manifest missing daemon artifact for {platform_key}"))?;
    let expected_sha = normalize_sha256_hex(&daemon_artifact.sha256)?;
    let final_path =
        managed_remote_daemon_download_path(app, channel, platform_key, &expected_sha)?;
    if final_path.exists() {
        let digest = sha256_hex_file(&final_path)
            .with_context(|| format!("computing sha256 for {}", final_path.display()))?;
        if digest.eq_ignore_ascii_case(&expected_sha) {
            return Ok(final_path);
        }
        std::fs::remove_file(&final_path).with_context(|| {
            format!(
                "removing corrupted cached remote daemon at {}",
                final_path.display()
            )
        })?;
    }
    let parent = final_path
        .parent()
        .ok_or_else(|| anyhow!("invalid download target path: {}", final_path.display()))?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("creating managed daemon cache dir {}", parent.display()))?;
    let tmp_path = parent.join(format!("ctx.tmp-{}", std::process::id()));
    let artifact_url = join_url(&base_url, &daemon_artifact.url_path);
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(REMOTE_DAEMON_DOWNLOAD_TIMEOUT_SECS))
        .build()
        .context("building managed daemon download client")?;
    let mut resp = client
        .get(&artifact_url)
        .send()
        .with_context(|| format!("downloading managed daemon artifact: {artifact_url}"))?
        .error_for_status()
        .with_context(|| format!("managed daemon artifact download http error: {artifact_url}"))?;
    {
        let mut file = std::fs::File::create(&tmp_path)
            .with_context(|| format!("creating {}", tmp_path.display()))?;
        std::io::copy(&mut resp, &mut file)
            .with_context(|| format!("writing {}", tmp_path.display()))?;
    }
    let digest = sha256_hex_file(&tmp_path)
        .with_context(|| format!("computing sha256 for {}", tmp_path.display()))?;
    if !digest.eq_ignore_ascii_case(&expected_sha) {
        std::fs::remove_file(&tmp_path).ok();
        anyhow::bail!(
            "managed daemon artifact checksum mismatch (expected {}, got {})",
            expected_sha,
            digest
        );
    }
    std::fs::rename(&tmp_path, &final_path).with_context(|| {
        format!(
            "moving managed daemon artifact into cache ({} -> {})",
            tmp_path.display(),
            final_path.display()
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&final_path)
            .with_context(|| format!("reading metadata for {}", final_path.display()))?
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&final_path, perms)
            .with_context(|| format!("setting executable bit on {}", final_path.display()))?;
    }
    Ok(final_path)
}

fn install_remote_daemon_over_ssh(
    app: &tauri::AppHandle,
    host: &str,
    user: Option<&str>,
    remote_platform: RemoteLinuxPlatform,
    remote_ctx_bin: &str,
) -> Result<()> {
    let target = match user {
        Some(u) if !u.trim().is_empty() => format!("{}@{}", u.trim(), host),
        _ => host.to_string(),
    };
    let channel = normalize_update_channel(std::env::var("CTX_DESKTOP_CHANNEL").ok().as_deref())
        .map_err(anyhow::Error::msg)?;
    let local_bin = ensure_managed_remote_daemon_binary(app, remote_platform.arch, &channel)?;
    let parent_dir = remote_ctx_bin_parent_dir(remote_ctx_bin)?;
    let remote_ctx_bin = validate_remote_ctx_bin(remote_ctx_bin)?;
    let temp_remote_path = format!("{remote_ctx_bin}.tmp-{}", std::process::id());
    let install_cmd = format!(
        "mkdir -p {parent} && cat > {tmp} && chmod 755 {tmp} && mv -f {tmp} {dest}",
        parent = remote_path_expr(&parent_dir),
        tmp = remote_path_expr(&temp_remote_path),
        dest = remote_path_expr(&remote_ctx_bin),
    );
    let mut child = new_ssh_command()
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg("ConnectTimeout=15")
        .arg("-o")
        .arg("ServerAliveInterval=15")
        .arg("-o")
        .arg("ServerAliveCountMax=2")
        .arg(target)
        .arg(format!("sh -lc {}", shell_escape(&install_cmd)))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("spawning ssh for remote daemon install")?;
    {
        let mut file = std::fs::File::open(&local_bin)
            .with_context(|| format!("opening managed daemon at {}", local_bin.display()))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("ssh stdin unavailable for daemon install"))?;
        std::io::copy(&mut file, &mut stdin).context("streaming daemon binary over ssh")?;
    }
    let output = child
        .wait_with_output()
        .context("waiting for remote daemon install ssh command")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            anyhow::bail!("remote daemon install failed");
        }
        anyhow::bail!("remote daemon install failed: {stderr}");
    }

    Ok(())
}

fn remote_ctx_bin_exists_over_ssh(
    host: &str,
    user: Option<&str>,
    remote_ctx_bin: &str,
) -> Result<bool> {
    let ctx_bin = validate_remote_ctx_bin(remote_ctx_bin)?;
    let check_cmd = format!(
        "if [ -x {ctx_bin} ]; then exit 0; else exit 1; fi",
        ctx_bin = remote_path_expr(&ctx_bin),
    );
    let output = run_remote_ssh_shell(host, user, &check_cmd)
        .context("checking remote managed daemon binary")?;
    if output.status.success() {
        return Ok(true);
    }
    if output.status.code() == Some(1) {
        return Ok(false);
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let detail = if !stderr.is_empty() { stderr } else { stdout };
    anyhow::bail!("checking remote managed daemon binary failed: {detail}");
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
    let ctx_bin_name = std::path::Path::new(remote_ctx_bin)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("ctx");
    let serve_patterns = [
        format!("{remote_ctx_bin} serve"),
        format!("{ctx_bin_name} serve"),
    ];
    let mut pkill_patterns = Vec::new();
    for serve_pattern in serve_patterns {
        pkill_patterns.push(format!("{serve_pattern} --bind 127.0.0.1:{remote_port}"));
        pkill_patterns.push(format!("{serve_pattern} --bind 0.0.0.0:{remote_port}"));
        pkill_patterns.push(format!("{serve_pattern} --port {remote_port}"));
        pkill_patterns.push(serve_pattern);
    }
    let pkill_chain = pkill_patterns
        .iter()
        .map(|pattern| format!("pkill -f -- {} >/dev/null 2>&1", shell_escape(pattern)))
        .collect::<Vec<_>>()
        .join(" || \\\n");
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
{pkill_chain}; \
status=$?; \
if [ $status -ne 0 ]; then echo \"remote daemon stop failed (pkill exit $status)\" >&2; exit $status; fi; \
sleep 1",
        port = remote_port,
        pkill_chain = pkill_chain,
    )
}

fn run_remote_ssh_shell(host: &str, user: Option<&str>, cmd: &str) -> Result<std::process::Output> {
    let target = match user {
        Some(u) if !u.trim().is_empty() => format!("{}@{}", u.trim(), host),
        _ => host.to_string(),
    };
    let remote_cmd = format!("sh -lc {}", shell_escape(cmd));
    new_ssh_command()
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

fn probe_remote_podman_path(host: &str, user: Option<&str>) -> Result<Option<String>> {
    let output = run_remote_ssh_shell(
        host,
        user,
        "if command -v podman >/dev/null 2>&1; then command -v podman; else exit 1; fi",
    )
    .context("probing remote podman path")?;
    if !output.status.success() {
        if output.status.code() == Some(1) {
            return Ok(None);
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if !stderr.is_empty() { stderr } else { stdout };
        anyhow::bail!("remote podman probe failed: {detail}");
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path.is_empty() {
        return Ok(None);
    }
    Ok(Some(path))
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

    let mut cmd = new_ssh_command();
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
    // Canonical path: pass an explicit podman binary path into the remote daemon when available,
    // so runtime execution does not depend on daemon-process PATH lookup.
    let mut daemon_env = Vec::<String>::new();
    if let Some(remote_podman_path) = probe_remote_podman_path(host, user)? {
        daemon_env.push(format!(
            "CTX_PODMAN_PATH={}",
            shell_escape(remote_podman_path.as_str())
        ));
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
    fn validate_remote_ctx_bin_values() {
        let valid = validate_remote_ctx_bin("/opt/ctx/bin/ctx").expect("absolute path is valid");
        assert_eq!(valid, "/opt/ctx/bin/ctx");
        let valid_home = validate_remote_ctx_bin("~/.ctx/bin/ctx").expect("~/ path is valid");
        assert_eq!(valid_home, "~/.ctx/bin/ctx");

        let err_empty = validate_remote_ctx_bin(" ").expect_err("empty path must fail");
        assert!(
            err_empty.to_string().contains("remote_ctx_bin is required"),
            "unexpected error: {err_empty:#}"
        );
        let err_rel = validate_remote_ctx_bin("ctx").expect_err("relative path must fail");
        assert!(
            err_rel
                .to_string()
                .contains("must be an absolute path or ~/ path"),
            "unexpected error: {err_rel:#}"
        );
    }

    #[test]
    fn remote_ctx_bin_parent_dir_handles_home_and_absolute_paths() {
        let parent_home =
            remote_ctx_bin_parent_dir("~/.ctx/bin/ctx").expect("home-based path parent");
        assert_eq!(parent_home, "~/.ctx/bin");
        let parent_abs =
            remote_ctx_bin_parent_dir("/opt/ctx/bin/ctx").expect("absolute path parent");
        assert_eq!(parent_abs, "/opt/ctx/bin");
    }

    #[test]
    fn remote_arch_mapping_supports_expected_linux_arches() {
        assert_eq!(normalize_remote_arch_token("x86_64"), Some("x86_64"));
        assert_eq!(normalize_remote_arch_token("amd64"), Some("x86_64"));
        assert_eq!(normalize_remote_arch_token("aarch64"), Some("aarch64"));
        assert_eq!(normalize_remote_arch_token("arm64"), Some("aarch64"));
        assert_eq!(normalize_remote_arch_token("i686"), None);
    }

    #[test]
    fn remote_ctx_bootstrap_plan_is_managed_or_install() {
        assert_eq!(
            plan_remote_ctx_bootstrap(true),
            RemoteCtxBootstrapPlan::UseManaged
        );
        assert_eq!(
            plan_remote_ctx_bootstrap(false),
            RemoteCtxBootstrapPlan::InstallManaged
        );
    }

    #[test]
    fn windows_detection_helpers_match_expected_tokens() {
        assert!(is_windows_os_token("Windows_NT"));
        assert!(is_windows_os_token("MINGW64_NT-10.0-22631"));
        assert!(!is_windows_os_token("Linux"));
        assert!(looks_like_windows_shell_error(
            "'sh' is not recognized as an internal or external command"
        ));
        assert!(!looks_like_windows_shell_error(
            "ssh: connect to host example port 22: timed out"
        ));
    }

    #[test]
    fn ssh_auth_failure_detection_matches_permission_denied_errors() {
        assert!(looks_like_ssh_auth_failure(
            "ssh failed to probe remote platform: Permission denied (publickey,password)."
        ));
        assert!(looks_like_ssh_auth_failure(
            "ssh failed: authentication failed for devbox.example"
        ));
        assert!(!looks_like_ssh_auth_failure(
            "ssh: connect to host devbox.example port 22: Operation timed out"
        ));
    }

    #[test]
    fn ssh_bootstrap_authorized_keys_command_is_idempotent() {
        let cmd = ssh_authorized_keys_install_command();
        assert!(
            cmd.contains("grep -qxF \"$key\" \"$HOME/.ssh/authorized_keys\" ||"),
            "expected duplicate guard in bootstrap command: {cmd}"
        );
        assert!(
            !cmd.contains("cat >> \"$HOME/.ssh/authorized_keys\""),
            "bootstrap command should no longer append blindly: {cmd}"
        );
    }

    #[test]
    fn reap_password_once_child_after_input_error_reports_remote_output() {
        #[cfg(windows)]
        let mut cmd = {
            let mut c = Command::new("cmd");
            c.arg("/C").arg("echo remote-ssh-failed 1>&2 & exit /b 19");
            c
        };
        #[cfg(not(windows))]
        let mut cmd = {
            let mut c = Command::new("sh");
            c.arg("-lc").arg("echo remote-ssh-failed >&2; exit 19");
            c
        };
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let child = cmd.spawn().expect("spawn failure fixture");
        let err = reap_password_once_child_after_input_error(child, "stdin write failed");
        let msg = err.to_string();
        assert!(
            msg.contains("stdin write failed"),
            "unexpected error: {msg}"
        );
        assert!(
            msg.contains("password-once ssh exited with status"),
            "missing process status detail: {msg}"
        );
        assert!(
            msg.contains("remote-ssh-failed"),
            "missing remote stderr detail: {msg}"
        );
    }

    #[test]
    fn parse_remote_platform_probe_output_handles_noise() {
        let noisy = "welcome banner\nmotd here\n__CTX_PLATFORM_OS__Linux\nother line\n__CTX_PLATFORM_ARCH__x86_64\n";
        let parsed = parse_remote_platform_probe_stdout(noisy)
            .expect("expected marker-based platform parse to succeed");
        assert_eq!(parsed.0, "Linux");
        assert_eq!(parsed.1, "x86_64");
    }

    #[test]
    fn parse_remote_platform_probe_output_requires_markers() {
        assert!(
            parse_remote_platform_probe_stdout("Linux\nx86_64\n").is_none(),
            "legacy non-marker output should not be parsed"
        );
    }

    #[test]
    fn ssh_config_override_normalization() {
        assert_eq!(
            normalized_ssh_config_override(Some(" /tmp/ctx-fixture-ssh-config ")),
            Some("/tmp/ctx-fixture-ssh-config".to_string())
        );
        assert_eq!(normalized_ssh_config_override(Some("   ")), None);
        assert_eq!(normalized_ssh_config_override(None), None);
    }

    #[test]
    fn ssh_stderr_snippet_trims_content() {
        let log = std::sync::Arc::new(std::sync::Mutex::new("  stderr line  ".to_string()));
        assert_eq!(ssh_stderr_snippet(&log), "stderr line".to_string());
        let empty = std::sync::Arc::new(std::sync::Mutex::new("  ".to_string()));
        assert_eq!(ssh_stderr_snippet(&empty), String::new());
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
    fn remote_daemon_platform_key_mapping() {
        assert_eq!(
            remote_daemon_platform_key_for_arch("x86_64").expect("x86_64 should map"),
            "linux-x64"
        );
        assert_eq!(
            remote_daemon_platform_key_for_arch("aarch64").expect("aarch64 should map"),
            "linux-arm64"
        );
        assert!(
            remote_daemon_platform_key_for_arch("armv7").is_err(),
            "unsupported arch must fail"
        );
    }

    #[test]
    fn normalize_sha256_hex_validation() {
        let valid = "A".repeat(64);
        assert_eq!(
            normalize_sha256_hex(&valid).expect("valid digest should normalize"),
            "a".repeat(64)
        );
        assert!(normalize_sha256_hex("abc").is_err());
        assert!(normalize_sha256_hex(&"z".repeat(64)).is_err());
    }

    #[test]
    fn join_url_prefers_absolute_urls_and_joins_relative_paths() {
        assert_eq!(
            join_url(
                "https://api.ctx.rs/functions/v1",
                "/releases/stable/latest.json"
            ),
            "https://api.ctx.rs/functions/v1/releases/stable/latest.json"
        );
        assert_eq!(
            join_url(
                "https://api.ctx.rs/functions/v1",
                "https://example.test/file"
            ),
            "https://example.test/file"
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
            "expected absolute-path serve fallback pattern in stop command: {cmd}"
        );
        assert!(
            cmd.contains("ctx serve"),
            "expected basename serve fallback pattern in stop command: {cmd}"
        );
    }
}
