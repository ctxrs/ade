use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RemoteUpdateTargetDecision {
    pub(super) ctx_bin: String,
    pub(super) install_managed: bool,
}

pub(super) fn resolve_remote_update_target_ctx_bin(
    recorded_active_ctx_bin: Option<String>,
    managed_remote_ctx_bin: &str,
    recorded_active_exists: bool,
    managed_exists: bool,
) -> RemoteUpdateTargetDecision {
    match recorded_active_ctx_bin.filter(|value| !value.trim().is_empty()) {
        Some(active_ctx_bin) if recorded_active_exists => RemoteUpdateTargetDecision {
            ctx_bin: active_ctx_bin,
            install_managed: false,
        },
        _ if managed_exists => RemoteUpdateTargetDecision {
            ctx_bin: managed_remote_ctx_bin.to_string(),
            install_managed: false,
        },
        _ => RemoteUpdateTargetDecision {
            ctx_bin: managed_remote_ctx_bin.to_string(),
            install_managed: true,
        },
    }
}

#[tauri::command]
pub(crate) async fn desktop_update_remote_daemon(
    app: tauri::AppHandle,
    req: DesktopRemoteDaemonUpdateReq,
) -> Result<DesktopRemoteDaemonUpdateResp, String> {
    if !req.confirm {
        return Err("confirm required".to_string());
    }
    let channel = req.channel.clone();
    let app_for_update = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app_for_update.state::<ConnectionManager>();
        update_current_remote_daemon(&app_for_update, state.inner(), channel.as_deref())
    })
    .await
    .map_err(|err| format!("remote daemon update task failed: {err}"))?
    .map_err(to_err)
}

pub(crate) fn update_current_remote_daemon(
    app: &tauri::AppHandle,
    state: &ConnectionManager,
    requested_channel: Option<&str>,
) -> Result<DesktopRemoteDaemonUpdateResp> {
    let channel = normalize_update_channel(requested_channel).map_err(anyhow::Error::msg)?;
    let target = state.ssh_target()?;
    let managed_remote_ctx_bin = target.runtime.managed_ctx_bin.clone();
    let host = target.host;
    let user = target.user;
    let remote_port = target.remote_port;
    let remote_data_dir = target.remote_data_dir;
    let channel_for_update = channel.clone();
    let recorded_active_ctx_bin = target
        .runtime
        .active_ctx_bin
        .filter(|value| !value.trim().is_empty());

    let recorded_active_exists = recorded_active_ctx_bin
        .as_deref()
        .map(|active_ctx_bin| remote_ctx_bin_exists_over_ssh(&host, user.as_deref(), active_ctx_bin))
        .transpose()?
        .unwrap_or(false);
    let managed_exists = if recorded_active_ctx_bin.as_deref() == Some(&managed_remote_ctx_bin) {
        recorded_active_exists
    } else {
        remote_ctx_bin_exists_over_ssh(&host, user.as_deref(), &managed_remote_ctx_bin)?
    };
    let decision = resolve_remote_update_target_ctx_bin(
        recorded_active_ctx_bin,
        &managed_remote_ctx_bin,
        recorded_active_exists,
        managed_exists,
    );

    if decision.install_managed {
        let remote_platform = probe_remote_linux_platform(&host, user.as_deref())?;
        install_remote_daemon_over_ssh(
            app,
            &host,
            user.as_deref(),
            remote_platform,
            &managed_remote_ctx_bin,
        )
        .map_err(|install_err| install_err.context(REMOTE_BOOTSTRAP_CAPABILITY_MSG))?;
    }
    let active_ctx_bin = decision.ctx_bin;
    run_remote_daemon_self_update(
        app,
        &host,
        user.as_deref(),
        remote_port,
        remote_data_dir.as_deref(),
        &active_ctx_bin,
        &channel_for_update,
    )?;
    let auth = read_remote_daemon_auth_with_retry(&host, user.as_deref(), remote_data_dir.as_deref())?;
    state
        .update_ssh_token(auth.token)
        .map_err(anyhow::Error::msg)?;
    state
        .update_ssh_runtime(SshRuntimeMetadata {
            managed_ctx_bin: managed_remote_ctx_bin.clone(),
            active_ctx_bin: Some(active_ctx_bin),
            ssh_password_once: None,
            admin_password_once: None,
        })
        .map_err(anyhow::Error::msg)?;
    Ok(DesktopRemoteDaemonUpdateResp {
        updated: true,
        message: format!("Remote daemon updated on channel `{channel}` and restarted."),
    })
}

fn run_remote_daemon_self_update(
    app: &tauri::AppHandle,
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

    stop_remote_daemon_over_ssh(host, user, remote_port, &ctx_bin)
        .context("stopping remote daemon after self-update")?;
    sync_remote_bundle_metadata_over_ssh(app, host, user, remote_data_dir)
        .context("syncing remote bundle metadata after self-update")?;
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

pub(super) fn remote_stop_daemon_cmd(remote_port: u16, remote_ctx_bin: &str) -> String {
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
