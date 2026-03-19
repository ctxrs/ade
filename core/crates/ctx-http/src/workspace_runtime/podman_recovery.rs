use super::*;
use std::collections::HashMap;
use std::process::{Command as StdCommand, Stdio};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};

mod helper_cleanup;

use self::helper_cleanup::cleanup_ctx_managed_podman_helper_processes;
#[cfg(test)]
pub(in crate::workspace_runtime) use self::helper_cleanup::collect_ctx_managed_podman_helper_pids;
#[cfg(test)]
pub(in crate::workspace_runtime) use self::helper_cleanup::collect_ctx_managed_podman_helper_pids_from_ps_output;
#[cfg(test)]
#[allow(unused_imports)]
pub(in crate::workspace_runtime) use self::helper_cleanup::{
    is_ctx_managed_podman_helper_process_command, kill_ctx_managed_podman_helper_processes,
    literal_pkill_pattern,
};

use ctx_store::Store;
use tokio::io::AsyncReadExt;
use tokio::sync::Mutex;

static PODMAN_MACHINE_SINGLEFLIGHT_LOCKS: OnceLock<StdMutex<HashMap<String, Arc<Mutex<()>>>>> =
    OnceLock::new();

pub(super) fn podman_machine_singleflight_lock(machine_name: &str) -> Arc<Mutex<()>> {
    let registry = PODMAN_MACHINE_SINGLEFLIGHT_LOCKS.get_or_init(|| StdMutex::new(HashMap::new()));
    let mut guard = match registry.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    guard
        .entry(machine_name.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

fn podman_machine_heartbeat_interval() -> Duration {
    if cfg!(test) {
        Duration::from_millis(100)
    } else {
        Duration::from_secs(5)
    }
}

async fn configured_podman_machine_memory_mb(
    data_root: &Path,
    observer: Option<&dyn HarnessSetupObserver>,
) -> u32 {
    let default_memory_mb = container_machine_memory_mb(&ContainerExecutionSettings::default());
    let db_path = data_root.join("db").join("db.sqlite");
    let store = match Store::open_sqlite(&db_path, None).await {
        Ok(store) => store,
        Err(err) => {
            observe_log(
                observer,
                HarnessSetupPhase::MachineCheck,
                HarnessSetupLogLevel::Warn,
                &format!(
                    "failed to open execution settings while recovering local sandbox runtime; using default machine memory: {err}"
                ),
            );
            return default_memory_mb;
        }
    };

    let loaded = crate::settings::load_settings(&store).await;
    store.close().await;

    match loaded {
        Ok(settings) => {
            container_machine_memory_mb(&settings.execution.unwrap_or_default().container)
        }
        Err(err) => {
            observe_log(
                observer,
                HarnessSetupPhase::MachineCheck,
                HarnessSetupLogLevel::Warn,
                &format!(
                    "failed to load execution settings while recovering local sandbox runtime; using default machine memory: {err:#}"
                ),
            );
            default_memory_mb
        }
    }
}

pub(super) async fn podman_machine_present(data_root: &Path, machine_name: &str) -> Result<bool> {
    let mut cmd = podman_command(data_root)?;
    cmd.arg("machine").arg("inspect").arg(machine_name);
    let output = command_output_with_timeout(cmd, PODMAN_INFO_TIMEOUT).await?;
    Ok(output.status.success())
}

pub(super) fn looks_like_missing_machine_error(message_lc: &str) -> bool {
    message_lc.contains("no such")
        || message_lc.contains("not found")
        || message_lc.contains("does not exist")
        || message_lc.contains("no machine")
}

pub(super) fn looks_like_recoverable_machine_start_error(message_lc: &str) -> bool {
    message_lc.contains("already running")
        || message_lc.contains("already starting")
        || message_lc.contains("already started")
        || message_lc.contains("in progress")
        || message_lc.contains("timed out")
        || message_lc.contains("resource busy")
        || message_lc.contains("another process")
        || message_lc.contains("lock")
        || message_lc.contains("port conflict")
        || message_lc.contains("unable to connect to \"gvproxy\" socket")
        || message_lc.contains("reassigning")
        || message_lc.contains("exited unexpectedly")
        || message_lc.contains("address already in use")
}

pub(super) fn looks_like_running_but_unreachable_machine_start_error(message_lc: &str) -> bool {
    message_lc.contains("already running")
        || message_lc.contains("already started")
        || message_lc.contains("unable to connect to \"gvproxy\" socket")
}

pub(super) fn podman_machine_temp_state_paths(
    data_root: &Path,
    machine_name: &str,
) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let tmp_root = podman_temp_root(data_root).join("podman");
    paths.push(tmp_root.join("gvproxy.pid"));
    paths.push(tmp_root.join(format!("{machine_name}-api.sock")));
    paths.push(tmp_root.join(format!("{machine_name}-gvproxy.sock")));
    paths.push(tmp_root.join(format!("{machine_name}.sock")));
    let home_root = podman_home_root(data_root).join(".podman");
    paths.push(home_root.join(format!("{machine_name}-api.sock")));
    paths.push(home_root.join(format!("{machine_name}-gvproxy.sock")));
    paths
}

fn clear_stale_podman_machine_temp_state(
    data_root: &Path,
    machine_name: &str,
    observer: Option<&dyn HarnessSetupObserver>,
) {
    for path in podman_machine_temp_state_paths(data_root, machine_name) {
        if path.exists() {
            match std::fs::remove_file(&path) {
                Ok(()) => observe_log(
                    observer,
                    HarnessSetupPhase::MachineStartOrInit,
                    HarnessSetupLogLevel::Warn,
                    &format!("removed stale podman temp state {}", path.display()),
                ),
                Err(err) => observe_log(
                    observer,
                    HarnessSetupPhase::MachineStartOrInit,
                    HarnessSetupLogLevel::Warn,
                    &format!(
                        "failed to remove stale podman temp state {}: {err}",
                        path.display()
                    ),
                ),
            }
        }
    }
}

async fn best_effort_start_machine_after_init(
    data_root: &Path,
    machine_name: &str,
    observer: Option<&dyn HarnessSetupObserver>,
    last_err: &mut String,
) -> Result<()> {
    let mut start = podman_command(data_root)?;
    start.arg("machine").arg("start").arg(machine_name);
    match command_output_with_timeout(start, PODMAN_MACHINE_START_TIMEOUT).await {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => {
            let combined = command_output_message(&out);
            if !combined.is_empty() {
                *last_err = combined.clone();
            }
            observe_log(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                HarnessSetupLogLevel::Warn,
                &format!("podman machine start after init returned non-zero: {combined}"),
            );
            Ok(())
        }
        Err(err) => {
            *last_err = err.to_string();
            observe_log(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                HarnessSetupLogLevel::Warn,
                &format!("podman machine start after init failed: {err}"),
            );
            Ok(())
        }
    }
}

fn format_heartbeat_elapsed(duration: Duration) -> String {
    let total_seconds = duration.as_secs();
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    if minutes == 0 {
        format!("{seconds}s")
    } else {
        format!("{minutes}m {seconds}s")
    }
}

async fn wait_for_podman_machine_ready(
    data_root: &Path,
    observer: Option<&dyn HarnessSetupObserver>,
    success_message: &str,
    last_err: &mut String,
) -> Result<bool> {
    let deadline = tokio::time::Instant::now() + podman_machine_ready_timeout();
    let started = tokio::time::Instant::now();
    let mut last_heartbeat = started;
    while tokio::time::Instant::now() < deadline {
        let mut cmd = podman_command(data_root)?;
        cmd.arg("info");
        match command_output_with_timeout(cmd, PODMAN_INFO_TIMEOUT).await {
            Ok(out) if out.status.success() => {
                observe_log(
                    observer,
                    HarnessSetupPhase::MachineStartOrInit,
                    HarnessSetupLogLevel::Info,
                    success_message,
                );
                persist_podman_machine_cache_to_shared_best_effort(data_root, observer).await;
                return Ok(true);
            }
            Ok(out) => {
                let combined = command_output_message(&out);
                if !combined.is_empty() {
                    *last_err = combined;
                }
            }
            Err(err) => *last_err = err.to_string(),
        }
        let now = tokio::time::Instant::now();
        if now.duration_since(last_heartbeat) >= podman_machine_heartbeat_interval() {
            observe_log(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                HarnessSetupLogLevel::Info,
                &format!(
                    "still waiting for local sandbox runtime readiness ({} elapsed)",
                    format_heartbeat_elapsed(started.elapsed())
                ),
            );
            observe_progress(
                observer,
                HarnessSetupProgressUpdate {
                    phase: HarnessSetupPhase::MachineStartOrInit,
                    active_download: None,
                },
            );
            last_heartbeat = now;
        }
        tokio::time::sleep(podman_machine_ready_poll_interval()).await;
    }
    Ok(false)
}

pub(super) struct PodmanMachineInitOutcome {
    pub(super) output: std::process::Output,
    pub(super) continued_after_machine_present: bool,
}

async fn read_child_pipe<R>(mut reader: R) -> std::io::Result<Vec<u8>>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut buf = Vec::new();
    reader.read_to_end(&mut buf).await?;
    Ok(buf)
}

async fn collect_child_output(
    status: std::process::ExitStatus,
    stdout_task: tokio::task::JoinHandle<std::io::Result<Vec<u8>>>,
    stderr_task: tokio::task::JoinHandle<std::io::Result<Vec<u8>>>,
) -> Result<std::process::Output> {
    let stdout = stdout_task
        .await
        .context("joining podman machine init stdout capture")??;
    let stderr = stderr_task
        .await
        .context("joining podman machine init stderr capture")??;
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

pub(super) async fn run_podman_machine_init(
    data_root: &Path,
    machine_name: &str,
    machine_image: Option<&Path>,
    memory_mb: Option<u32>,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<PodmanMachineInitOutcome> {
    observe_phase(
        observer,
        HarnessSetupPhase::MachineStartOrInit,
        "materializing local sandbox runtime from managed cache",
    );
    let mut init = podman_command(data_root)?;
    init.arg("machine").arg("init").arg(machine_name);
    if let Some(machine_image) = machine_image {
        init.arg("--image").arg(machine_image);
    }
    if let Some(memory_mb) = memory_mb {
        init.arg("--memory").arg(memory_mb.to_string());
    }
    init.stdout(Stdio::piped());
    init.stderr(Stdio::piped());
    init.kill_on_drop(true);
    let mut child = init.spawn().context("spawning podman machine init")?;
    let stdout = child
        .stdout
        .take()
        .context("podman machine init stdout was not captured")?;
    let stderr = child
        .stderr
        .take()
        .context("podman machine init stderr was not captured")?;
    let stdout_task = tokio::spawn(read_child_pipe(stdout));
    let stderr_task = tokio::spawn(read_child_pipe(stderr));
    let deadline = tokio::time::Instant::now() + PODMAN_MACHINE_INIT_TIMEOUT;
    let started = tokio::time::Instant::now();
    let mut machine_present_since: Option<tokio::time::Instant> = None;
    let mut last_heartbeat = started;

    loop {
        if let Some(status) = child
            .try_wait()
            .context("polling podman machine init process")?
        {
            return Ok(PodmanMachineInitOutcome {
                output: collect_child_output(status, stdout_task, stderr_task).await?,
                continued_after_machine_present: false,
            });
        }

        let machine_present = podman_machine_present(data_root, machine_name)
            .await
            .unwrap_or(false);
        if machine_present {
            let now = tokio::time::Instant::now();
            let present_since = machine_present_since.get_or_insert(now);
            if now.duration_since(*present_since) >= podman_machine_init_created_machine_grace() {
                observe_log(
                    observer,
                    HarnessSetupPhase::MachineStartOrInit,
                    HarnessSetupLogLevel::Warn,
                    "podman machine init created machine state but did not exit; terminating init and continuing with explicit start",
                );
                let _ = child.start_kill();
                let status = child
                    .wait()
                    .await
                    .context("waiting for terminated podman machine init")?;
                return Ok(PodmanMachineInitOutcome {
                    output: collect_child_output(status, stdout_task, stderr_task).await?,
                    continued_after_machine_present: true,
                });
            }
        } else {
            machine_present_since = None;
        }

        if tokio::time::Instant::now() >= deadline {
            let _ = child.start_kill();
            let status = child
                .wait()
                .await
                .context("waiting for timed out podman machine init")?;
            let output = collect_child_output(status, stdout_task, stderr_task).await?;
            if machine_present {
                observe_log(
                    observer,
                    HarnessSetupPhase::MachineStartOrInit,
                    HarnessSetupLogLevel::Warn,
                    "podman machine init timed out after machine creation; continuing with explicit start",
                );
                return Ok(PodmanMachineInitOutcome {
                    output,
                    continued_after_machine_present: true,
                });
            }
            anyhow::bail!(
                "podman machine init timed out after {}s",
                PODMAN_MACHINE_INIT_TIMEOUT.as_secs()
            );
        }

        let now = tokio::time::Instant::now();
        if now.duration_since(last_heartbeat) >= podman_machine_heartbeat_interval() {
            observe_log(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                HarnessSetupLogLevel::Info,
                &format!(
                    "still materializing local sandbox runtime from cached image ({} elapsed)",
                    format_heartbeat_elapsed(started.elapsed())
                ),
            );
            observe_progress(
                observer,
                HarnessSetupProgressUpdate {
                    phase: HarnessSetupPhase::MachineStartOrInit,
                    active_download: None,
                },
            );
            last_heartbeat = now;
        }

        tokio::time::sleep(podman_machine_init_poll_interval()).await;
    }
}

pub(super) async fn initialize_podman_machine(
    data_root: &Path,
    machine_name: &str,
    memory_mb: Option<u32>,
    observer: Option<&dyn HarnessSetupObserver>,
    last_err: &mut String,
) -> Result<()> {
    let machine_image = if cfg!(target_os = "macos") {
        Some(
            ensure_managed_podman_machine_cache(data_root, observer, None)
                .await
                .context("managed podman machine cache unavailable")?,
        )
    } else {
        None
    };
    let init_outcome = run_podman_machine_init(
        data_root,
        machine_name,
        machine_image.as_deref(),
        memory_mb,
        observer,
    )
    .await
    .context("podman machine init")?;
    let out = init_outcome.output;
    let combined = command_output_message(&out);
    if init_outcome.continued_after_machine_present {
        if !combined.is_empty() {
            *last_err = combined;
        }
    } else if !out.status.success() {
        let combined_lc = combined.to_ascii_lowercase();
        if combined_lc.contains("already exists") {
            let message = if combined.is_empty() {
                "podman machine init reported existing machine; starting it explicitly".to_string()
            } else {
                format!(
                    "podman machine init reported existing machine; starting it explicitly: {combined}"
                )
            };
            observe_log(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                HarnessSetupLogLevel::Warn,
                &message,
            );
            if !combined.is_empty() {
                *last_err = combined;
            }
        } else if combined.is_empty() {
            anyhow::bail!("podman machine init failed (status: {})", out.status);
        } else {
            anyhow::bail!("podman machine init failed: {combined}");
        }
    }

    best_effort_start_machine_after_init(data_root, machine_name, observer, last_err).await
}

pub(super) async fn ensure_podman_machine_running_with_observer(
    data_root: &Path,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<()> {
    let machine_name = ctx_podman_machine_name(data_root);
    let machine_lock = podman_machine_singleflight_lock(&machine_name);
    let machine_guard = match machine_lock.try_lock() {
        Ok(guard) => guard,
        Err(_) => {
            observe_log(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                HarnessSetupLogLevel::Info,
                "waiting for concurrent local sandbox runtime init/start operation",
            );
            machine_lock.lock().await
        }
    };
    if !podman_machine_required() {
        observe_log(
            observer,
            HarnessSetupPhase::MachineCheck,
            HarnessSetupLogLevel::Info,
            "podman machine not required on this platform",
        );
        return Ok(());
    }
    seed_shared_podman_machine_cache_best_effort(data_root, observer).await;
    let desired_memory_mb = configured_podman_machine_memory_mb(data_root, observer).await;

    let mut last_err = {
        let mut cmd = podman_command(data_root)?;
        cmd.arg("info");
        match command_output_with_timeout(cmd, PODMAN_INFO_TIMEOUT).await {
            Ok(out) if out.status.success() => {
                observe_log(
                    observer,
                    HarnessSetupPhase::MachineCheck,
                    HarnessSetupLogLevel::Info,
                    "local sandbox runtime is already reachable",
                );
                persist_podman_machine_cache_to_shared_best_effort(data_root, observer).await;
                return Ok(());
            }
            Ok(out) => String::from_utf8_lossy(&out.stderr).trim().to_string(),
            Err(err) => err.to_string(),
        }
    };

    observe_phase(
        observer,
        HarnessSetupPhase::MachineStartOrInit,
        "starting or initializing local sandbox runtime",
    );
    clear_stale_podman_machine_temp_state(data_root, &machine_name, observer);

    let mut wait_after_start = true;
    let mut force_recreate = false;
    let start_out = {
        let mut start = podman_command(data_root)?;
        start.arg("machine").arg("start").arg(&machine_name);
        command_output_with_timeout(start, PODMAN_MACHINE_START_TIMEOUT).await?
    };
    if start_out.status.success() {
        observe_log(
            observer,
            HarnessSetupPhase::MachineStartOrInit,
            HarnessSetupLogLevel::Info,
            "local sandbox runtime start command completed; waiting for readiness",
        );
    } else {
        let combined = command_output_message(&start_out);
        let combined_lc = combined.to_ascii_lowercase();

        let looks_like_missing_machine = looks_like_missing_machine_error(&combined_lc);

        if looks_like_missing_machine {
            observe_phase(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                "materializing local sandbox runtime from managed cache",
            );
            initialize_podman_machine(
                data_root,
                &machine_name,
                Some(desired_memory_mb),
                observer,
                &mut last_err,
            )
            .await?;
            observe_phase(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                "waiting for local sandbox runtime readiness",
            );
        } else if looks_like_recoverable_machine_start_error(&combined_lc) {
            if looks_like_running_but_unreachable_machine_start_error(&combined_lc) {
                let message = if combined.is_empty() {
                    "podman machine start reported an already-running machine while podman remained unreachable; restarting once"
                        .to_string()
                } else {
                    format!(
                        "podman machine start reported an already-running machine while podman remained unreachable; restarting once: {combined}"
                    )
                };
                observe_log(
                    observer,
                    HarnessSetupPhase::MachineStartOrInit,
                    HarnessSetupLogLevel::Warn,
                    &message,
                );
                wait_after_start = false;
                force_recreate = true;
            } else {
                let message = if combined.is_empty() {
                    "podman machine start returned a recoverable error; waiting for readiness"
                        .to_string()
                } else {
                    format!(
                        "podman machine start returned recoverable error; waiting for readiness: {combined}"
                    )
                };
                observe_log(
                    observer,
                    HarnessSetupPhase::MachineStartOrInit,
                    HarnessSetupLogLevel::Warn,
                    &message,
                );
            }
            if !combined.is_empty() {
                last_err = combined;
            }
        } else if combined.is_empty() {
            anyhow::bail!(
                "podman machine start failed with non-zero exit {}",
                start_out.status
            );
        } else {
            anyhow::bail!("podman machine start failed: {combined}");
        }
    }

    if wait_after_start {
        observe_phase(
            observer,
            HarnessSetupPhase::MachineStartOrInit,
            "waiting for local sandbox runtime readiness",
        );
        if wait_for_podman_machine_ready(
            data_root,
            observer,
            "local sandbox runtime is ready",
            &mut last_err,
        )
        .await?
        {
            return Ok(());
        }
    }

    if !force_recreate {
        observe_log(
            observer,
            HarnessSetupPhase::MachineStartOrInit,
            HarnessSetupLogLevel::Warn,
            "podman machine remained unreachable after start; restarting once",
        );
        let stop_out = {
            let mut stop = podman_command(data_root)?;
            stop.arg("machine").arg("stop").arg(&machine_name);
            command_output_with_timeout(stop, PODMAN_MACHINE_START_TIMEOUT).await?
        };
        if !stop_out.status.success() {
            let combined = command_output_message(&stop_out);
            if !combined.is_empty() {
                last_err = combined.clone();
                observe_log(
                    observer,
                    HarnessSetupPhase::MachineStartOrInit,
                    HarnessSetupLogLevel::Warn,
                    &format!("podman machine stop returned non-zero during recovery: {combined}"),
                );
            }
        }
        let restart_out = {
            let mut start = podman_command(data_root)?;
            start.arg("machine").arg("start").arg(&machine_name);
            command_output_with_timeout(start, PODMAN_MACHINE_START_TIMEOUT).await?
        };
        if !restart_out.status.success() {
            let combined = command_output_message(&restart_out);
            if !combined.is_empty() {
                last_err = combined.clone();
                observe_log(
                    observer,
                    HarnessSetupPhase::MachineStartOrInit,
                    HarnessSetupLogLevel::Warn,
                    &format!(
                        "podman machine start returned non-zero during restart recovery: {combined}"
                    ),
                );
            } else {
                observe_log(
                    observer,
                    HarnessSetupPhase::MachineStartOrInit,
                    HarnessSetupLogLevel::Warn,
                    &format!(
                        "podman machine start returned non-zero during restart recovery: {}",
                        restart_out.status
                    ),
                );
            }
        }
        observe_phase(
            observer,
            HarnessSetupPhase::MachineStartOrInit,
            "waiting for local sandbox runtime readiness",
        );
        if wait_for_podman_machine_ready(
            data_root,
            observer,
            "local sandbox runtime recovered after restart",
            &mut last_err,
        )
        .await?
        {
            return Ok(());
        }
    }

    let machine_present = podman_machine_present(data_root, &machine_name)
        .await
        .unwrap_or(false);
    if machine_present || force_recreate {
        observe_log(
            observer,
            HarnessSetupPhase::MachineStartOrInit,
            HarnessSetupLogLevel::Warn,
            if force_recreate {
                "podman machine reported an already-running but unreachable state; recreating machine"
            } else {
                "podman machine still unreachable after restart; recreating machine"
            },
        );
        cleanup_ctx_managed_podman_helper_processes(data_root, &machine_name, observer);
        clear_stale_podman_machine_temp_state(data_root, &machine_name, observer);

        if machine_present {
            let mut rm = podman_command(data_root)?;
            rm.arg("machine").arg("rm").arg("-f").arg(&machine_name);
            let rm_out = command_output_with_timeout(rm, PODMAN_MACHINE_START_TIMEOUT).await?;
            if !rm_out.status.success() {
                let combined = command_output_message(&rm_out);
                if !combined.is_empty() {
                    last_err = format!("podman machine rm -f failed: {combined}");
                }
            }
        }

        if let Err(err) = initialize_podman_machine(
            data_root,
            &machine_name,
            Some(desired_memory_mb),
            observer,
            &mut last_err,
        )
        .await
        {
            last_err = format!("podman machine init failed after recreate: {err:#}");
        } else {
            observe_log(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                HarnessSetupLogLevel::Info,
                "recreated podman machine; waiting for readiness",
            );
            if wait_for_podman_machine_ready(
                data_root,
                observer,
                "podman machine recovered after recreation",
                &mut last_err,
            )
            .await?
            {
                return Ok(());
            }
        }
    }

    drop(machine_guard);
    if last_err.trim().is_empty() {
        anyhow::bail!("podman machine remained unreachable after bounded recovery");
    }
    anyhow::bail!(
        "podman machine remained unreachable after bounded recovery: {}",
        last_err.trim()
    );
}
