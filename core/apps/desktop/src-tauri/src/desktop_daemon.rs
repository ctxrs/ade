use super::*;

use std::fs::OpenOptions;

use crate::desktop_local_daemon::ensure_local_connection;

mod bundle_preflight;
mod health;
mod login_relay;
mod path_env;
#[cfg(test)]
mod tests;

pub(super) use health::{
    daemon_health, existing_local_daemon_matches, existing_local_daemon_matches_or_absent,
    local_daemon_health_matches_expected, normalize_daemon_pid, probe_daemon_health,
    probe_daemon_health_with_retry, probe_local_daemon_health_with_retry,
    reclaim_incompatible_local_daemon, should_reclaim_incompatible_local_daemon,
    spawned_local_daemon_incompatibility_message, terminate_pid, wait_for_daemon_reclaim,
};
pub(super) use login_relay::desktop_start_codex_login_relay;
use login_relay::is_loopback_host_name;
use path_env::{resolve_daemon_path_env, resolve_local_daemon_path_env};

const SSH_CONFIG_OVERRIDE_ENV: &str = "CTX_DESKTOP_SSH_CONFIG_PATH";
const AVF_LINUX_HELPER_PATH_ENV: &str = "CTX_AVF_LINUX_HELPER_PATH";
const DESKTOP_BUNDLE_DIR_ENV: &str = "CTX_BUNDLE_DIR";
const DESKTOP_DAEMON_BIN_NAME: &str = "ctx-daemon";
const AVF_GUEST_GATEWAY_HOST: &str = "192.168.64.1";
const DAEMON_AUTOMATION_ENV_BLOCKLIST: &[&str] = &[
    "AUTOMATION_LIBRARY_PATH",
    "AUTOMATION_PORT",
    "REMOTE_WEBDRIVER_URL",
    "TAURI_DRIVER_PORT",
    "TEST_RUNNER_BACKEND_PORT",
];

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

fn avf_guest_gateway_bind(local_port: u16) -> Option<String> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let bind_addr = format!("{AVF_GUEST_GATEWAY_HOST}:{local_port}");
    match std::net::TcpListener::bind(&bind_addr) {
        Ok(listener) => {
            drop(listener);
            Some(bind_addr)
        }
        Err(_) => None,
    }
}

fn daemon_env_unset_args() -> Vec<std::ffi::OsString> {
    let mut args = Vec::with_capacity(DAEMON_AUTOMATION_ENV_BLOCKLIST.len() * 2);
    for key in DAEMON_AUTOMATION_ENV_BLOCKLIST {
        args.push("-u".into());
        args.push((*key).into());
    }
    args
}

fn strip_automation_env(cmd: &mut Command) {
    for key in DAEMON_AUTOMATION_ENV_BLOCKLIST {
        cmd.env_remove(key);
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct DaemonAuthFile {
    pub(super) token: String,
    #[serde(default)]
    pub(super) daemon_url: Option<String>,
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

pub(super) fn enforce_desktop_parity_bundle_preflight(app: &tauri::AppHandle) -> Result<()> {
    bundle_preflight::enforce_desktop_parity_bundle_preflight(desktop_bundle_dir(app).as_deref())
}

pub(super) fn desktop_dev_instance_id() -> &'static str {
    option_env!("CTX_DEV_INSTANCE_ID").unwrap_or("unknown")
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
    // Debug desktop builds must not share the release app's local daemon state by default.
    let root = desktop_local_data_root()?;
    std::fs::create_dir_all(&root)
        .with_context(|| format!("creating ctx home {}", root.display()))?;
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

fn select_optional_bin_path(
    name: &str,
    bundled: Option<PathBuf>,
    dev: Option<PathBuf>,
    debug_build: bool,
    is_macos: bool,
) -> Option<PathBuf> {
    if !debug_build {
        return bundled;
    }
    if is_macos && name == "ctx-avf-linux-helper" {
        return bundled.or(dev);
    }
    dev.or(bundled)
}

fn resolve_optional_bin(app: &tauri::AppHandle, name: &str) -> Option<PathBuf> {
    let bundled = resource_bin(app, name);
    let dev = if cfg!(debug_assertions) {
        dev_bin(name)
    } else {
        None
    };
    select_optional_bin_path(
        name,
        bundled,
        dev,
        cfg!(debug_assertions),
        cfg!(target_os = "macos"),
    )
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

fn configured_bundle_dir() -> Option<PathBuf> {
    let raw = std::env::var(DESKTOP_BUNDLE_DIR_ENV).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let path = PathBuf::from(trimmed);
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

fn select_bundle_dir_path(
    configured: Option<PathBuf>,
    bundled: Option<PathBuf>,
    dev: Option<PathBuf>,
) -> Option<PathBuf> {
    configured.or(bundled).or(dev)
}

fn desktop_bundle_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
    let bundled = app
        .path()
        .resource_dir()
        .ok()
        .map(|p| p.join("bundles"))
        .filter(|p| p.exists());
    select_bundle_dir_path(configured_bundle_dir(), bundled, dev_bundle_dir())
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
    let ctx_bin = resolve_daemon_bin(app)?;
    let avf_linux_helper_bin = resolve_optional_bin(app, "ctx-avf-linux-helper");
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
    let bundle_dir = desktop_bundle_dir(app);
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
        if let Some(helper) = avf_linux_helper_bin.as_ref() {
            cmd.arg("--setenv").arg(format!(
                "{AVF_LINUX_HELPER_PATH_ENV}={}",
                helper.to_string_lossy()
            ));
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
        cmd.arg("/usr/bin/env");
        cmd.args(daemon_env_unset_args());
        cmd.arg(&ctx_bin);
        cmd
    } else {
        let mut cmd = Command::new(&ctx_bin);
        strip_automation_env(&mut cmd);
        if let Some(path_env) = resolved_path_env.as_ref() {
            cmd.env("PATH", path_env);
        }
        if let Some(dist) = web_dist.as_ref() {
            cmd.env("CTX_WEB_DIST", dist.to_string_lossy().to_string());
        }
        if let Some(mcp) = mcp_bin.as_ref() {
            cmd.env("CTX_MCP_COMMAND", mcp.to_string_lossy().to_string());
        }
        if let Some(helper) = avf_linux_helper_bin.as_ref() {
            cmd.env(
                AVF_LINUX_HELPER_PATH_ENV,
                helper.to_string_lossy().to_string(),
            );
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
        .arg(format!("127.0.0.1:{local_port}"));
    if avf_linux_helper_bin.is_some() {
        if let Some(avf_bind) = avf_guest_gateway_bind(local_port) {
            cmd.arg("--bind").arg(avf_bind);
        }
    }
    cmd.arg("--data-dir")
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
