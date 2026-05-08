use std::collections::HashMap;
use std::path::Path;

#[cfg(unix)]
use std::process::Command as StdCommand;

#[cfg(unix)]
use super::detection::{
    collect_ctx_managed_sandbox_helper_processes_from_ps_rows, snapshot_ps_process_rows,
};

#[cfg(not(unix))]
use super::detection::{
    collect_ctx_managed_sandbox_helper_pids, is_ctx_managed_sandbox_helper_process_command,
};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(in crate::daemon::workspace_runtime) struct SandboxHelperCleanupOutcome {
    pub(in crate::daemon::workspace_runtime) killed: Vec<u32>,
    pub(in crate::daemon::workspace_runtime) failed: Vec<u32>,
    pub(in crate::daemon::workspace_runtime) skipped: Vec<u32>,
}

pub(in crate::daemon::workspace_runtime) fn literal_pkill_pattern(command: &str) -> String {
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
pub(in crate::daemon::workspace_runtime) fn kill_ctx_managed_sandbox_helper_processes(
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
pub(in crate::daemon::workspace_runtime) fn kill_ctx_managed_sandbox_helper_processes(
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
