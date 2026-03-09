use super::*;

struct ConnectedRemoteDaemon {
    base_url: String,
    token: String,
    tunnel: TunnelHandle,
    runtime: SshRuntimeMetadata,
}

struct BootstrapPlanContext {
    target: SshConnectTarget,
    platform: RemoteLinuxPlatform,
    decision: RemoteBootstrapPlan,
}

enum InitialConnectOutcome {
    Connected(ConnectedRemoteDaemon),
    Planned(BootstrapPlanContext),
}

pub(super) fn cleanup_ephemeral_tunnel_on_error<T>(
    tunnel: TunnelHandle,
    err: anyhow::Error,
) -> Result<T> {
    let _ = tunnel.kill();
    Err(err)
}

fn set_job_phase(job_id: Option<&str>, phase: ConnectJobPhase) {
    if let Some(job_id) = job_id {
        record_connect_job_phase(job_id, phase);
    }
}

fn normalize_connect_target(req: SshConnectReq) -> Result<SshConnectTarget, String> {
    let host = req.host.trim().to_string();
    if host.is_empty() {
        return Err("host is required".to_string());
    }
    Ok(SshConnectTarget {
        host,
        user: normalize_optional_text(req.user.as_deref()),
        password_once: normalize_optional_text(req.password_once.as_deref()),
        remote_port: req.remote_port.unwrap_or(4399),
        start_remote: req.start_remote,
        remote_data_dir: normalize_optional_text(req.remote_data_dir.as_deref()),
    })
}

fn prepare_initial_connect(
    target: SshConnectTarget,
    job_id: Option<String>,
) -> Result<InitialConnectOutcome> {
    set_job_phase(job_id.as_deref(), ConnectJobPhase::Probing);
    let (platform, auth_bootstrap_used) = probe_remote_linux_platform_with_optional_password(
        &target.host,
        target.user.as_deref(),
        target.password_once.as_deref(),
    )?;
    let managed_binary_present = remote_ctx_bin_exists_over_ssh(
        &target.host,
        target.user.as_deref(),
        MANAGED_REMOTE_CTX_BIN,
    )?;
    set_job_phase(job_id.as_deref(), ConnectJobPhase::OpeningTunnel);
    let local_port = pick_unused_local_port()?;
    let mut tunnel = TunnelHandle::start(
        &target.host,
        target.user.as_deref(),
        local_port,
        target.remote_port,
    )?;
    let base_url = tunnel.base_url();
    let no_start_remote = env_bool("CTX_DESKTOP_SSH_NO_START_REMOTE", false);
    let existing_daemon_reachable = if target.start_remote && !no_start_remote {
        tunnel.probe_health_quick_for_bootstrap(&base_url).is_ok()
    } else {
        tunnel.probe_health_with_retry(&base_url).is_ok()
    };
    let probe = RemoteProbe {
        platform,
        auth_bootstrap_used,
        managed_binary_present,
        existing_daemon_reachable,
    };
    set_job_phase(job_id.as_deref(), ConnectJobPhase::Planning);
    let decision = plan_remote_bootstrap(RemoteBootstrapPlannerInput {
        start_remote: target.start_remote,
        no_start_remote,
        existing_daemon_reachable: probe.existing_daemon_reachable,
        managed_binary_present: probe.managed_binary_present,
    });
    match decision {
        RemoteBootstrapPlan::ConnectToRunningDaemon => {
            set_job_phase(job_id.as_deref(), ConnectJobPhase::ReadingAuth);
            let auth = match read_remote_daemon_auth_with_retry(
                &target.host,
                target.user.as_deref(),
                target.remote_data_dir.as_deref(),
            ) {
                Ok(auth) => auth,
                Err(err) => return cleanup_ephemeral_tunnel_on_error(tunnel, err),
            };
            let active_ctx_bin = probe
                .managed_binary_present
                .then(|| MANAGED_REMOTE_CTX_BIN.to_string());
            Ok(InitialConnectOutcome::Connected(ConnectedRemoteDaemon {
                base_url,
                token: auth.token,
                tunnel,
                runtime: SshRuntimeMetadata {
                    managed_ctx_bin: MANAGED_REMOTE_CTX_BIN.to_string(),
                    active_ctx_bin,
                },
            }))
        }
        RemoteBootstrapPlan::RefuseBecauseStartRemoteDisabled => {
            let _ = tunnel.kill();
            Err(anyhow!(
                "failed to reach remote daemon: remote start skipped (start_remote={}, no_start_remote={})",
                target.start_remote,
                no_start_remote
            ))
        }
        _ => {
            let _ = tunnel.kill();
            Ok(InitialConnectOutcome::Planned(BootstrapPlanContext {
                target,
                platform: probe.platform,
                decision,
            }))
        }
    }
}

fn execute_bootstrap_plan(
    app: tauri::AppHandle,
    plan: BootstrapPlanContext,
    job_id: Option<String>,
) -> Result<ConnectedRemoteDaemon> {
    if matches!(
        plan.decision,
        RemoteBootstrapPlan::InstallManagedDaemonThenStart
    ) {
        set_job_phase(job_id.as_deref(), ConnectJobPhase::InstallingManagedDaemon);
        install_remote_daemon_over_ssh(
            &app,
            &plan.target.host,
            plan.target.user.as_deref(),
            plan.platform,
            MANAGED_REMOTE_CTX_BIN,
        )
        .map_err(|install_err| install_err.context(REMOTE_BOOTSTRAP_CAPABILITY_MSG))?;
    }

    set_job_phase(job_id.as_deref(), ConnectJobPhase::StartingRemoteDaemon);
    start_remote_daemon_over_ssh(
        &plan.target.host,
        plan.target.user.as_deref(),
        plan.target.remote_port,
        plan.target.remote_data_dir.as_deref(),
        MANAGED_REMOTE_CTX_BIN,
    )?;

    set_job_phase(job_id.as_deref(), ConnectJobPhase::OpeningTunnel);
    let local_port = pick_unused_local_port()?;
    let mut tunnel = TunnelHandle::start(
        &plan.target.host,
        plan.target.user.as_deref(),
        local_port,
        plan.target.remote_port,
    )?;
    let base_url = tunnel.base_url();
    if let Err(err) = tunnel.probe_health_with_retry(&base_url) {
        return cleanup_ephemeral_tunnel_on_error(tunnel, err);
    }
    set_job_phase(job_id.as_deref(), ConnectJobPhase::ReadingAuth);
    let auth = match read_remote_daemon_auth_with_retry(
        &plan.target.host,
        plan.target.user.as_deref(),
        plan.target.remote_data_dir.as_deref(),
    ) {
        Ok(auth) => auth,
        Err(err) => return cleanup_ephemeral_tunnel_on_error(tunnel, err),
    };
    Ok(ConnectedRemoteDaemon {
        base_url,
        token: auth.token,
        tunnel,
        runtime: SshRuntimeMetadata {
            managed_ctx_bin: MANAGED_REMOTE_CTX_BIN.to_string(),
            active_ctx_bin: Some(MANAGED_REMOTE_CTX_BIN.to_string()),
        },
    })
}

async fn desktop_connect_ssh_inner(
    app: tauri::AppHandle,
    req: SshConnectReq,
    job_id: Option<String>,
) -> Result<DesktopConnectionInfo, String> {
    {
        let state = app.state::<ConnectionManager>();
        state.disconnect();
    }

    let target = normalize_connect_target(req)?;
    let channel = normalize_update_channel(std::env::var("CTX_DESKTOP_CHANNEL").ok().as_deref())?;
    let prepared = tauri::async_runtime::spawn_blocking({
        let target = target.clone();
        let job_id = job_id.clone();
        move || prepare_initial_connect(target, job_id)
    })
    .await
    .map_err(|e| format!("failed to reach remote daemon: {e}"))?
    .map_err(|e| format!("failed to reach remote daemon: {e:#}"))?;

    let connected = match prepared {
        InitialConnectOutcome::Connected(connected) => connected,
        InitialConnectOutcome::Planned(plan) => {
            desktop_updater::ensure_desktop_app_current_for_remote_bootstrap(&app, &channel)
                .await?;
            tauri::async_runtime::spawn_blocking({
                let app = app.clone();
                let job_id = job_id.clone();
                move || execute_bootstrap_plan(app, plan, job_id)
            })
            .await
            .map_err(|e| format!("failed to reach remote daemon: {e}"))?
            .map_err(|e| format!("failed to reach remote daemon: {e:#}"))?
        }
    };

    set_job_phase(job_id.as_deref(), ConnectJobPhase::HandingOffConnection);
    let state = app.state::<ConnectionManager>();
    state.set_ssh(
        connected.base_url,
        Some(connected.token),
        connected
            .tunnel
            .into_connection_child()
            .map_err(|err| format!("failed to hand off ssh tunnel: {err:#}"))?,
        target.host,
        target.user,
        target.remote_port,
        target.remote_data_dir,
        connected.runtime,
    );
    Ok(state.info())
}

#[tauri::command]
pub(crate) async fn desktop_connect_ssh(
    app: tauri::AppHandle,
    req: SshConnectReq,
) -> Result<DesktopConnectionInfo, String> {
    desktop_connect_ssh_inner(app, req, None).await
}

#[tauri::command]
pub(crate) fn desktop_connect_ssh_begin(
    app: tauri::AppHandle,
    req: SshConnectReq,
) -> Result<String, String> {
    let job_id = begin_connect_job()?;
    let app_for_job = app.clone();
    let job_id_for_task = job_id.clone();
    tauri::async_runtime::spawn(async move {
        let result =
            desktop_connect_ssh_inner(app_for_job, req, Some(job_id_for_task.clone())).await;
        match result {
            Ok(info) => complete_connect_job_success(&job_id_for_task, info),
            Err(err) => complete_connect_job_failure(&job_id_for_task, err),
        }
    });
    Ok(job_id)
}
