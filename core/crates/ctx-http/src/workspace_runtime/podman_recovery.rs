use super::*;
use std::collections::HashMap;
use std::process::{Command as StdCommand, Stdio};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};

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

pub(super) async fn podman_machine_present(data_root: &Path) -> Result<bool> {
    let mut cmd = podman_command(data_root)?;
    let machine_name = ctx_podman_machine_name(data_root);
    cmd.arg("machine").arg("inspect").arg(&machine_name);
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

#[cfg(any(test, not(unix)))]
pub(super) fn is_ctx_managed_podman_helper_process_command(
    command: &[String],
    data_root: &Path,
    machine_name: &str,
) -> bool {
    let rendered = command.join("\n");
    is_ctx_managed_podman_helper_process_rendered(&rendered, data_root, machine_name)
}

fn is_ctx_managed_podman_helper_process_rendered(
    rendered: &str,
    data_root: &Path,
    machine_name: &str,
) -> bool {
    if !rendered.contains("/gvproxy") && !rendered.contains("/vfkit") {
        return false;
    }

    if !rendered.contains(machine_name) {
        return false;
    }

    let scoped_roots = [
        data_root.join("managed").join("runtimes").join("podman"),
        podman_runtime_root(data_root),
        podman_home_root(data_root),
        podman_temp_root(data_root),
        data_root.join("podman"),
    ];
    scoped_roots.iter().any(|root| {
        let root = root.to_string_lossy();
        rendered.contains(root.as_ref())
    })
}

#[cfg(any(test, not(unix)))]
pub(super) fn collect_ctx_managed_podman_helper_pids<I>(
    rows: I,
    data_root: &Path,
    machine_name: &str,
) -> Vec<u32>
where
    I: IntoIterator<Item = (u32, Vec<String>)>,
{
    let mut pids = rows
        .into_iter()
        .filter_map(|(pid, command)| {
            is_ctx_managed_podman_helper_process_command(&command, data_root, machine_name)
                .then_some(pid)
        })
        .collect::<Vec<_>>();
    pids.sort_unstable();
    pids.dedup();
    pids
}

fn parse_ps_pid_and_command(line: &str) -> Option<(u32, &str)> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let split_idx = trimmed.find(char::is_whitespace)?;
    let pid = trimmed[..split_idx].trim().parse::<u32>().ok()?;
    let command = trimmed[split_idx..].trim_start();
    if command.is_empty() {
        return None;
    }
    Some((pid, command))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PsProcessRow {
    pid: u32,
    command: String,
}

fn collect_ps_process_rows(output: &str) -> Vec<PsProcessRow> {
    let mut rows = output
        .lines()
        .filter_map(parse_ps_pid_and_command)
        .map(|(pid, command)| PsProcessRow {
            pid,
            command: command.to_string(),
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        left.pid
            .cmp(&right.pid)
            .then_with(|| left.command.cmp(&right.command))
    });
    rows.dedup_by(|left, right| left.pid == right.pid && left.command == right.command);
    rows
}

#[cfg(unix)]
fn snapshot_ps_process_rows() -> Option<Vec<PsProcessRow>> {
    let output = StdCommand::new("ps")
        .arg("-axo")
        .arg("pid=,command=")
        .output()
        .ok()?;
    Some(collect_ps_process_rows(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

#[cfg(unix)]
fn collect_ctx_managed_podman_helper_processes_from_ps_rows<I>(
    rows: I,
    data_root: &Path,
    machine_name: &str,
) -> Vec<PsProcessRow>
where
    I: IntoIterator<Item = PsProcessRow>,
{
    let mut processes = rows
        .into_iter()
        .filter(|row| {
            is_ctx_managed_podman_helper_process_rendered(&row.command, data_root, machine_name)
        })
        .collect::<Vec<_>>();
    processes.sort_by(|left, right| {
        left.pid
            .cmp(&right.pid)
            .then_with(|| left.command.cmp(&right.command))
    });
    processes.dedup_by(|left, right| left.pid == right.pid && left.command == right.command);
    processes
}

#[cfg(test)]
pub(super) fn collect_ctx_managed_podman_helper_pids_from_ps_output(
    output: &str,
    data_root: &Path,
    machine_name: &str,
) -> Vec<u32> {
    let mut pids = collect_ps_process_rows(output)
        .into_iter()
        .filter_map(|row| {
            is_ctx_managed_podman_helper_process_rendered(&row.command, data_root, machine_name)
                .then_some(row.pid)
        })
        .collect::<Vec<_>>();
    pids.sort_unstable();
    pids.dedup();
    pids
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct PodmanHelperCleanupOutcome {
    pub(super) killed: Vec<u32>,
    pub(super) failed: Vec<u32>,
    pub(super) skipped: Vec<u32>,
}

pub(super) fn literal_pkill_pattern(command: &str) -> String {
    let mut pattern = String::with_capacity(command.len());
    for ch in command.chars() {
        if matches!(
            ch,
            '\\' | '.' | '[' | ']' | '(' | ')' | '{' | '}' | '^' | '$' | '*' | '+' | '?' | '|'
        ) {
            pattern.push('\\');
        }
        pattern.push(ch);
    }
    pattern
}

#[cfg(unix)]
fn kill_ctx_managed_podman_helper_command(command: &str) -> bool {
    let literal_pattern = literal_pkill_pattern(command);
    StdCommand::new("pkill")
        .arg("-9")
        .arg("-f")
        .arg("-x")
        .arg(literal_pattern)
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(unix)]
pub(super) fn kill_ctx_managed_podman_helper_processes(
    data_root: &Path,
    machine_name: &str,
) -> PodmanHelperCleanupOutcome {
    let Some(before_rows) = snapshot_ps_process_rows() else {
        return PodmanHelperCleanupOutcome::default();
    };
    let helpers = collect_ctx_managed_podman_helper_processes_from_ps_rows(
        before_rows,
        data_root,
        machine_name,
    );
    let mut outcome = PodmanHelperCleanupOutcome::default();
    if helpers.is_empty() {
        return outcome;
    }

    let mut kill_results = HashMap::new();
    for helper in &helpers {
        kill_results
            .entry(helper.command.clone())
            .or_insert_with(|| kill_ctx_managed_podman_helper_command(&helper.command));
    }

    let after_rows = snapshot_ps_process_rows().map(|rows| {
        rows.into_iter()
            .map(|row| (row.pid, row.command))
            .collect::<HashMap<_, _>>()
    });

    for helper in helpers {
        let pid = helper.pid;
        match after_rows
            .as_ref()
            .and_then(|rows| rows.get(&pid))
            .map(String::as_str)
        {
            Some(command_after) if command_after == helper.command => outcome.failed.push(pid),
            Some(_) => outcome.skipped.push(pid),
            None => {
                if kill_results.get(&helper.command).copied().unwrap_or(false) {
                    outcome.killed.push(pid);
                } else if after_rows.is_some() {
                    outcome.skipped.push(pid);
                } else {
                    outcome.failed.push(pid);
                }
            }
        }
    }
    outcome.killed.sort_unstable();
    outcome.failed.sort_unstable();
    outcome.skipped.sort_unstable();
    outcome
}

#[cfg(not(unix))]
pub(super) fn kill_ctx_managed_podman_helper_processes(
    data_root: &Path,
    machine_name: &str,
) -> PodmanHelperCleanupOutcome {
    let mut system = sysinfo::System::new_all();
    system.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    let rows = system.processes().iter().map(|(pid, process)| {
        let pid_u32 = (*pid).as_u32();
        let command = process
            .cmd()
            .iter()
            .map(|part| part.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        (pid_u32, command)
    });
    let pids = collect_ctx_managed_podman_helper_pids(rows, data_root, machine_name);
    let mut outcome = PodmanHelperCleanupOutcome::default();
    for pid in pids {
        system.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        let still_matches = system
            .process(sysinfo::Pid::from_u32(pid))
            .map(|process| {
                let command = process
                    .cmd()
                    .iter()
                    .map(|part| part.to_string_lossy().into_owned())
                    .collect::<Vec<_>>();
                is_ctx_managed_podman_helper_process_command(&command, data_root, machine_name)
            })
            .unwrap_or(false);
        if !still_matches {
            outcome.skipped.push(pid);
            continue;
        }
        let killed = if let Some(process) = system.process(sysinfo::Pid::from_u32(pid)) {
            process.kill_with(sysinfo::Signal::Kill).unwrap_or(false)
        } else {
            false
        };
        if killed {
            outcome.killed.push(pid);
        } else {
            outcome.failed.push(pid);
        }
    }
    outcome
}

fn cleanup_ctx_managed_podman_helper_processes(
    data_root: &Path,
    machine_name: &str,
    observer: Option<&dyn HarnessSetupObserver>,
) {
    let outcome = kill_ctx_managed_podman_helper_processes(data_root, machine_name);
    if !outcome.killed.is_empty() {
        observe_log(
            observer,
            HarnessSetupPhase::MachineStartOrInit,
            HarnessSetupLogLevel::Warn,
            &format!(
                "killed stale ctx-managed podman helper process(es) before recovery: {}",
                outcome
                    .killed
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        );
    }
    if !outcome.failed.is_empty() {
        observe_log(
            observer,
            HarnessSetupPhase::MachineStartOrInit,
            HarnessSetupLogLevel::Warn,
            &format!(
                "failed to kill stale ctx-managed podman helper process(es) before recovery: {}",
                outcome
                    .failed
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        );
    }
    if !outcome.skipped.is_empty() {
        observe_log(
            observer,
            HarnessSetupPhase::MachineStartOrInit,
            HarnessSetupLogLevel::Warn,
            &format!(
                "skipped killing stale ctx-managed podman helper process(es) after command-scoped cleanup no longer matched the original helper identity: {}",
                outcome
                    .skipped
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        );
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

async fn wait_for_podman_machine_ready(
    data_root: &Path,
    observer: Option<&dyn HarnessSetupObserver>,
    success_message: &str,
    last_err: &mut String,
) -> Result<bool> {
    let deadline = tokio::time::Instant::now() + podman_machine_ready_timeout();
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
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<PodmanMachineInitOutcome> {
    let mut init = podman_command(data_root)?;
    init.arg("machine").arg("init").arg(machine_name);
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
    let mut machine_present_since: Option<tokio::time::Instant> = None;

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

        let machine_present = podman_machine_present(data_root).await.unwrap_or(false);
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

        tokio::time::sleep(podman_machine_init_poll_interval()).await;
    }
}

pub(super) async fn initialize_podman_machine(
    data_root: &Path,
    machine_name: &str,
    observer: Option<&dyn HarnessSetupObserver>,
    last_err: &mut String,
) -> Result<()> {
    let init_outcome = run_podman_machine_init(data_root, machine_name, observer)
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
                "waiting for concurrent podman machine init/start operation",
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

    let mut last_err = {
        let mut cmd = podman_command(data_root)?;
        cmd.arg("info");
        match command_output_with_timeout(cmd, PODMAN_INFO_TIMEOUT).await {
            Ok(out) if out.status.success() => {
                observe_log(
                    observer,
                    HarnessSetupPhase::MachineCheck,
                    HarnessSetupLogLevel::Info,
                    "podman runtime is already reachable",
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
        "starting or initializing podman machine",
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
            "podman machine start command completed; waiting for readiness",
        );
    } else {
        let combined = command_output_message(&start_out);
        let combined_lc = combined.to_ascii_lowercase();

        let looks_like_missing_machine = looks_like_missing_machine_error(&combined_lc);

        if looks_like_missing_machine {
            observe_log(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                HarnessSetupLogLevel::Info,
                "podman machine not found; running init then explicit start",
            );
            initialize_podman_machine(data_root, &machine_name, observer, &mut last_err).await?;
            observe_log(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                HarnessSetupLogLevel::Info,
                "podman machine initialized; waiting for readiness",
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
        observe_log(
            observer,
            HarnessSetupPhase::MachineStartOrInit,
            HarnessSetupLogLevel::Info,
            "waiting for podman machine readiness after start",
        );
        if wait_for_podman_machine_ready(
            data_root,
            observer,
            "podman machine is ready",
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
        observe_log(
            observer,
            HarnessSetupPhase::MachineStartOrInit,
            HarnessSetupLogLevel::Info,
            "waiting for podman machine readiness after restart",
        );
        if wait_for_podman_machine_ready(
            data_root,
            observer,
            "podman machine recovered after restart",
            &mut last_err,
        )
        .await?
        {
            return Ok(());
        }
    }

    let machine_present = podman_machine_present(data_root).await.unwrap_or(false);
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

        if let Err(err) =
            initialize_podman_machine(data_root, &machine_name, observer, &mut last_err).await
        {
            last_err = format!("podman machine init failed after recreate: {err:#}");
        } else {
            observe_log(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                HarnessSetupLogLevel::Info,
                "waiting for podman machine readiness after recreation",
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
