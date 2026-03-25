use super::*;

#[cfg(any(test, not(unix)))]
#[cfg(any(test, not(unix)))]
pub(in crate::workspace_runtime) fn is_ctx_managed_sandbox_helper_process_command(
    command: &[String],
    data_root: &Path,
    machine_name: &str,
) -> bool {
    let rendered = command.join("\n");
    is_ctx_managed_sandbox_helper_process_rendered(&rendered, data_root, machine_name)
}

fn is_ctx_managed_sandbox_helper_process_rendered(
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
        data_root
            .join("managed")
            .join("runtimes")
            .join("sandbox-cli"),
        sandbox_machine_runtime_root(data_root),
        sandbox_machine_home_root(data_root),
        sandbox_machine_temp_root(data_root),
        data_root.join("sandbox-cli"),
    ];
    scoped_roots.iter().any(|root| {
        let root = root.to_string_lossy();
        rendered.contains(root.as_ref())
    })
}

#[cfg(any(test, not(unix)))]
pub(in crate::workspace_runtime) fn collect_ctx_managed_sandbox_helper_pids<I>(
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
            is_ctx_managed_sandbox_helper_process_command(&command, data_root, machine_name)
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
fn collect_ctx_managed_sandbox_helper_processes_from_ps_rows<I>(
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
            is_ctx_managed_sandbox_helper_process_rendered(&row.command, data_root, machine_name)
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
pub(in crate::workspace_runtime) fn collect_ctx_managed_sandbox_helper_pids_from_ps_output(
    output: &str,
    data_root: &Path,
    machine_name: &str,
) -> Vec<u32> {
    let mut pids = collect_ps_process_rows(output)
        .into_iter()
        .filter_map(|row| {
            is_ctx_managed_sandbox_helper_process_rendered(&row.command, data_root, machine_name)
                .then_some(row.pid)
        })
        .collect::<Vec<_>>();
    pids.sort_unstable();
    pids.dedup();
    pids
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(in crate::workspace_runtime) struct SandboxHelperCleanupOutcome {
    pub(in crate::workspace_runtime) killed: Vec<u32>,
    pub(in crate::workspace_runtime) failed: Vec<u32>,
    pub(in crate::workspace_runtime) skipped: Vec<u32>,
}

pub(in crate::workspace_runtime) fn literal_pkill_pattern(command: &str) -> String {
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
fn kill_ctx_managed_sandbox_helper_command(command: &str) -> bool {
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
pub(in crate::workspace_runtime) fn kill_ctx_managed_sandbox_helper_processes(
    data_root: &Path,
    machine_name: &str,
) -> SandboxHelperCleanupOutcome {
    let Some(before_rows) = snapshot_ps_process_rows() else {
        return SandboxHelperCleanupOutcome::default();
    };
    let helpers = collect_ctx_managed_sandbox_helper_processes_from_ps_rows(
        before_rows,
        data_root,
        machine_name,
    );
    let mut outcome = SandboxHelperCleanupOutcome::default();
    if helpers.is_empty() {
        return outcome;
    }

    let mut kill_results = HashMap::new();
    for helper in &helpers {
        kill_results
            .entry(helper.command.clone())
            .or_insert_with(|| kill_ctx_managed_sandbox_helper_command(&helper.command));
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
pub(in crate::workspace_runtime) fn kill_ctx_managed_sandbox_helper_processes(
    data_root: &Path,
    machine_name: &str,
) -> SandboxHelperCleanupOutcome {
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
    let pids = collect_ctx_managed_sandbox_helper_pids(rows, data_root, machine_name);
    let mut outcome = SandboxHelperCleanupOutcome::default();
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
                is_ctx_managed_sandbox_helper_process_command(&command, data_root, machine_name)
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

pub(super) fn cleanup_ctx_managed_sandbox_helper_processes(
    data_root: &Path,
    machine_name: &str,
    observer: Option<&dyn HarnessSetupObserver>,
) {
    let outcome = kill_ctx_managed_sandbox_helper_processes(data_root, machine_name);
    if !outcome.killed.is_empty() {
        observe_log(
            observer,
            HarnessSetupPhase::MachineStartOrInit,
            HarnessSetupLogLevel::Warn,
            &format!(
                "killed stale ctx-managed sandbox helper process(es) before recovery: {}",
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
                "failed to kill stale ctx-managed sandbox helper process(es) before recovery: {}",
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
                "skipped killing stale ctx-managed sandbox helper process(es) after command-scoped cleanup no longer matched the original helper identity: {}",
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
