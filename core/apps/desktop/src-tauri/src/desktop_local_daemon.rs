use super::*;
use crate::desktop_daemon::{
    daemon_data_dir, daemon_health, existing_local_daemon_matches,
    existing_local_daemon_matches_or_absent, normalize_daemon_pid, probe_daemon_health,
    probe_local_daemon_health_with_retry, read_daemon_auth_with_retry, resolve_env_local_daemon,
    resolve_existing_local_daemon, spawn_and_validate_local_daemon, SpawnedLocalDaemonReady,
};
pub(super) use ctx_desktop_ipc::DesktopRestartLocalDaemonReq;

fn local_connect_mutex() -> &'static std::sync::Mutex<()> {
    static LOCAL_CONNECT_MUTEX: std::sync::OnceLock<std::sync::Mutex<()>> =
        std::sync::OnceLock::new();
    LOCAL_CONNECT_MUTEX.get_or_init(|| std::sync::Mutex::new(()))
}

pub(super) fn lock_local_connect_gate() -> Result<std::sync::MutexGuard<'static, ()>> {
    local_connect_mutex()
        .lock()
        .map_err(|err| anyhow!("local connect mutex poisoned: {err}"))
}

#[tauri::command]
pub(super) async fn desktop_connect_local(
    app: tauri::AppHandle,
) -> Result<DesktopConnectionInfo, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = lock_local_connect_gate().map_err(to_err)?;
        let state = app.state::<ConnectionManager>();
        let data_dir = daemon_data_dir(&app).map_err(to_err)?;
        let desktop_identity = load_desktop_build_identity(&app).map_err(to_err)?;
        let result = connect_local_with_sources(
            state.inner(),
            |url| {
                existing_local_daemon_matches_or_absent(
                    url,
                    &data_dir,
                    &desktop_identity,
                )
            },
            || resolve_env_local_daemon(&app),
            probe_daemon_health,
            || resolve_existing_local_daemon(&app, &data_dir),
            || spawn_and_validate_local_daemon(&app, &data_dir, &desktop_identity),
        );
        if let Err(err) = &result {
            log_desktop_startup_error(&format!(
                "desktop_startup: daemon_connect_failed kind=local error={}",
                serde_json::to_string(&err.to_string())
                    .unwrap_or_else(|_| "\"unknown\"".to_string()),
            ));
        }
        result.map_err(to_err)
    })
    .await
    .map_err(|e| format!("failed to connect to daemon: {e}"))?
}

pub(super) fn connect_local_with_sources<
    CurrentLocalMatchesFn,
    ResolveEnvFn,
    ProbeHealthFn,
    ResolveExistingFn,
    SpawnFn,
>(
    state: &ConnectionManager,
    current_local_matches_or_absent: CurrentLocalMatchesFn,
    resolve_env_local_daemon: ResolveEnvFn,
    probe_health: ProbeHealthFn,
    mut resolve_existing_local_daemon: ResolveExistingFn,
    spawn_and_validate_local_daemon: SpawnFn,
) -> Result<DesktopConnectionInfo>
where
    CurrentLocalMatchesFn: Fn(&str) -> bool,
    ResolveEnvFn: FnOnce() -> Result<Option<(String, String)>>,
    ProbeHealthFn: Fn(&str) -> Result<()>,
    ResolveExistingFn: FnMut() -> Result<Option<(String, String, Option<u32>)>>,
    SpawnFn: FnOnce() -> Result<SpawnedLocalDaemonReady>,
{
    // Idempotent: if we're already connected to a healthy local daemon, keep the connection.
    // The workspace wizard calls connect_local as part of its flow; disconnecting here can
    // kill a just-started daemon and introduce flakiness on cold start.
    let info = state.info();
    if matches!(info.kind, DesktopConnectionKind::Local) {
        if let Some(url) = info.base_url.as_deref() {
            if current_local_matches_or_absent(url) {
                return Ok(info);
            }
        }
    }

    // Keep any currently healthy connection active until a replacement has been validated.
    // ConnectionManager swaps and cleans up the old transport only after the new one is ready.
    if let Some((url, token)) = resolve_env_local_daemon()? {
        probe_health(&url)?;
        state.set_local_attached(url, token, None, LocalConnectionSource::EnvOverride);
        return Ok(state.info());
    }
    if let Some((url, token, daemon_pid)) = resolve_existing_local_daemon()? {
        state.set_local_attached(
            url,
            token,
            daemon_pid,
            LocalConnectionSource::ExistingCompatibleDaemon,
        );
        return Ok(state.info());
    }

    // Block until the daemon is actually reachable before returning. The workspace wizard
    // applies the connection and navigates immediately after `desktop_connect_local` resolves;
    // returning early causes the workbench to briefly render a "daemon unavailable" overlay.
    let spawned = spawn_and_validate_local_daemon();
    let fallback_existing = if spawned.is_err() {
        resolve_existing_local_daemon().ok().flatten()
    } else {
        None
    };
    apply_validated_local_connection(state, spawned, fallback_existing)
}

pub(super) fn apply_validated_local_connection(
    state: &ConnectionManager,
    spawned: Result<SpawnedLocalDaemonReady>,
    fallback_existing: Option<(String, String, Option<u32>)>,
) -> Result<DesktopConnectionInfo> {
    match spawned {
        Ok(spawned) => {
            state.set_local(
                spawned.url,
                spawned.token,
                spawned.child,
                spawned.systemd_scope,
            );
            Ok(state.info())
        }
        Err(err) => {
            if let Some((url, token, daemon_pid)) = fallback_existing {
                state.set_local_attached(
                    url,
                    token,
                    daemon_pid,
                    LocalConnectionSource::ExistingCompatibleDaemon,
                );
                return Ok(state.info());
            }
            Err(err)
        }
    }
}

pub(super) fn restart_local_with_spawn<SpawnFn>(
    state: &ConnectionManager,
    spawn_and_validate_local_daemon: SpawnFn,
) -> Result<DesktopConnectionInfo>
where
    SpawnFn: FnOnce() -> Result<SpawnedLocalDaemonReady>,
{
    if state.should_disconnect_for_local_restart() {
        state.disconnect_for_local_restart()?;
    }
    let spawned = spawn_and_validate_local_daemon()?;
    state.set_local(
        spawned.url,
        spawned.token,
        spawned.child,
        spawned.systemd_scope,
    );
    Ok(state.info())
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
        let _guard = lock_local_connect_gate().map_err(to_err)?;
        let state = app.state::<ConnectionManager>();
        let manager: &ConnectionManager = state.inner();
        let data_dir = daemon_data_dir(&app).map_err(to_err)?;
        let desktop_identity = load_desktop_build_identity(&app).map_err(to_err)?;
        restart_local_with_spawn(manager, || spawn_and_validate_local_daemon(&app, &data_dir, &desktop_identity))
        .map_err(to_err)
    })
    .await
    .map_err(|e| format!("failed to restart local daemon: {e}"))?
}

pub(super) fn ensure_local_connection(
    app: &tauri::AppHandle,
    state: &ConnectionManager,
) -> Result<()> {
    let result = (|| -> Result<()> {
        if !matches!(state.info().kind, DesktopConnectionKind::None) {
            return Ok(());
        }
        // Multiple webview requests can race on cold start (overlay pollers, initial data loads, etc.).
        // Serialize the "connect local" path so we don't concurrently spawn the daemon and trip the
        // daemon's lockfile, which can surface as spurious "daemon unavailable" errors in the UI.
        let _guard = lock_local_connect_gate()?;
        if !matches!(state.info().kind, DesktopConnectionKind::None) {
            return Ok(());
        }
        let data_dir = daemon_data_dir(app)?;
        let desktop_identity = load_desktop_build_identity(app)?;
        if let Some((url, token)) = resolve_env_local_daemon(app)? {
            probe_daemon_health(&url)?;
            state.set_local_attached(url, token, None, LocalConnectionSource::EnvOverride);
            return Ok(());
        }
        if let Some((url, token, daemon_pid)) = resolve_existing_local_daemon(app, &data_dir)? {
            state.set_local_attached(
                url,
                token,
                daemon_pid,
                LocalConnectionSource::ExistingCompatibleDaemon,
            );
            return Ok(());
        }
        let spawned = match spawn_and_validate_local_daemon(app, &data_dir, &desktop_identity) {
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
                    &desktop_identity,
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
                let daemon_pid = daemon_health(url)
                    .ok()
                    .and_then(|health| normalize_daemon_pid(health.pid));
                state.set_local_attached(
                    url.to_string(),
                    auth.token,
                    daemon_pid,
                    LocalConnectionSource::ExistingCompatibleDaemon,
                );
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
    })();
    if let Err(err) = &result {
        log_desktop_startup_error(&format!(
            "desktop_startup: daemon_connect_failed kind=local error={}",
            serde_json::to_string(&err.to_string()).unwrap_or_else(|_| "\"unknown\"".to_string()),
        ));
    }
    result
}

#[cfg(test)]
mod desktop_local_daemon_tests {
    use super::*;

    #[cfg(unix)]
    fn spawn_detached_sleep_pid() -> u32 {
        let output = Command::new("sh")
            .arg("-c")
            .arg("sleep 30 >/dev/null 2>&1 & echo $!")
            .output()
            .expect("spawn detached sleep");
        assert!(
            output.status.success(),
            "detached sleep spawn failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse::<u32>()
            .expect("parse detached sleep pid")
    }

    #[cfg(unix)]
    fn pid_is_alive(pid: u32) -> bool {
        Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    #[cfg(unix)]
    fn wait_for_pid_exit(pid: u32, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if !pid_is_alive(pid) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(80));
        }
        !pid_is_alive(pid)
    }

    #[cfg(unix)]
    fn spawn_tokio_sleep_child() -> Child {
        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg("sleep 30 >/dev/null 2>&1")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        command.spawn().expect("spawn tokio sleep child")
    }

    #[test]
    #[cfg(unix)]
    fn desktop_restart_local_daemon_spawn_failure_preserves_env_override_local_connection() {
        let state = ConnectionManager::default();
        let daemon_pid = spawn_detached_sleep_pid();
        assert!(
            pid_is_alive(daemon_pid),
            "attached daemon should start alive before restart attempt"
        );
        state.set_local_attached(
            "http://127.0.0.1:4315".to_string(),
            "attached-token".to_string(),
            Some(daemon_pid),
            LocalConnectionSource::EnvOverride,
        );

        let err = restart_local_with_spawn(&state, || Err(anyhow!("daemon lockfile is held")))
            .expect_err("spawn failure should not disconnect an env override local daemon");
        assert!(format!("{err:#}").contains("daemon lockfile is held"));

        let info = state.info();
        assert!(matches!(info.kind, DesktopConnectionKind::Local));
        assert_eq!(info.base_url.as_deref(), Some("http://127.0.0.1:4315"));
        assert_eq!(info.token.as_deref(), Some("attached-token"));
        assert!(
            pid_is_alive(daemon_pid),
            "env override daemon pid {daemon_pid} must remain alive after restart failure"
        );

        let _ = Command::new("kill")
            .arg("-KILL")
            .arg(daemon_pid.to_string())
            .output();
    }

    #[test]
    #[cfg(unix)]
    fn desktop_restart_local_daemon_restarts_reattached_compatible_daemon() {
        let state = ConnectionManager::default();
        let previous_pid = spawn_detached_sleep_pid();
        assert!(
            pid_is_alive(previous_pid),
            "reattached compatible daemon should start alive before restart"
        );
        state.set_local_attached(
            "http://127.0.0.1:4316".to_string(),
            "compatible-token".to_string(),
            Some(previous_pid),
            LocalConnectionSource::ExistingCompatibleDaemon,
        );

        let replacement = spawn_tokio_sleep_child();
        let replacement_pid = replacement.id();
        assert!(
            pid_is_alive(replacement_pid),
            "replacement child should start alive before installation"
        );

        let info = restart_local_with_spawn(&state, || {
            if pid_is_alive(previous_pid) {
                anyhow::bail!(
                    "reattached compatible daemon pid {previous_pid} was still alive when replacement spawn began"
                );
            }
            Ok(SpawnedLocalDaemonReady {
                url: "http://127.0.0.1:4317".to_string(),
                token: "replacement-token".to_string(),
                child: replacement,
                systemd_scope: false,
            })
        })
        .expect("restart should replace a reattached compatible local daemon");

        assert!(matches!(info.kind, DesktopConnectionKind::Local));
        assert_eq!(info.base_url.as_deref(), Some("http://127.0.0.1:4317"));
        assert_eq!(info.token.as_deref(), Some("replacement-token"));
        assert!(
            pid_is_alive(replacement_pid),
            "replacement child {replacement_pid} should remain active after restart"
        );

        state.disconnect();
        assert!(
            wait_for_pid_exit(replacement_pid, Duration::from_secs(3)),
            "replacement child {replacement_pid} should terminate on disconnect"
        );
    }
}
