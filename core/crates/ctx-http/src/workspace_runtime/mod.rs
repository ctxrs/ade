#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use std::io::ErrorKind;
#[cfg(test)]
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;
#[cfg(test)]
use std::sync::{Mutex as StdMutex, OnceLock};
#[cfg(test)]
use std::time::Duration;

#[cfg(test)]
use anyhow::{anyhow, Context, Result};
#[cfg(test)]
use ctx_core::models::{Workspace, Worktree};
#[cfg(test)]
use sysinfo::System;

#[cfg(all(test, any(target_os = "macos", target_os = "windows")))]
use crate::resource_utilization::SystemSnapshot;
#[cfg(all(test, any(target_os = "macos", target_os = "windows")))]
use crate::settings::normalize_container_machine_idle_shutdown_seconds;
#[cfg(test)]
use crate::settings::ContainerMachineMemoryProfile;
#[cfg(test)]
use crate::settings::{
    ContainerExecutionSettings, ContainerRuntimeKind, ExecutionMode, ExecutionSettings,
};
#[cfg(all(test, any(target_os = "macos", target_os = "windows")))]
use crate::terminals::TerminalManager;
#[cfg(all(test, any(target_os = "macos", target_os = "windows")))]
use ctx_core::ids::SessionId;
#[cfg(all(test, any(target_os = "macos", target_os = "windows")))]
use ctx_core::models::ExecutionEnvironment;
#[cfg(all(test, any(target_os = "macos", target_os = "windows")))]
use ctx_store::StoreManager;
#[cfg(test)]
use url::Url;

#[cfg(test)]
mod machine;
#[cfg(test)]
mod reclaim_unit_tests;
#[cfg(test)]
mod sandbox_machine_lifecycle;
#[cfg(test)]
mod sandbox_machine_recovery;
#[cfg(test)]
// EXCEPTION: these tests intentionally serialize env-var mutations with a sync lock
// that spans async calls so process-global state cannot interleave across test cases.
#[allow(clippy::await_holding_lock)]
mod tests;

#[cfg(test)]
struct AvfDaemonGatewayProxy {
    gateway_addr: String,
    backend_addr: String,
    handle: tokio::task::JoinHandle<()>,
}

#[cfg(test)]
static AVF_DAEMON_GATEWAY_PROXIES: OnceLock<StdMutex<HashMap<u16, AvfDaemonGatewayProxy>>> =
    OnceLock::new();

#[cfg(test)]
use self::machine::{
    ensure_managed_sandbox_cli_runtime, ensure_managed_sandbox_machine_cache,
    persist_sandbox_machine_cache_to_shared, persist_sandbox_machine_cache_to_shared_best_effort,
    sandbox_machine_cache_root, sandbox_machine_home_root, sandbox_machine_runtime_root,
    sandbox_machine_temp_root, seed_shared_sandbox_machine_cache,
    seed_shared_sandbox_machine_cache_best_effort,
};
#[cfg(test)]
pub(crate) use self::sandbox_machine_lifecycle::SandboxMachineLifecycleExt;
#[cfg(test)]
use self::sandbox_machine_recovery::{
    run_sandbox_machine_init, sandbox_machine_present, sandbox_machine_singleflight_lock,
};
#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use ctx_avf_linux_runtime::AVF_LINUX_HELPER_PATH_ENV;
#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use ctx_harness_runtime::{
    container_image_present, ensure_builder_backend_launch_ready_with_observer,
    launch_ready_detail_message, launch_ready_gap_message, local_runtime_available,
    prefetch_container_image, prewarm_selected_runtime_for_launch_with_observer,
    prewarm_selected_runtime_with_observer, resolve_container_image, runtime_prewarm_ready_message,
    runtime_prewarm_target, sandbox_container_command, sandbox_machine_name,
    selected_runtime_launch_readiness_state, selected_runtime_launch_ready, selected_runtime_state,
    selected_shared_substrate_lifecycle, workspace_launch_ready_message,
};
#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use ctx_harness_runtime::{
    sandbox_engine_ready, selected_sandbox_command_backend, selected_sandbox_command_mode,
    HarnessExecutionPlan, HarnessRuntimeKind, HarnessRuntimeStats, SandboxCommandBackend,
    CTX_AVF_HOST_DATA_ROOT_ENV, CTX_AVF_HOST_WORKTREE_ROOT_ENV, CTX_AVF_WORKSPACE_ID_ENV,
    CTX_AVF_WORKTREE_ID_ENV, CTX_HARNESS_LINUX_SANDBOX_ENV, CTX_HARNESS_RUNTIME_KIND_ENV,
};
#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use ctx_sandbox_container_runtime::CTX_HARNESS_SANDBOX_CLI_PATH_ENV;
#[cfg(test)]
pub(crate) use ctx_sandbox_container_runtime::{
    command_output_message, command_output_with_timeout, sandbox_cli_invocation,
};
#[cfg(test)]
use ctx_sandbox_container_runtime::{
    ensure_managed_default_container_image_tar_with_source, managed_default_image_install_lock,
    sandbox_cli_binary_path,
};
#[cfg(test)]
use ctx_workspace_container::sandbox_machine_required;
#[cfg(test)]
pub(crate) use ctx_workspace_runtime::HarnessRuntimeManager;

#[cfg(test)]
const SANDBOX_MACHINE_CACHE_DIR_ENV: &str = "CTX_SANDBOX_MACHINE_CACHE_DIR";
#[cfg(test)]
const SANDBOX_INFO_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(test)]
const SANDBOX_MACHINE_START_TIMEOUT: Duration = Duration::from_secs(180);
#[cfg(test)]
const SANDBOX_MACHINE_INIT_TIMEOUT: Duration = Duration::from_secs(8 * 60);
#[cfg(test)]
const DEFAULT_PRESET_HOST_MEMORY_MB: u32 = 32 * 1024;
#[cfg(test)]
const SANDBOX_VM_MEMORY_PRESET_FLOOR_MB: u32 = 4096;
#[cfg(test)]
const SANDBOX_VM_MEMORY_ECONOMY_CAP_MB: u32 = 8192;
#[cfg(test)]
const SANDBOX_VM_MEMORY_BALANCED_CAP_MB: u32 = 16 * 1024;
#[cfg(test)]
const SANDBOX_VM_MEMORY_PERFORMANCE_CAP_MB: u32 = 32 * 1024;
#[cfg(test)]
const MI_B: u64 = 1024 * 1024;

#[cfg(test)]
fn avf_daemon_gateway_proxies() -> &'static StdMutex<HashMap<u16, AvfDaemonGatewayProxy>> {
    AVF_DAEMON_GATEWAY_PROXIES.get_or_init(|| StdMutex::new(HashMap::new()))
}

#[cfg(test)]
async fn ensure_avf_guest_gateway_proxy(
    gateway_addr: &str,
    backend_addr: &str,
    port: u16,
) -> Result<()> {
    let mut replaced_existing_proxy = false;
    let existing_handle_to_abort = {
        let mut proxies = avf_daemon_gateway_proxies()
            .lock()
            .map_err(|_| anyhow!("AVF daemon gateway proxy mutex poisoned"))?;
        proxies.retain(|_, proxy| !proxy.handle.is_finished());
        if let Some(existing) = proxies.get(&port) {
            if existing.gateway_addr == gateway_addr && existing.backend_addr == backend_addr {
                return Ok(());
            }
        }
        let removed = proxies.remove(&port).map(|proxy| proxy.handle);
        if removed.is_some() {
            replaced_existing_proxy = true;
        }
        removed
    };

    if let Some(handle) = existing_handle_to_abort {
        handle.abort();
        tokio::task::yield_now().await;
    }

    let listener = loop {
        match tokio::net::TcpListener::bind(gateway_addr).await {
            Ok(listener) => break listener,
            Err(err)
                if replaced_existing_proxy
                    && matches!(
                        err.kind(),
                        ErrorKind::AddrInUse | ErrorKind::AddrNotAvailable
                    ) =>
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
                continue;
            }
            Err(err)
                if matches!(
                    err.kind(),
                    ErrorKind::AddrInUse | ErrorKind::AddrNotAvailable
                ) =>
            {
                tracing::debug!(
                    gateway_addr,
                    backend_addr,
                    "AVF daemon gateway proxy bind is unavailable; assuming a guest-reachable listener already exists"
                );
                return Ok(());
            }
            Err(err) => {
                return Err(err).with_context(|| {
                    format!("binding AVF guest gateway proxy at {gateway_addr} for {backend_addr}")
                });
            }
        }
    };

    let gateway_addr = gateway_addr.to_string();
    let backend_addr = backend_addr.to_string();
    let gateway_addr_for_task = gateway_addr.clone();
    let backend_addr_for_task = backend_addr.clone();
    let handle = tokio::spawn(async move {
        loop {
            let (mut inbound, peer_addr) = match listener.accept().await {
                Ok(parts) => parts,
                Err(err) => {
                    tracing::warn!(
                        gateway_addr = gateway_addr_for_task,
                        backend_addr = backend_addr_for_task,
                        "AVF daemon gateway proxy accept failed: {err}"
                    );
                    break;
                }
            };
            let backend_addr = backend_addr_for_task.clone();
            let gateway_addr = gateway_addr_for_task.clone();
            tokio::spawn(async move {
                match tokio::net::TcpStream::connect(&backend_addr).await {
                    Ok(mut outbound) => {
                        if let Err(err) =
                            tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await
                        {
                            tracing::debug!(
                                gateway_addr,
                                backend_addr,
                                %peer_addr,
                                "AVF daemon gateway proxy relay closed with error: {err}"
                            );
                        }
                    }
                    Err(err) => {
                        tracing::warn!(
                            gateway_addr,
                            backend_addr,
                            %peer_addr,
                            "AVF daemon gateway proxy could not connect to backend: {err}"
                        );
                    }
                }
            });
        }
    });
    let mut proxies = avf_daemon_gateway_proxies()
        .lock()
        .map_err(|_| anyhow!("AVF daemon gateway proxy mutex poisoned"))?;
    if let Some(existing) = proxies.get(&port) {
        if !existing.handle.is_finished() {
            handle.abort();
            return Ok(());
        }
    }
    proxies.insert(
        port,
        AvfDaemonGatewayProxy {
            gateway_addr: gateway_addr.clone(),
            backend_addr: backend_addr.clone(),
            handle,
        },
    );
    tracing::info!(
        gateway_addr,
        backend_addr,
        "started AVF daemon gateway proxy"
    );
    Ok(())
}

#[cfg(test)]
pub(crate) async fn ensure_avf_guest_gateway_proxy_for_test(
    gateway_addr: &str,
    backend_addr: &str,
    port: u16,
) -> Result<()> {
    ensure_avf_guest_gateway_proxy(gateway_addr, backend_addr, port).await
}

#[cfg(test)]
fn detected_host_memory_mb() -> Option<u32> {
    if let Ok(raw) = std::env::var("CTX_TEST_HOST_MEMORY_MB") {
        if let Ok(value) = raw.parse::<u32>() {
            if value > 0 {
                return Some(value);
            }
        }
    }

    let mut system = System::new();
    system.refresh_memory();
    let total_bytes = system.total_memory();
    let total_mb = total_bytes / MI_B;
    if total_mb == 0 {
        return None;
    }
    u32::try_from(total_mb).ok()
}

#[cfg(test)]
fn preset_memory_mb(total_memory_mb: u32, numerator: u32, denominator: u32, cap_mb: u32) -> u32 {
    total_memory_mb
        .saturating_mul(numerator)
        .checked_div(denominator)
        .unwrap_or(SANDBOX_VM_MEMORY_PRESET_FLOOR_MB)
        .clamp(SANDBOX_VM_MEMORY_PRESET_FLOOR_MB, cap_mb)
}

#[cfg(test)]
fn container_machine_memory_mb_for_host_memory(
    settings: &ContainerExecutionSettings,
    host_memory_mb: u32,
) -> u32 {
    match settings.machine.memory_profile {
        ContainerMachineMemoryProfile::Economy => {
            preset_memory_mb(host_memory_mb, 1, 8, SANDBOX_VM_MEMORY_ECONOMY_CAP_MB)
        }
        ContainerMachineMemoryProfile::Balanced => {
            preset_memory_mb(host_memory_mb, 1, 4, SANDBOX_VM_MEMORY_BALANCED_CAP_MB)
        }
        ContainerMachineMemoryProfile::Performance => {
            preset_memory_mb(host_memory_mb, 1, 2, SANDBOX_VM_MEMORY_PERFORMANCE_CAP_MB)
        }
        ContainerMachineMemoryProfile::Custom => settings
            .machine
            .custom_memory_mb
            .unwrap_or(preset_memory_mb(
                host_memory_mb,
                1,
                4,
                SANDBOX_VM_MEMORY_BALANCED_CAP_MB,
            ))
            .max(1024),
    }
}

#[cfg(test)]
fn container_machine_memory_mb(settings: &ContainerExecutionSettings) -> u32 {
    let host_memory_mb = detected_host_memory_mb().unwrap_or(DEFAULT_PRESET_HOST_MEMORY_MB);
    container_machine_memory_mb_for_host_memory(settings, host_memory_mb)
}

#[cfg(test)]
fn sandbox_machine_init_created_machine_grace() -> Duration {
    if cfg!(test) {
        Duration::from_millis(300)
    } else {
        Duration::from_secs(10)
    }
}

#[cfg(test)]
fn sandbox_machine_init_poll_interval() -> Duration {
    if cfg!(test) {
        Duration::from_millis(50)
    } else {
        Duration::from_secs(1)
    }
}

#[cfg(test)]
fn sandbox_machine_ready_timeout() -> Duration {
    Duration::from_millis(300)
}

#[cfg(test)]
fn sandbox_machine_ready_poll_interval() -> Duration {
    Duration::from_millis(25)
}

#[cfg(test)]
use ctx_harness_setup::{observe_log, observe_phase};
