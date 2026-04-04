use super::*;

const LOCAL_ADMIN_PASSWORD_REQUIRED_SENTINEL: &str = "CTX_LOCAL_ADMIN_PASSWORD_REQUIRED";
const REMOTE_ADMIN_PASSWORD_REQUIRED_SENTINEL: &str = "CTX_REMOTE_ADMIN_PASSWORD_REQUIRED";
pub(crate) const ROOTFUL_WRAPPER_PATH: &str = "/usr/local/bin/ctx-rootful-nerdctl";
pub(crate) const MANAGED_CONTAINERD_ADDRESS: &str = "/run/containerd/containerd.sock";
pub(crate) const MANAGED_CONTAINERD_NAMESPACE: &str = "default";
const LOCAL_PREFETCH_DELAY_MS: u64 = 500;
const PREFETCH_WAIT_POLL_MS: u64 = 250;
const PREFETCH_WAIT_TIMEOUT_MS: u64 = 180_000;

const BOOTSTRAP_SCRIPT: &str =
    include_str!("../../../../crates/ctx-http/src/workspace_runtime/linux_sandbox_bootstrap.sh");

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
struct LinuxSandboxBootstrapStatus {
    state: String,
    #[serde(default)]
    message: String,
}

#[derive(Debug, Clone, Deserialize)]
struct LinuxSandboxPrepareResponse {
    ready: bool,
    needs_password: bool,
    #[serde(default)]
    message: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DesktopLinuxSandboxEnsureResp {
    pub(crate) ready: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DesktopLocalLinuxSandboxEnsureReq {
    #[serde(default)]
    pub(crate) admin_password_once: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DesktopRemoteLinuxSandboxEnsureReq {
    #[serde(default)]
    pub(crate) admin_password_once: Option<String>,
}

fn local_prefetch_inflight() -> &'static std::sync::Mutex<bool> {
    static INFLIGHT: std::sync::OnceLock<std::sync::Mutex<bool>> = std::sync::OnceLock::new();
    INFLIGHT.get_or_init(|| std::sync::Mutex::new(false))
}

fn linux_sandbox_root(data_dir: &Path) -> PathBuf {
    data_dir.join("linux-sandbox-runtime")
}

fn linux_sandbox_status_path(data_dir: &Path) -> PathBuf {
    linux_sandbox_root(data_dir).join("status.json")
}

fn linux_sandbox_script_path(data_dir: &Path) -> PathBuf {
    linux_sandbox_root(data_dir).join("bootstrap.sh")
}

fn write_local_bootstrap_script(data_dir: &Path) -> Result<PathBuf> {
    let root = linux_sandbox_root(data_dir);
    std::fs::create_dir_all(&root)
        .with_context(|| format!("creating linux sandbox bootstrap dir {}", root.display()))?;
    let script_path = linux_sandbox_script_path(data_dir);
    let should_write = match std::fs::read_to_string(&script_path) {
        Ok(existing) => existing != BOOTSTRAP_SCRIPT,
        Err(_) => true,
    };
    if should_write {
        std::fs::write(&script_path, BOOTSTRAP_SCRIPT)
            .with_context(|| format!("writing {}", script_path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o700))
                .with_context(|| format!("setting mode on {}", script_path.display()))?;
        }
    }
    Ok(script_path)
}

fn parse_status_json(raw: &str) -> Result<LinuxSandboxBootstrapStatus> {
    serde_json::from_str::<LinuxSandboxBootstrapStatus>(raw.trim())
        .context("parsing linux sandbox bootstrap status")
}

fn read_local_status(data_dir: &Path) -> Result<LinuxSandboxBootstrapStatus> {
    let status_path = linux_sandbox_status_path(data_dir);
    if !status_path.exists() {
        return Ok(LinuxSandboxBootstrapStatus {
            state: "download_pending".to_string(),
            message: String::new(),
        });
    }
    let raw = std::fs::read_to_string(&status_path)
        .with_context(|| format!("reading {}", status_path.display()))?;
    parse_status_json(&raw)
}

fn read_local_status_via_bootstrap(data_dir: &Path) -> Result<LinuxSandboxBootstrapStatus> {
    let script_path = write_local_bootstrap_script(data_dir)?;
    let output = Command::new(&script_path)
        .arg("status")
        .arg("--data-dir")
        .arg(data_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .context("running local linux sandbox bootstrap status")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if output.status.success() {
        return parse_status_json(&stdout);
    }
    if let Ok(status) = parse_status_json(&stdout) {
        return Ok(status);
    }
    let detail = format_output_detail(&output);
    anyhow::bail!("local linux sandbox bootstrap status failed: {detail}");
}

fn format_output_detail(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stderr.is_empty() {
        return stderr;
    }
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn local_stage_spawn(app: tauri::AppHandle) {
    if !cfg!(target_os = "linux") {
        return;
    }
    let should_spawn = {
        let mut guard = match local_prefetch_inflight().lock() {
            Ok(guard) => guard,
            Err(_) => return,
        };
        if *guard {
            false
        } else {
            *guard = true;
            true
        }
    };
    if !should_spawn {
        return;
    }
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(LOCAL_PREFETCH_DELAY_MS));
        let result = (|| -> Result<()> {
            let data_dir = daemon_data_dir(&app)?;
            let script_path = write_local_bootstrap_script(&data_dir)?;
            let output = Command::new(&script_path)
                .arg("stage")
                .arg("--data-dir")
                .arg(&data_dir)
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .output()
                .context("running local linux sandbox bootstrap stage")?;
            if !output.status.success() {
                let detail = format_output_detail(&output);
                anyhow::bail!("local linux sandbox bootstrap stage failed: {detail}");
            }
            Ok(())
        })();
        if let Err(err) = result {
            eprintln!("local linux sandbox bootstrap stage failed: {err:#}");
        }
        if let Ok(mut guard) = local_prefetch_inflight().lock() {
            *guard = false;
        }
    });
}

pub(crate) fn schedule_local_linux_sandbox_prefetch(app: tauri::AppHandle) {
    local_stage_spawn(app);
}

pub(crate) fn local_linux_sandbox_runtime_ready(data_dir: &Path) -> bool {
    matches!(
        read_local_status_via_bootstrap(data_dir),
        Ok(status) if status.state == "ready"
    )
}

pub(crate) fn configure_local_linux_sandbox_daemon_env(cmd: &mut Command, data_dir: &Path) {
    if !cfg!(target_os = "linux") {
        return;
    }
    if !local_linux_sandbox_runtime_ready(data_dir) {
        return;
    }
    cmd.env("CTX_HARNESS_SANDBOX_CLI_PATH", ROOTFUL_WRAPPER_PATH);
    cmd.env("CONTAINERD_ADDRESS", MANAGED_CONTAINERD_ADDRESS);
    cmd.env("CONTAINERD_NAMESPACE", MANAGED_CONTAINERD_NAMESPACE);
}

pub(crate) fn remote_linux_sandbox_daemon_env_prefix(_data_dir: &str) -> String {
    format!(
        "if [ -x {wrapper} ] && [ -S {address} ] && {wrapper} info >/dev/null 2>&1; then export CTX_HARNESS_SANDBOX_CLI_PATH={wrapper} CONTAINERD_ADDRESS={address} CONTAINERD_NAMESPACE={namespace}; fi;",
        wrapper = shell_escape(ROOTFUL_WRAPPER_PATH),
        address = shell_escape(MANAGED_CONTAINERD_ADDRESS),
        namespace = shell_escape(MANAGED_CONTAINERD_NAMESPACE),
    )
}

fn wait_for_local_stage_completion(data_dir: &Path) -> Result<LinuxSandboxBootstrapStatus> {
    let started = Instant::now();
    loop {
        let status = read_local_status(data_dir)?;
        match status.state.as_str() {
            "download_pending" | "downloading" => {
                if started.elapsed() > Duration::from_millis(PREFETCH_WAIT_TIMEOUT_MS) {
                    anyhow::bail!("timed out waiting for Linux sandbox downloads to finish");
                }
                std::thread::sleep(Duration::from_millis(PREFETCH_WAIT_POLL_MS));
            }
            _ => return Ok(status),
        }
    }
}

fn is_posix_safe_username(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn current_local_user() -> Result<String> {
    let output = std::process::Command::new("id")
        .arg("-un")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .context("running id -un for Linux sandbox activation")?;
    if !output.status.success() {
        let detail = format_output_detail(&output);
        if detail.is_empty() {
            anyhow::bail!("id -un failed while preparing Linux sandbox runtime");
        }
        anyhow::bail!("id -un failed while preparing Linux sandbox runtime: {detail}");
    }
    let trimmed = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if trimmed.is_empty() {
        anyhow::bail!("id -un returned an empty username");
    }
    if !is_posix_safe_username(&trimmed) {
        anyhow::bail!("id -un returned a non-POSIX-safe username");
    }
    Ok(trimmed)
}

enum LocalActivationOutcome {
    Ready,
    NeedsPassword,
}

fn activation_args(data_dir: &Path, allow_user: &str) -> Vec<String> {
    vec![
        "/bin/sh".to_string(),
        "-s".to_string(),
        "--".to_string(),
        "activate".to_string(),
        "--data-dir".to_string(),
        data_dir.to_string_lossy().to_string(),
        "--allow-user".to_string(),
        allow_user.to_string(),
    ]
}

fn run_command_with_stdin(
    mut command: Command,
    stdin: &[u8],
    context: &str,
) -> Result<std::process::Output> {
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().with_context(|| context.to_string())?;
    if let Some(mut child_stdin) = child.stdin.take() {
        use std::io::Write as _;
        child_stdin
            .write_all(stdin)
            .with_context(|| format!("{context}: writing stdin payload"))?;
    }
    child
        .wait_with_output()
        .with_context(|| format!("{context}: waiting for command output"))
}

fn local_sudo_needs_password(output: &std::process::Output) -> bool {
    let detail = format_output_detail(output).to_ascii_lowercase();
    detail.contains("a password is required")
        || detail.contains("password is required")
        || detail.contains("sorry, try again")
        || detail.contains("incorrect password")
        || (detail.contains("sudo:")
            && (detail.contains("no tty")
                || detail.contains("askpass")
                || detail.contains("password")))
}

fn run_local_activation(
    data_dir: &Path,
    allow_user: &str,
    admin_password_once: Option<&str>,
) -> Result<LocalActivationOutcome> {
    let args = activation_args(data_dir, allow_user);
    let output = run_command_with_stdin(
        {
            let mut command = Command::new("sudo");
            command.arg("--non-interactive").args(&args);
            command
        },
        BOOTSTRAP_SCRIPT.as_bytes(),
        "running local Linux sandbox activation via sudo",
    )?;
    if output.status.success() {
        return Ok(LocalActivationOutcome::Ready);
    }
    if local_sudo_needs_password(&output) && admin_password_once.is_none() {
        return Ok(LocalActivationOutcome::NeedsPassword);
    }
    if let Some(password) = admin_password_once {
        let mut stdin = Vec::with_capacity(password.len() + BOOTSTRAP_SCRIPT.len() + 1);
        stdin.extend_from_slice(password.as_bytes());
        stdin.push(b'\n');
        stdin.extend_from_slice(BOOTSTRAP_SCRIPT.as_bytes());
        let output = run_command_with_stdin(
            {
                let mut command = Command::new("sudo");
                command.arg("-S").arg("-p").arg("").args(&args);
                command
            },
            &stdin,
            "running local Linux sandbox activation via sudo password",
        )?;
        if output.status.success() {
            return Ok(LocalActivationOutcome::Ready);
        }
        if local_sudo_needs_password(&output) {
            return Ok(LocalActivationOutcome::NeedsPassword);
        }
        let detail = format_output_detail(&output);
        anyhow::bail!("Preparing Linux sandbox runtime failed. {detail}");
    }
    let detail = format_output_detail(&output);
    anyhow::bail!("Preparing Linux sandbox runtime failed. {detail}");
}

fn local_linux_platform_is_supported() -> bool {
    cfg!(target_os = "linux")
}

#[tauri::command]
pub(crate) async fn desktop_ensure_local_linux_sandbox_ready(
    app: tauri::AppHandle,
    req: DesktopLocalLinuxSandboxEnsureReq,
) -> Result<DesktopLinuxSandboxEnsureResp, String> {
    if !local_linux_platform_is_supported() {
        return Ok(DesktopLinuxSandboxEnsureResp { ready: true });
    }
    tauri::async_runtime::spawn_blocking(move || {
        let data_dir = daemon_data_dir(&app).map_err(to_err)?;
        write_local_bootstrap_script(&data_dir).map_err(to_err)?;
        local_stage_spawn(app.clone());
        let status = wait_for_local_stage_completion(&data_dir).map_err(to_err)?;
        match status.state.as_str() {
            "ready" => Ok(DesktopLinuxSandboxEnsureResp { ready: true }),
            "downloaded_not_activated" => {
                let allow_user = current_local_user().map_err(to_err)?;
                match run_local_activation(
                    &data_dir,
                    &allow_user,
                    req.admin_password_once.as_deref(),
                )
                .map_err(to_err)?
                {
                    LocalActivationOutcome::NeedsPassword => {
                        return Err(format!(
                            "{LOCAL_ADMIN_PASSWORD_REQUIRED_SENTINEL}: Local admin password required to prepare sandbox on this machine."
                        ));
                    }
                    LocalActivationOutcome::Ready => {}
                }
                let state = app.state::<ConnectionManager>();
                let manager: &ConnectionManager = state.inner();
                let desktop_version = app.package_info().version.to_string();
                let desktop_dev_instance_id = desktop_dev_instance_id();
                restart_local_with_spawn(manager, || {
                    spawn_and_validate_local_daemon(
                        &app,
                        &data_dir,
                        &desktop_version,
                        desktop_dev_instance_id,
                    )
                })
                .map_err(to_err)?;
                Ok(DesktopLinuxSandboxEnsureResp { ready: true })
            }
            "manual_runtime_required" => Err(if status.message.trim().is_empty() {
                "Preparing Linux sandbox runtime failed. Managed sandbox setup is currently supported on Ubuntu/Debian only.".to_string()
            } else {
                format!("Preparing Linux sandbox runtime failed. {}", status.message.trim())
            }),
            other => Err(if status.message.trim().is_empty() {
                format!("Preparing Linux sandbox runtime failed. bootstrap_state={other}")
            } else {
                format!("Preparing Linux sandbox runtime failed. {}", status.message.trim())
            }),
        }
    })
    .await
    .map_err(|err| format!("Preparing Linux sandbox runtime failed. {err}"))?
}

fn active_remote_passwords(target: &SshConnectionTarget) -> (Option<String>, Option<String>) {
    (
        target.runtime.ssh_password_once.clone(),
        target.runtime.admin_password_once.clone(),
    )
}

fn build_remote_stage_request() -> DesktopDaemonRequest {
    DesktopDaemonRequest {
        method: "POST".to_string(),
        path: "/api/execution/linux_sandbox_runtime/stage".to_string(),
        body: None,
        headers: vec![("Content-Type".to_string(), "application/json".to_string())],
    }
}

fn parse_remote_daemon_json<T: serde::de::DeserializeOwned>(
    response: DesktopHttpResponse,
    expected_path: &str,
) -> Result<T> {
    if (200..300).contains(&response.status) {
        return serde_json::from_str::<T>(&response.body)
            .with_context(|| format!("parsing {expected_path} response"));
    }
    let body = response.body.trim();
    if body.is_empty() {
        anyhow::bail!("{expected_path} failed with status {}", response.status);
    }
    anyhow::bail!(
        "{expected_path} failed with status {}: {body}",
        response.status
    );
}

fn remote_daemon_missing_linux_sandbox_endpoint(response: &DesktopHttpResponse) -> bool {
    response.status == 404
}

fn select_remote_admin_password<'a>(
    requested_admin_password: Option<&'a str>,
    cached_admin_password: Option<&'a str>,
    ssh_password_once: Option<&'a str>,
) -> Option<&'a str> {
    // EXCEPTION: Product decision for remote Linux bootstrap is to reuse the
    // session's one-time SSH password for the first sudo attempt when no
    // distinct admin password has been provided, to minimize prompts.
    requested_admin_password
        .or(cached_admin_password)
        .or(ssh_password_once)
}

fn runtime_with_persisted_remote_admin_password(
    mut runtime: SshRuntimeMetadata,
    password: &str,
) -> SshRuntimeMetadata {
    runtime.admin_password_once = Some(password.to_string());
    runtime
}

fn remote_stage_status(manager: &ConnectionManager) -> Result<LinuxSandboxBootstrapStatus> {
    parse_remote_daemon_json(
        manager.daemon_request(build_remote_stage_request())?,
        "/api/execution/linux_sandbox_runtime/stage",
    )
}

fn remote_stage_status_with_upgrade(
    app: &tauri::AppHandle,
    manager: &ConnectionManager,
) -> Result<LinuxSandboxBootstrapStatus> {
    let response = manager.daemon_request(build_remote_stage_request())?;
    if remote_daemon_missing_linux_sandbox_endpoint(&response) {
        update_current_remote_daemon(app, manager, None)?;
        return remote_stage_status(manager);
    }
    parse_remote_daemon_json(response, "/api/execution/linux_sandbox_runtime/stage")
}

fn build_remote_prepare_request(sudo_password: Option<&str>) -> DesktopDaemonRequest {
    DesktopDaemonRequest {
        method: "POST".to_string(),
        path: "/api/execution/linux_sandbox_runtime/prepare".to_string(),
        body: Some(
            serde_json::json!({
                "activation_mode": "remote",
                "sudo_password": sudo_password,
            })
            .to_string(),
        ),
        headers: vec![("Content-Type".to_string(), "application/json".to_string())],
    }
}

fn remote_prepare_status(
    manager: &ConnectionManager,
    sudo_password: Option<&str>,
) -> Result<LinuxSandboxPrepareResponse> {
    parse_remote_daemon_json(
        manager.daemon_request(build_remote_prepare_request(sudo_password))?,
        "/api/execution/linux_sandbox_runtime/prepare",
    )
}

#[tauri::command]
pub(crate) async fn desktop_ensure_remote_linux_sandbox_ready(
    app: tauri::AppHandle,
    req: DesktopRemoteLinuxSandboxEnsureReq,
) -> Result<DesktopLinuxSandboxEnsureResp, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ConnectionManager>();
        let manager: &ConnectionManager = state.inner();
        let target = manager.ssh_target().map_err(to_err)?;
        let (ssh_password_once, cached_admin_password) = active_remote_passwords(&target);
        let status = remote_stage_status_with_upgrade(&app, manager).map_err(to_err)?;
        match status.state.as_str() {
            "ready" => Ok(DesktopLinuxSandboxEnsureResp { ready: true }),
            "manual_runtime_required" => Err(if status.message.trim().is_empty() {
                "Preparing sandbox on remote host failed. Managed sandbox setup is currently supported on Ubuntu/Debian only.".to_string()
            } else {
                format!("Preparing sandbox on remote host failed. {}", status.message.trim())
            }),
            "downloaded_not_activated" => {
                let selected_password = select_remote_admin_password(
                    req.admin_password_once.as_deref(),
                    cached_admin_password.as_deref(),
                    ssh_password_once.as_deref(),
                );
                let prepare = remote_prepare_status(manager, selected_password).map_err(to_err)?;
                if prepare.ready {
                    if let Some(password) = selected_password {
                        let latest_runtime = manager.ssh_target().map_err(to_err)?.runtime;
                        manager
                            .update_ssh_runtime(runtime_with_persisted_remote_admin_password(
                                latest_runtime,
                                password,
                            ))
                            .map_err(to_err)?;
                    }
                    return Ok(DesktopLinuxSandboxEnsureResp { ready: true });
                }
                if prepare.needs_password {
                    return Err(format!(
                        "{REMOTE_ADMIN_PASSWORD_REQUIRED_SENTINEL}: {}",
                        if prepare.message.trim().is_empty() {
                            "Remote admin password required to prepare sandbox on this host."
                        } else {
                            prepare.message.trim()
                        }
                    ));
                }
                Err(if prepare.message.trim().is_empty() {
                    "Preparing sandbox on remote host failed.".to_string()
                } else {
                    format!("Preparing sandbox on remote host failed. {}", prepare.message.trim())
                })
            }
            other => Err(if status.message.trim().is_empty() {
                format!("Preparing sandbox on remote host failed. bootstrap_state={other}")
            } else {
                format!("Preparing sandbox on remote host failed. {}", status.message.trim())
            }),
        }
    })
    .await
    .map_err(|err| format!("Preparing sandbox on remote host failed. {err}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_daemon_env_prefix_checks_ready_marker() {
        let prefix = remote_linux_sandbox_daemon_env_prefix("~/.ctx");
        assert!(prefix.contains("CTX_HARNESS_SANDBOX_CLI_PATH"));
        assert!(prefix.contains("info >/dev/null 2>&1"));
        assert!(prefix.contains(ROOTFUL_WRAPPER_PATH));
    }

    #[test]
    fn parse_status_json_reads_expected_shape() {
        let status = parse_status_json(
            r#"{"state":"downloaded_not_activated","supported":true,"message":"","distro":"ubuntu"}"#,
        )
        .expect("status should parse");
        assert_eq!(status.state, "downloaded_not_activated");
        assert!(status.message.is_empty());
    }

    #[test]
    fn select_remote_admin_password_prefers_fresh_admin_entry() {
        let selected = select_remote_admin_password(
            Some("fresh-admin"),
            Some("cached-admin"),
            Some("ssh-password"),
        );
        assert_eq!(selected, Some("fresh-admin"));
    }

    #[test]
    fn select_remote_admin_password_falls_back_to_ssh_password() {
        let selected = select_remote_admin_password(None, None, Some("ssh-password"));
        assert_eq!(selected, Some("ssh-password"));
    }

    #[test]
    fn missing_remote_linux_sandbox_endpoint_detected_from_404() {
        assert!(remote_daemon_missing_linux_sandbox_endpoint(&DesktopHttpResponse {
            status: 404,
            body: String::new(),
            content_type: None,
        }));
        assert!(!remote_daemon_missing_linux_sandbox_endpoint(&DesktopHttpResponse {
            status: 200,
            body: String::new(),
            content_type: None,
        }));
    }

    #[test]
    fn posix_safe_username_rejects_shell_metacharacters() {
        assert!(is_posix_safe_username("ctx-user_01"));
        assert!(!is_posix_safe_username("ctx user"));
        assert!(!is_posix_safe_username("ctx$(rm -rf /)"));
    }

    #[test]
    fn persisting_remote_admin_password_keeps_latest_runtime_paths() {
        let updated = runtime_with_persisted_remote_admin_password(
            SshRuntimeMetadata {
                managed_ctx_bin: "~/.ctx/bin/ctx-managed".to_string(),
                active_ctx_bin: Some("~/.ctx/bin/ctx-active".to_string()),
                ssh_password_once: None,
                admin_password_once: None,
            },
            "admin-secret",
        );
        assert_eq!(updated.managed_ctx_bin, "~/.ctx/bin/ctx-managed");
        assert_eq!(updated.active_ctx_bin.as_deref(), Some("~/.ctx/bin/ctx-active"));
        assert_eq!(updated.admin_password_once.as_deref(), Some("admin-secret"));
    }
}
