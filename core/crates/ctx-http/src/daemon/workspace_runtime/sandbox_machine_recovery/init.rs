use super::*;

pub(in crate::daemon::workspace_runtime) struct SandboxMachineInitOutcome {
    pub(in crate::daemon::workspace_runtime) output: std::process::Output,
    pub(in crate::daemon::workspace_runtime) continued_after_machine_present: bool,
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
        .context("joining sandbox machine init stdout capture")??;
    let stderr = stderr_task
        .await
        .context("joining sandbox machine init stderr capture")??;
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

pub(in crate::daemon::workspace_runtime) async fn run_sandbox_machine_init(
    data_root: &Path,
    machine_name: &str,
    machine_image: Option<&Path>,
    memory_mb: Option<u32>,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<SandboxMachineInitOutcome> {
    observe_phase(
        observer,
        HarnessSetupPhase::MachineStartOrInit,
        "materializing local sandbox runtime from managed cache",
    );
    let mut init = sandbox_container_command(data_root)?;
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
    let mut child = init.spawn().context("spawning sandbox machine init")?;
    let stdout = child
        .stdout
        .take()
        .context("sandbox machine init stdout was not captured")?;
    let stderr = child
        .stderr
        .take()
        .context("sandbox machine init stderr was not captured")?;
    let stdout_task = tokio::spawn(read_child_pipe(stdout));
    let stderr_task = tokio::spawn(read_child_pipe(stderr));
    let deadline = tokio::time::Instant::now() + SANDBOX_MACHINE_INIT_TIMEOUT;
    let started = tokio::time::Instant::now();
    let mut machine_present_since: Option<tokio::time::Instant> = None;
    let mut last_heartbeat = started;

    loop {
        if let Some(status) = child
            .try_wait()
            .context("polling sandbox machine init process")?
        {
            return Ok(SandboxMachineInitOutcome {
                output: collect_child_output(status, stdout_task, stderr_task).await?,
                continued_after_machine_present: false,
            });
        }

        let machine_present = sandbox_machine_present(data_root, machine_name)
            .await
            .unwrap_or(false);
        if machine_present {
            let now = tokio::time::Instant::now();
            let present_since = machine_present_since.get_or_insert(now);
            if now.duration_since(*present_since) >= sandbox_machine_init_created_machine_grace() {
                observe_log(
                    observer,
                    HarnessSetupPhase::MachineStartOrInit,
                    HarnessSetupLogLevel::Warn,
                    "sandbox machine init created machine state but did not exit; terminating init and continuing with explicit start",
                );
                let _ = child.start_kill();
                let status = child
                    .wait()
                    .await
                    .context("waiting for terminated sandbox machine init")?;
                return Ok(SandboxMachineInitOutcome {
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
                .context("waiting for timed out sandbox machine init")?;
            let output = collect_child_output(status, stdout_task, stderr_task).await?;
            if machine_present {
                observe_log(
                    observer,
                    HarnessSetupPhase::MachineStartOrInit,
                    HarnessSetupLogLevel::Warn,
                    "sandbox machine init timed out after machine creation; continuing with explicit start",
                );
                return Ok(SandboxMachineInitOutcome {
                    output,
                    continued_after_machine_present: true,
                });
            }
            anyhow::bail!(
                "sandbox machine init timed out after {}s",
                SANDBOX_MACHINE_INIT_TIMEOUT.as_secs()
            );
        }

        let now = tokio::time::Instant::now();
        if now.duration_since(last_heartbeat) >= sandbox_machine_heartbeat_interval() {
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

        tokio::time::sleep(sandbox_machine_init_poll_interval()).await;
    }
}

pub(in crate::daemon::workspace_runtime) async fn initialize_sandbox_machine(
    data_root: &Path,
    machine_name: &str,
    memory_mb: Option<u32>,
    observer: Option<&dyn HarnessSetupObserver>,
    last_err: &mut String,
) -> Result<()> {
    let machine_image = if cfg!(target_os = "macos") {
        Some(
            ensure_managed_sandbox_machine_cache(data_root, observer, None)
                .await
                .context("managed sandbox machine cache unavailable")?,
        )
    } else {
        None
    };
    initialize_sandbox_machine_with_image(
        data_root,
        machine_name,
        machine_image.as_deref(),
        memory_mb,
        observer,
        last_err,
    )
    .await
}

pub(in crate::daemon::workspace_runtime) async fn initialize_sandbox_machine_with_image(
    data_root: &Path,
    machine_name: &str,
    machine_image: Option<&Path>,
    memory_mb: Option<u32>,
    observer: Option<&dyn HarnessSetupObserver>,
    last_err: &mut String,
) -> Result<()> {
    let init_outcome =
        run_sandbox_machine_init(data_root, machine_name, machine_image, memory_mb, observer)
            .await
            .context("sandbox machine init")?;
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
                "sandbox machine init reported existing machine; starting it explicitly".to_string()
            } else {
                format!(
                    "sandbox machine init reported existing machine; starting it explicitly: {combined}"
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
            anyhow::bail!("sandbox machine init failed (status: {})", out.status);
        } else {
            anyhow::bail!("sandbox machine init failed: {combined}");
        }
    }

    best_effort_start_machine_after_init(data_root, machine_name, observer, last_err).await
}
