use sysinfo::{Pid, Signal, System};

use crate::daemon::AppState;

pub(super) const PROCESS_KILL_SIGNAL: Signal = Signal::Kill;

pub(super) fn signal_pids(pids: &[u32], signal: Signal) -> usize {
    let mut system = System::new();
    system.refresh_processes();
    let mut killed = 0usize;
    for pid in pids {
        if let Some(process) = system.process(Pid::from_u32(*pid)) {
            if process.kill_with(signal).unwrap_or(false) {
                killed += 1;
            }
        }
    }
    killed
}

pub(super) async fn list_provider_processes(
    state: &AppState,
) -> Vec<ctx_providers::adapters::ProviderProcessInfo> {
    let providers = state.providers.provider_adapter_entries().await;
    let mut processes = Vec::new();
    for (_, adapter) in providers {
        processes.extend(adapter.list_processes().await);
    }
    processes
}
