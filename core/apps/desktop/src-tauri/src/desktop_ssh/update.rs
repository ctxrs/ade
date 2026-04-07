use super::*;

const REMOTE_UPDATE_HEALTH_RETRIES: usize = 24;
const REMOTE_UPDATE_HEALTH_DELAY_MS: u64 = 500;

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
    let base_url = state
        .info()
        .base_url
        .ok_or_else(|| anyhow!("current SSH connection is missing a base_url"))?;
    let recorded_active_ctx_bin = target
        .runtime
        .active_ctx_bin
        .filter(|value| !value.trim().is_empty());

    let recorded_active_exists = recorded_active_ctx_bin
        .as_deref()
        .map(|active_ctx_bin| {
            remote_ctx_bin_exists_over_ssh(&host, user.as_deref(), active_ctx_bin)
        })
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
        &base_url,
    )?;
    let auth =
        read_remote_daemon_auth_with_retry(&host, user.as_deref(), remote_data_dir.as_deref())?;
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
    base_url: &str,
) -> Result<()> {
    let ctx_bin = validate_remote_ctx_bin(remote_ctx_bin)?;
    let backup_ctx_bin = remote_update_backup_ctx_bin(&ctx_bin)?;
    backup_remote_ctx_bin_over_ssh(host, user, &ctx_bin, &backup_ctx_bin)
        .context("backing up remote daemon binary before self-update")?;
    let update_cmd = format!(
        "if [ -x {ctx_bin} ]; then {ctx_bin} self-update --yes --channel {channel}; else echo 'ctx not executable at configured remote path' >&2; exit 127; fi",
        ctx_bin = remote_path_expr(&ctx_bin),
        channel = shell_escape(channel),
    );
    let mut daemon_stopped = false;
    let update_result = (|| {
        let output =
            run_remote_ssh_shell(host, user, &update_cmd).context("running remote self-update")?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let detail = if !stderr.is_empty() { stderr } else { stdout };
            anyhow::bail!("remote self-update failed: {detail}");
        }

        sync_remote_bundle_metadata_over_ssh(app, host, user, remote_data_dir)
            .context("syncing remote bundle metadata before daemon restart")?;
        stop_remote_daemon_over_ssh(host, user, remote_port, remote_data_dir, &ctx_bin)
            .context("stopping remote daemon after self-update")?;
        daemon_stopped = true;
        start_remote_daemon_over_ssh(host, user, remote_port, remote_data_dir, &ctx_bin)
            .context("starting remote daemon after self-update")?;
        wait_for_remote_daemon_health(base_url)
            .context("waiting for restarted remote daemon health")?;
        Ok(())
    })();

    match update_result {
        Ok(()) => {
            if let Err(err) = cleanup_remote_update_backup_over_ssh(host, user, &backup_ctx_bin) {
                eprintln!("failed to remove remote update backup {backup_ctx_bin}: {err:#}");
            }
            Ok(())
        }
        Err(err) if daemon_stopped => {
            match rollback_remote_daemon_update_over_ssh(
                host,
                user,
                remote_port,
                remote_data_dir,
                &ctx_bin,
                &backup_ctx_bin,
                base_url,
            ) {
                Ok(()) => Err(anyhow!(
                    "{err:#}; restored the previous remote daemon binary and restarted it"
                )),
                Err(rollback_err) => Err(anyhow!("{err:#}; rollback failed: {rollback_err:#}")),
            }
        }
        Err(err) => {
            if let Err(restore_err) =
                restore_remote_ctx_bin_over_ssh(host, user, &ctx_bin, &backup_ctx_bin)
            {
                return Err(anyhow!(
                    "{err:#}; failed to restore pre-update daemon binary while the old daemon was still running: {restore_err:#}"
                ));
            }
            if let Err(cleanup_err) =
                cleanup_remote_update_backup_over_ssh(host, user, &backup_ctx_bin)
            {
                eprintln!(
                    "failed to remove remote update backup {backup_ctx_bin}: {cleanup_err:#}"
                );
            }
            Err(err)
        }
    }
}

fn stop_remote_daemon_over_ssh(
    host: &str,
    user: Option<&str>,
    remote_port: u16,
    remote_data_dir: Option<&str>,
    remote_ctx_bin: &str,
) -> Result<()> {
    let cmd = remote_stop_daemon_cmd(remote_port, remote_data_dir, remote_ctx_bin);
    let output = run_remote_ssh_shell(host, user, &cmd).context("stopping remote daemon")?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let detail = if !stderr.is_empty() { stderr } else { stdout };
    anyhow::bail!("remote stop command failed: {detail}");
}

pub(super) fn remote_stop_daemon_cmd(
    remote_port: u16,
    remote_data_dir: Option<&str>,
    remote_ctx_bin: &str,
) -> String {
    let data_dir = remote_data_dir
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("~/.ctx");
    format!(
        "if ! command -v lsof >/dev/null 2>&1; then echo 'lsof unavailable on remote host' >&2; exit 127; fi; \
ctx_bin={ctx_bin}; \
data_dir={data_dir}; \
expected_cmd=\"$ctx_bin serve --bind 127.0.0.1:{port} --data-dir $data_dir\"; \
set -- $(lsof -tiTCP:{port} -sTCP:LISTEN 2>/dev/null); \
if [ $# -eq 0 ]; then echo \"remote daemon stop failed (no listener on port {port})\" >&2; exit 1; fi; \
if [ $# -ne 1 ]; then echo \"remote daemon stop failed (expected exactly one listener on port {port}, found $#)\" >&2; exit 1; fi; \
pid=\"$1\"; \
cmdline=\"$(ps -p \"$pid\" -o args= 2>/dev/null || true)\"; \
if [ -z \"$cmdline\" ]; then echo \"remote daemon stop failed (unable to inspect pid $pid on port {port})\" >&2; exit 1; fi; \
case \"$cmdline\" in \
  *\"$expected_cmd\"*) ;; \
  *) echo \"remote daemon stop refused for pid $pid on port {port}: $cmdline\" >&2; exit 1 ;; \
esac; \
kill \"$pid\" >/dev/null 2>&1 || {{ echo \"remote daemon stop failed (kill pid $pid on port {port})\" >&2; exit 1; }}; \
sleep 1",
        port = remote_port,
        ctx_bin = remote_path_expr(remote_ctx_bin),
        data_dir = remote_path_expr(data_dir),
    )
}

pub(super) fn remote_update_backup_ctx_bin(remote_ctx_bin: &str) -> Result<String> {
    let ctx_bin = validate_remote_ctx_bin(remote_ctx_bin)?;
    Ok(format!("{ctx_bin}.pre-update-backup"))
}

pub(super) fn remote_backup_ctx_bin_cmd(remote_ctx_bin: &str, backup_ctx_bin: &str) -> String {
    format!(
        "if [ -x {ctx_bin} ]; then cp {ctx_bin} {backup} && chmod 755 {backup}; else echo 'ctx not executable at configured remote path' >&2; exit 127; fi",
        ctx_bin = remote_path_expr(remote_ctx_bin),
        backup = remote_path_expr(backup_ctx_bin),
    )
}

pub(super) fn remote_restore_ctx_bin_cmd(remote_ctx_bin: &str, backup_ctx_bin: &str) -> String {
    format!(
        "if [ -x {backup} ]; then cp {backup} {ctx_bin} && chmod 755 {ctx_bin}; else echo 'remote daemon backup missing after failed update' >&2; exit 127; fi",
        ctx_bin = remote_path_expr(remote_ctx_bin),
        backup = remote_path_expr(backup_ctx_bin),
    )
}

pub(super) fn remote_cleanup_backup_ctx_bin_cmd(backup_ctx_bin: &str) -> String {
    format!("rm -f {}", remote_path_expr(backup_ctx_bin))
}

fn backup_remote_ctx_bin_over_ssh(
    host: &str,
    user: Option<&str>,
    remote_ctx_bin: &str,
    backup_ctx_bin: &str,
) -> Result<()> {
    let cmd = remote_backup_ctx_bin_cmd(remote_ctx_bin, backup_ctx_bin);
    let output =
        run_remote_ssh_shell(host, user, &cmd).context("backing up remote daemon binary")?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let detail = if !stderr.is_empty() { stderr } else { stdout };
    anyhow::bail!("remote daemon backup failed: {detail}");
}

fn restore_remote_ctx_bin_over_ssh(
    host: &str,
    user: Option<&str>,
    remote_ctx_bin: &str,
    backup_ctx_bin: &str,
) -> Result<()> {
    let cmd = remote_restore_ctx_bin_cmd(remote_ctx_bin, backup_ctx_bin);
    let output =
        run_remote_ssh_shell(host, user, &cmd).context("restoring remote daemon binary")?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let detail = if !stderr.is_empty() { stderr } else { stdout };
    anyhow::bail!("remote daemon restore failed: {detail}");
}

fn cleanup_remote_update_backup_over_ssh(
    host: &str,
    user: Option<&str>,
    backup_ctx_bin: &str,
) -> Result<()> {
    let cmd = remote_cleanup_backup_ctx_bin_cmd(backup_ctx_bin);
    let output =
        run_remote_ssh_shell(host, user, &cmd).context("cleaning up remote daemon backup")?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let detail = if !stderr.is_empty() { stderr } else { stdout };
    anyhow::bail!("remote daemon backup cleanup failed: {detail}");
}

fn rollback_remote_daemon_update_over_ssh(
    host: &str,
    user: Option<&str>,
    remote_port: u16,
    remote_data_dir: Option<&str>,
    remote_ctx_bin: &str,
    backup_ctx_bin: &str,
    base_url: &str,
) -> Result<()> {
    let _ = stop_remote_daemon_over_ssh(host, user, remote_port, remote_data_dir, remote_ctx_bin);
    restore_remote_ctx_bin_over_ssh(host, user, remote_ctx_bin, backup_ctx_bin)
        .context("restoring pre-update remote daemon binary")?;
    start_remote_daemon_over_ssh(host, user, remote_port, remote_data_dir, remote_ctx_bin)
        .context("restarting previous remote daemon binary after rollback")?;
    wait_for_remote_daemon_health(base_url)
        .context("waiting for rolled back remote daemon health")?;
    if let Err(err) = cleanup_remote_update_backup_over_ssh(host, user, backup_ctx_bin) {
        eprintln!("failed to remove remote update backup {backup_ctx_bin}: {err:#}");
    }
    Ok(())
}

fn wait_for_remote_daemon_health(base_url: &str) -> Result<()> {
    let mut last_err: Option<anyhow::Error> = None;
    for attempt in 0..REMOTE_UPDATE_HEALTH_RETRIES {
        match probe_daemon_health(base_url) {
            Ok(()) => return Ok(()),
            Err(err) => last_err = Some(err),
        }
        if attempt + 1 < REMOTE_UPDATE_HEALTH_RETRIES {
            std::thread::sleep(std::time::Duration::from_millis(
                REMOTE_UPDATE_HEALTH_DELAY_MS,
            ));
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow!("requesting /api/health failed")))
}
