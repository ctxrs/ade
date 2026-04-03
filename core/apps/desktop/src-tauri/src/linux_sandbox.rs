use super::*;

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

fn linux_sandbox_ready_marker(data_dir: &Path) -> PathBuf {
    linux_sandbox_root(data_dir).join("runtime-ready")
}

fn linux_sandbox_status_path(data_dir: &Path) -> PathBuf {
    linux_sandbox_root(data_dir).join("status.json")
}

fn linux_sandbox_script_path(data_dir: &Path) -> PathBuf {
    linux_sandbox_root(data_dir).join("bootstrap.sh")
}

fn remote_linux_sandbox_root(data_dir: &str) -> String {
    format!("{}/linux-sandbox-runtime", data_dir.trim_end_matches('/'))
}

fn remote_linux_sandbox_ready_marker_expr(data_dir: &str) -> String {
    remote_path_expr(&format!(
        "{}/runtime-ready",
        remote_linux_sandbox_root(data_dir)
    ))
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
    linux_sandbox_ready_marker(data_dir).exists()
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

pub(crate) fn remote_linux_sandbox_daemon_env_prefix(data_dir: &str) -> String {
    let marker = remote_linux_sandbox_ready_marker_expr(data_dir);
    format!(
        "if [ -f {marker} ]; then export CTX_HARNESS_SANDBOX_CLI_PATH={wrapper} CONTAINERD_ADDRESS={address} CONTAINERD_NAMESPACE={namespace}; fi;",
        marker = marker,
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

fn current_local_user() -> Result<String> {
    let value = std::env::var("USER").context("USER is not set")?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        anyhow::bail!("USER is empty");
    }
    Ok(trimmed.to_string())
}

fn run_local_activation(script_path: &Path, data_dir: &Path, allow_user: &str) -> Result<()> {
    let command = format!(
        "{} activate --data-dir {} --allow-user {}",
        shell_escape(script_path.to_string_lossy().as_ref()),
        shell_escape(data_dir.to_string_lossy().as_ref()),
        shell_escape(allow_user),
    );
    let output = if std::process::Command::new("pkexec")
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
    {
        Command::new("pkexec")
            .arg("/bin/sh")
            .arg("-lc")
            .arg(&command)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .context("running local Linux sandbox activation via pkexec")?
    } else {
        Command::new("sudo")
            .arg("/bin/sh")
            .arg("-lc")
            .arg(&command)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .context("running local Linux sandbox activation via sudo")?
    };
    if output.status.success() {
        return Ok(());
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
) -> Result<DesktopLinuxSandboxEnsureResp, String> {
    if !local_linux_platform_is_supported() {
        return Ok(DesktopLinuxSandboxEnsureResp { ready: true });
    }
    tauri::async_runtime::spawn_blocking(move || {
        let data_dir = daemon_data_dir(&app).map_err(to_err)?;
        let script_path = write_local_bootstrap_script(&data_dir).map_err(to_err)?;
        local_stage_spawn(app.clone());
        let status = wait_for_local_stage_completion(&data_dir).map_err(to_err)?;
        match status.state.as_str() {
            "ready" => Ok(DesktopLinuxSandboxEnsureResp { ready: true }),
            "downloaded_not_activated" => {
                let allow_user = current_local_user().map_err(to_err)?;
                run_local_activation(&script_path, &data_dir, &allow_user).map_err(to_err)?;
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

fn remote_stage_status(manager: &ConnectionManager) -> Result<LinuxSandboxBootstrapStatus> {
    parse_remote_daemon_json(
        manager.daemon_request(DesktopDaemonRequest {
            method: "POST".to_string(),
            path: "/api/execution/linux_sandbox_runtime/stage".to_string(),
            body: None,
            headers: vec![("Content-Type".to_string(), "application/json".to_string())],
        })?,
        "/api/execution/linux_sandbox_runtime/stage",
    )
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
        let mut target = manager.ssh_target().map_err(to_err)?;
        let (ssh_password_once, cached_admin_password) = active_remote_passwords(&target);
        let status = remote_stage_status(manager).map_err(to_err)?;
        match status.state.as_str() {
            "ready" => Ok(DesktopLinuxSandboxEnsureResp { ready: true }),
            "manual_runtime_required" => Err(if status.message.trim().is_empty() {
                "Preparing sandbox on remote host failed. Managed sandbox setup is currently supported on Ubuntu/Debian only.".to_string()
            } else {
                format!("Preparing sandbox on remote host failed. {}", status.message.trim())
            }),
            "downloaded_not_activated" => {
                let selected_password = cached_admin_password
                    .as_deref()
                    .or_else(|| ssh_password_once.as_deref())
                    .or(req.admin_password_once.as_deref());
                let prepare = remote_prepare_status(manager, selected_password).map_err(to_err)?;
                if prepare.ready {
                    if let Some(password) = selected_password {
                        target.runtime.admin_password_once = Some(password.to_string());
                        manager
                            .update_ssh_runtime(target.runtime.clone())
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
        assert!(prefix.contains("runtime-ready"));
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
}
