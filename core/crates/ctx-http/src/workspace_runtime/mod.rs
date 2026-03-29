use std::collections::{HashMap, HashSet};
use std::io::ErrorKind;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use futures::StreamExt;
#[cfg(test)]
use sysinfo::System;
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::{fs, io::AsyncWriteExt};

use ctx_core::ids::WorkspaceId;
use ctx_core::models::{Workspace, Worktree};
use ctx_fs::worktrees::worktrees_root;
use serde::{Deserialize, Serialize};

use crate::bundled_assets;
use crate::network_allowlist;
#[cfg(test)]
use crate::resource_utilization::SystemSnapshot;
#[cfg(all(test, any(target_os = "macos", target_os = "windows")))]
use crate::settings::normalize_container_machine_idle_shutdown_seconds;
#[cfg(test)]
use crate::settings::ContainerMachineMemoryProfile;
use crate::settings::{
    ContainerExecutionSettings, ContainerMountMode, ContainerNetworkMode, ContainerRuntimeKind,
    ExecutionMode, ExecutionSettings,
};
#[cfg(all(test, any(target_os = "macos", target_os = "windows")))]
use crate::terminals::TerminalManager;
use crate::updates;
#[cfg(all(test, any(target_os = "macos", target_os = "windows")))]
use ctx_core::ids::SessionId;
#[cfg(all(test, any(target_os = "macos", target_os = "windows")))]
use ctx_core::models::ExecutionEnvironment;
#[cfg(all(test, any(target_os = "macos", target_os = "windows")))]
use ctx_store::StoreManager;
use url::Url;

mod avf_linux_vm;
mod container;
mod image;
mod lifecycle_manager;
mod machine;
mod manager;
mod materialization;
mod network_policy_transition;
#[cfg(test)]
mod reclaim_unit_tests;
mod sandbox_cli;
#[cfg(test)]
mod sandbox_machine_lifecycle;
#[cfg(test)]
mod sandbox_machine_recovery;
mod shared_vm_orchestrator;
mod substrate;

static AVF_DAEMON_GATEWAY_PROXIES: OnceLock<StdMutex<HashMap<u16, tokio::task::JoinHandle<()>>>> =
    OnceLock::new();

pub(crate) use self::avf_linux_vm::build_guest_exec_command as build_avf_linux_guest_exec_command;
pub(crate) use self::avf_linux_vm::helper_path as avf_linux_helper_path;
#[cfg(test)]
pub(crate) use self::avf_linux_vm::override_managed_avf_linux_runtime_source_for_test;
pub(crate) use self::avf_linux_vm::run_guest_exec_capture as run_avf_linux_guest_exec_capture;
#[cfg(test)]
pub(crate) use self::avf_linux_vm::TestManagedAvfLinuxRuntimeSourceGuard;
#[cfg(test)]
pub(crate) use self::avf_linux_vm::AVF_LINUX_HELPER_PATH_ENV;
pub(crate) use self::avf_linux_vm::{
    ensure_shared_vm_ready_with_observer as ensure_avf_linux_shared_vm_ready_with_observer,
    ensure_workspace_vm_ready_with_observer as ensure_avf_linux_workspace_vm_ready_with_observer,
    prefetch_runtime_with_observer as prefetch_avf_linux_runtime_with_observer,
    workspace_vm_data_root as avf_linux_workspace_vm_data_root, AvfLinuxSharedVmLifecycleState,
};
use self::avf_linux_vm::{
    runtime_available as avf_linux_runtime_available, runtime_state as avf_linux_runtime_state,
    runtime_target_label as avf_linux_runtime_target_label,
};
use self::container::sandbox_machine_required;
pub(crate) use self::container::AVF_GUEST_HOST_GATEWAY;
#[cfg(test)]
use self::container::{bind_mount, should_mount_bundle_dir_in_container};
use self::container::{
    build_mounts, container_data_root, container_user, daemon_port_from_url, proxy_runtime_path,
    proxy_runtime_root, rewrite_daemon_url_for_avf_guest, rewrite_daemon_url_for_container,
    should_use_keep_id_userns, verify_disk_isolated_container_mounts,
};
use self::image::ensure_container_image_available;
pub(crate) use self::image::resolve_container_image;
pub use self::image::{
    bundled_default_container_image_tar, container_image_present, container_image_status,
    default_container_image, is_default_container_image, prefetch_container_image,
    prefetch_container_image_with_observer, prefetch_container_startup_artifacts_with_observer,
    ContainerImageStatus,
};
#[cfg(test)]
use self::image::{
    ensure_managed_default_container_image_tar_with_source, managed_default_image_install_lock,
};
pub(crate) use self::lifecycle_manager::{
    SharedSubstrateLifecycleManager, SubstrateLifecycleRecord,
};
use self::machine::sandbox_machine_name;
use self::machine::{
    download_managed_artifact, ManagedArtifactDownloadReporter, ManagedDownloadAggregate,
};
#[cfg(test)]
use self::machine::{
    ensure_managed_sandbox_cli_runtime, ensure_managed_sandbox_machine_cache,
    persist_sandbox_machine_cache_to_shared, persist_sandbox_machine_cache_to_shared_best_effort,
    sandbox_machine_cache_root, sandbox_machine_home_root, sandbox_machine_runtime_root,
    sandbox_machine_temp_root, seed_shared_sandbox_machine_cache,
    seed_shared_sandbox_machine_cache_best_effort,
};
pub(crate) use self::materialization::materialize_sandbox_worktree;
use self::network_policy_transition::apply_container_network_policy;
#[cfg(test)]
use self::sandbox_cli::sandbox_cli_binary_path;
use self::sandbox_cli::{
    command_output_message, container_exists, container_running, ensure_workspace_volume,
};
pub(crate) use self::sandbox_cli::{
    command_output_with_timeout, container_runtime_available, sandbox_cli_env_for_data_root,
    sandbox_cli_invocation, sandbox_container_command, sandbox_engine_ready,
    selected_sandbox_command_backend, SandboxCommandBackend, SHARED_VM_SANDBOX_CLI_GUEST_BIN,
};
#[cfg(test)]
use self::sandbox_machine_recovery::{
    run_sandbox_machine_init, sandbox_machine_present, sandbox_machine_singleflight_lock,
};
pub(crate) use self::shared_vm_orchestrator::SharedVmLifecycleOrchestrator;
pub(crate) use self::substrate::{
    SubstrateShutdownOutcome, SubstrateShutdownReason, SubstrateStartupOutcome,
    SubstrateStartupReason, SubstrateStartupSelection, UbuntuSandboxSubstrate,
};

pub(crate) fn local_runtime_available(data_root: &Path, runtime: &ContainerRuntimeKind) -> bool {
    match runtime {
        ContainerRuntimeKind::NativeContainer => container_runtime_available(data_root),
        ContainerRuntimeKind::SharedVmContainer => avf_linux_runtime_available(),
    }
}

pub(crate) fn runtime_prewarm_target(settings: &ContainerExecutionSettings) -> String {
    match settings.runtime {
        ContainerRuntimeKind::NativeContainer => resolve_container_image(settings),
        ContainerRuntimeKind::SharedVmContainer => avf_linux_runtime_target_label(),
    }
}

pub(crate) async fn selected_runtime_state(
    data_root: &Path,
    settings: &ContainerExecutionSettings,
) -> Result<(bool, bool)> {
    match settings.runtime {
        ContainerRuntimeKind::NativeContainer => {
            let machine_ready = normalize_container_engine_ready_for_runtime(
                sandbox_engine_ready(data_root).await,
            )?;
            let image_present = if machine_ready {
                container_image_present(data_root, &resolve_container_image(settings)).await?
            } else {
                false
            };
            Ok((machine_ready, image_present))
        }
        ContainerRuntimeKind::SharedVmContainer => avf_linux_runtime_state(data_root),
    }
}

pub(crate) async fn prewarm_selected_runtime_with_observer(
    data_root: &Path,
    settings: &ContainerExecutionSettings,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<()> {
    match settings.runtime {
        ContainerRuntimeKind::NativeContainer => {
            let image = resolve_container_image(settings);
            let machine_ready = normalize_container_engine_ready_for_runtime(
                sandbox_engine_ready(data_root).await,
            )?;
            if machine_ready {
                prefetch_container_image_with_observer(data_root, &image, observer).await
            } else {
                prefetch_container_startup_artifacts_with_observer(data_root, &image, observer)
                    .await
            }
        }
        ContainerRuntimeKind::SharedVmContainer => {
            SharedVmLifecycleOrchestrator::new(data_root)
                .prefetch_runtime(settings, observer)
                .await
        }
    }
}

pub(crate) async fn prewarm_selected_runtime_for_launch_with_observer(
    data_root: &Path,
    settings: &ContainerExecutionSettings,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<()> {
    match settings.runtime {
        ContainerRuntimeKind::NativeContainer => {
            ensure_native_container_runtime_launch_ready_with_observer(
                data_root, settings, observer,
            )
            .await
        }
        ContainerRuntimeKind::SharedVmContainer => {
            SharedSubstrateLifecycleManager::new(data_root)
                .ensure_shared_runtime_ready(settings, observer)
                .await?;
            let image = resolve_container_image(settings);
            prefetch_container_image_with_observer(data_root, &image, observer).await
        }
    }
}

pub(crate) fn selected_shared_substrate_lifecycle(
    data_root: &Path,
) -> Result<Option<SubstrateLifecycleRecord>> {
    if !matches!(
        selected_sandbox_command_backend(data_root),
        Ok(SandboxCommandBackend::SharedVmContainer)
    ) {
        return Ok(None);
    }

    let settings = ContainerExecutionSettings {
        runtime: ContainerRuntimeKind::SharedVmContainer,
        ..ContainerExecutionSettings::default()
    };
    SharedSubstrateLifecycleManager::new(data_root)
        .read_shared_runtime_lifecycle(&settings)
        .map(Some)
}

pub(crate) async fn selected_runtime_launch_ready(
    data_root: &Path,
    settings: &ContainerExecutionSettings,
) -> Result<bool> {
    let (vm_ready, image_ready) =
        selected_runtime_launch_readiness_state(data_root, settings).await?;
    Ok(vm_ready && image_ready)
}

pub(crate) async fn selected_runtime_launch_readiness_state(
    data_root: &Path,
    settings: &ContainerExecutionSettings,
) -> Result<(bool, bool)> {
    let substrate = UbuntuSandboxSubstrate::from_runtime_kind(settings.runtime.clone());
    match substrate.substrate {
        ctx_core::models::SandboxSubstrate::NativeContainer => {
            selected_runtime_state(data_root, settings).await
        }
        ctx_core::models::SandboxSubstrate::SharedVmContainer => {
            SharedVmLifecycleOrchestrator::new(data_root)
                .launch_readiness_state(settings)
                .await
        }
    }
}

pub(crate) fn launch_ready_gap_message(
    runtime_kind: ContainerRuntimeKind,
    runtime_target: &str,
    vm_ready: bool,
    image_ready: bool,
) -> String {
    UbuntuSandboxSubstrate::from_runtime_kind(runtime_kind).launch_ready_gap_message(
        runtime_target,
        vm_ready,
        image_ready,
    )
}

#[cfg(test)]
pub(crate) fn launch_ready_detail_message(runtime_kind: &ContainerRuntimeKind) -> &'static str {
    UbuntuSandboxSubstrate::from_runtime_kind(runtime_kind.clone()).launch_ready_detail_message()
}

pub(crate) fn runtime_prewarm_ready_message(
    runtime_kind: &ContainerRuntimeKind,
    launch_ready: bool,
) -> &'static str {
    UbuntuSandboxSubstrate::from_runtime_kind(runtime_kind.clone())
        .runtime_prewarm_ready_message(launch_ready)
}

pub(crate) fn workspace_launch_ready_message(runtime_kind: &ContainerRuntimeKind) -> &'static str {
    UbuntuSandboxSubstrate::from_runtime_kind(runtime_kind.clone()).workspace_launch_ready_message()
}

fn normalize_container_engine_ready_for_runtime(result: Result<bool>) -> Result<bool> {
    match result {
        Ok(value) => Ok(value),
        Err(err) => {
            let lowered = err.to_string().to_ascii_lowercase();
            if lowered.contains("sandbox container cli unavailable")
                || lowered.contains("native sandbox container runtime is unavailable")
            {
                return Ok(false);
            }
            Err(err)
        }
    }
}

async fn ensure_native_container_runtime_engine_ready_with_observer(
    data_root: &Path,
    _settings: &ContainerExecutionSettings,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<()> {
    observe_phase(
        observer,
        HarnessSetupPhase::MachineCheck,
        "checking container runtime",
    );
    if sandbox_engine_ready(data_root).await.unwrap_or(false) {
        observe_log(
            observer,
            HarnessSetupPhase::MachineCheck,
            HarnessSetupLogLevel::Info,
            "local sandbox runtime is already reachable",
        );
        return Ok(());
    }

    if sandbox_machine_required() {
        let machine_name = sandbox_machine_name(data_root);
        observe_phase(
            observer,
            HarnessSetupPhase::MachineStartOrInit,
            "starting local sandbox runtime",
        );
        let mut start = sandbox_container_command(data_root)?;
        start.arg("machine").arg("start").arg(&machine_name);
        let output = command_output_with_timeout(start, SANDBOX_MACHINE_START_TIMEOUT).await?;
        if !output.status.success() {
            let combined = command_output_message(&output);
            let combined_lc = combined.to_ascii_lowercase();
            if !(combined_lc.contains("already running")
                || combined_lc.contains("already starting")
                || combined_lc.contains("already started"))
            {
                if combined_lc.contains("not found")
                    || combined_lc.contains("does not exist")
                    || combined_lc.contains("no machine")
                {
                    anyhow::bail!(
                        "native sandbox container runtime machine '{machine_name}' is not initialized"
                    );
                }
                if combined.is_empty() {
                    anyhow::bail!(
                        "sandbox machine start failed with non-zero exit {}",
                        output.status
                    );
                }
                anyhow::bail!("sandbox machine start failed: {combined}");
            }
            observe_log(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                HarnessSetupLogLevel::Warn,
                "sandbox machine start reported an already-running state; waiting for runtime readiness",
            );
        }
        observe_phase(
            observer,
            HarnessSetupPhase::MachineStartOrInit,
            "waiting for local sandbox runtime readiness",
        );
        let deadline = Instant::now() + sandbox_machine_ready_timeout();
        loop {
            if sandbox_engine_ready(data_root).await.unwrap_or(false) {
                observe_log(
                    observer,
                    HarnessSetupPhase::MachineStartOrInit,
                    HarnessSetupLogLevel::Info,
                    "local sandbox runtime is ready",
                );
                return Ok(());
            }
            if Instant::now() >= deadline {
                anyhow::bail!(
                    "native sandbox container runtime did not become reachable after starting machine '{machine_name}'"
                );
            }
            tokio::time::sleep(sandbox_machine_ready_poll_interval()).await;
        }
    }

    if sandbox_cli_invocation(data_root).is_err() {
        anyhow::bail!(
            "native sandbox container runtime is unavailable; install nerdctl or set {}",
            CTX_HARNESS_SANDBOX_CLI_PATH_ENV
        );
    }

    anyhow::bail!("native sandbox container runtime is installed but not reachable");
}

async fn ensure_native_container_runtime_launch_ready_with_observer(
    data_root: &Path,
    settings: &ContainerExecutionSettings,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<()> {
    ensure_native_container_runtime_engine_ready_with_observer(data_root, settings, observer)
        .await?;
    let image = resolve_container_image(settings);
    prefetch_container_image_with_observer(data_root, &image, observer).await
}

pub(crate) async fn ensure_builder_backend_launch_ready_with_observer(
    data_root: &Path,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<()> {
    match selected_sandbox_command_backend(data_root)? {
        SandboxCommandBackend::NativeContainer => {
            let settings = ContainerExecutionSettings {
                runtime: ContainerRuntimeKind::NativeContainer,
                ..ContainerExecutionSettings::default()
            };
            ensure_native_container_runtime_engine_ready_with_observer(
                data_root, &settings, observer,
            )
            .await
        }
        SandboxCommandBackend::SharedVmContainer => {
            let settings = ContainerExecutionSettings {
                runtime: ContainerRuntimeKind::SharedVmContainer,
                ..ContainerExecutionSettings::default()
            };
            SharedSubstrateLifecycleManager::new(data_root)
                .ensure_shared_runtime_ready(&settings, observer)
                .await
                .map(|_| ())
        }
    }
}

// Default container image for ctx-managed execution.
//
// This must include:
// - iptables (for restricted egress enforcement)
// - /usr/local/bin/ctx-egress-proxy (Linux binary executed inside the container)
const DEFAULT_CONTAINER_IMAGE: &str = "ghcr.io/ctxrs/ctx-harness:ubuntu-24.04";
pub(crate) const CTX_HARNESS_SANDBOX_CLI_PATH_ENV: &str = "CTX_HARNESS_SANDBOX_CLI_PATH";
#[cfg(test)]
const SANDBOX_MACHINE_CACHE_DIR_ENV: &str = "CTX_SANDBOX_MACHINE_CACHE_DIR";
const EGRESS_PROXY_BINARY: &str = "ctx-egress-proxy";
const EGRESS_PROXY_RUNTIME_ID: &str = "ctx-egress-proxy";
const EGRESS_PROXY_CONFIG_NAME: &str = "egress-proxy.json";
const TRANSPARENT_PROXY_PORT: u16 = 15001;
const EGRESS_PROXY_CONTAINER_PATH: &str = "/usr/local/bin/ctx-egress-proxy";
// Dedicated Sandbox machine name prefix for ctx-managed container execution on macOS/Windows.
//
// Final machine name is deterministic per daemon data_root to avoid cross-daemon collisions in
// the sandbox CLI helper's host-global machine temp/socket state.
const CTX_SANDBOX_MACHINE_PREFIX: &str = "ctx";
// In-container root for disk-isolated workspaces (sandbox workspace volume mounted here).
pub(crate) const CTX_CONTAINER_WORKSPACE_ROOT: &str = "/ctx/ws";
pub(crate) const CTX_HARNESS_RUNTIME_KIND_ENV: &str = "CTX_HARNESS_RUNTIME_KIND";
pub(crate) const CTX_HARNESS_LINUX_SANDBOX_ENV: &str = "CTX_HARNESS_LINUX_SANDBOX";
pub(crate) const CTX_AVF_HOST_DATA_ROOT_ENV: &str = "CTX_AVF_HOST_DATA_ROOT";
pub(crate) const CTX_AVF_WORKSPACE_ID_ENV: &str = "CTX_AVF_WORKSPACE_ID";
pub(crate) const CTX_AVF_WORKTREE_ID_ENV: &str = "CTX_AVF_WORKTREE_ID";
pub(crate) const CTX_AVF_HOST_WORKTREE_ROOT_ENV: &str = "CTX_AVF_HOST_WORKTREE_ROOT";
const SANDBOX_INFO_TIMEOUT: Duration = Duration::from_secs(5);
const SANDBOX_MACHINE_START_TIMEOUT: Duration = Duration::from_secs(180);
// Bound machine init so wedged sandbox CLI subprocesses cannot stall launch indefinitely.
#[cfg(test)]
const SANDBOX_MACHINE_INIT_TIMEOUT: Duration = Duration::from_secs(8 * 60);
// First boot can be slow on fresh installs (image download + provisioning), but readiness loops
// must remain bounded tightly enough to surface actionable errors quickly.
const SANDBOX_MACHINE_READY_TIMEOUT: Duration = Duration::from_secs(2 * 60);
const SANDBOX_OP_TIMEOUT: Duration = Duration::from_secs(60);
const SANDBOX_IMAGE_LOAD_TIMEOUT: Duration = Duration::from_secs(10 * 60);
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

fn avf_daemon_gateway_proxies() -> &'static StdMutex<HashMap<u16, tokio::task::JoinHandle<()>>> {
    AVF_DAEMON_GATEWAY_PROXIES.get_or_init(|| StdMutex::new(HashMap::new()))
}

async fn ensure_avf_guest_gateway_proxy(
    gateway_addr: &str,
    backend_addr: &str,
    port: u16,
) -> Result<()> {
    {
        let mut proxies = avf_daemon_gateway_proxies()
            .lock()
            .map_err(|_| anyhow!("AVF daemon gateway proxy mutex poisoned"))?;
        proxies.retain(|_, handle| !handle.is_finished());
        if proxies.contains_key(&port) {
            return Ok(());
        }
    }

    match tokio::net::TcpListener::bind(gateway_addr).await {
        Ok(listener) => {
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
                if !existing.is_finished() {
                    handle.abort();
                    return Ok(());
                }
            }
            proxies.insert(port, handle);
            tracing::info!(
                gateway_addr,
                backend_addr,
                "started AVF daemon gateway proxy"
            );
            Ok(())
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
            Ok(())
        }
        Err(err) => Err(err).with_context(|| {
            format!("binding AVF guest gateway proxy at {gateway_addr} for {backend_addr}")
        }),
    }
}

async fn resolve_daemon_url_for_avf_guest(daemon_url: &str) -> Result<String> {
    let Ok(url) = Url::parse(daemon_url) else {
        return Ok(daemon_url.to_string());
    };
    let Some(host) = url.host_str() else {
        return Ok(daemon_url.to_string());
    };
    if !matches!(host, "127.0.0.1" | "localhost" | "::1") {
        return Ok(daemon_url.to_string());
    }
    let Some(port) = url.port_or_known_default() else {
        return Ok(daemon_url.to_string());
    };
    let gateway_addr = format!("{AVF_GUEST_HOST_GATEWAY}:{port}");
    match tokio::time::timeout(
        Duration::from_millis(500),
        tokio::net::TcpStream::connect(&gateway_addr),
    )
    .await
    {
        Ok(Ok(_)) => Ok(rewrite_daemon_url_for_avf_guest(daemon_url)),
        Ok(Err(_)) | Err(_) => {
            let backend_host = match host {
                "localhost" | "::1" => "127.0.0.1",
                other => other,
            };
            let backend_addr = format!("{backend_host}:{port}");
            ensure_avf_guest_gateway_proxy(&gateway_addr, &backend_addr, port).await?;
            Ok(rewrite_daemon_url_for_avf_guest(daemon_url))
        }
    }
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
    #[cfg(test)]
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

fn sandbox_machine_ready_timeout() -> Duration {
    if cfg!(test) {
        Duration::from_millis(300)
    } else {
        SANDBOX_MACHINE_READY_TIMEOUT
    }
}

fn sandbox_machine_ready_poll_interval() -> Duration {
    if cfg!(test) {
        Duration::from_millis(25)
    } else {
        Duration::from_secs(1)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum HarnessSetupPhase {
    ArtifactDownload,
    MachineCheck,
    MachineStartOrInit,
    ImageCheck,
    ImageLoad,
    ContainerCheck,
    ContainerStartOrCreate,
    RuntimeNetworkSetup,
    Ready,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum HarnessSetupLogLevel {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HarnessSetupDownloadStatus {
    pub artifact: String,
    pub downloaded_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_per_sec: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HarnessSetupProgressUpdate {
    pub phase: HarnessSetupPhase,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_download: Option<HarnessSetupDownloadStatus>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ManagedContainerBootstrapOverrides {
    pub(crate) default_image_source: Option<bundled_assets::ManagedArtifactSource>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HarnessRuntimeStats {
    pub container_count: usize,
    pub container_allowlist_entries: usize,
    pub container_external_mounts: usize,
    pub container_egress_guards: usize,
}

pub trait HarnessSetupObserver: Send + Sync {
    fn on_phase(&self, phase: HarnessSetupPhase, message: &str);
    fn on_log(&self, phase: HarnessSetupPhase, level: HarnessSetupLogLevel, message: &str);
    fn on_progress(&self, _progress: HarnessSetupProgressUpdate) {}
}

fn observe_phase(
    observer: Option<&dyn HarnessSetupObserver>,
    phase: HarnessSetupPhase,
    message: &str,
) {
    if let Some(observer) = observer {
        observer.on_phase(phase, message);
    }
}

fn observe_log(
    observer: Option<&dyn HarnessSetupObserver>,
    phase: HarnessSetupPhase,
    level: HarnessSetupLogLevel,
    message: &str,
) {
    if let Some(observer) = observer {
        observer.on_log(phase, level, message);
    }
}

fn observe_progress(
    observer: Option<&dyn HarnessSetupObserver>,
    progress: HarnessSetupProgressUpdate,
) {
    if let Some(observer) = observer {
        observer.on_progress(progress);
    }
}

pub(crate) fn workspace_container_name(workspace_id: WorkspaceId) -> String {
    format!("ctx-harness-{}", workspace_id.0)
}

#[derive(Debug, Clone)]
pub enum HarnessRuntimeKind {
    Host,
    NativeContainer { name: String },
    SharedVmContainer,
}

impl HarnessRuntimeKind {
    pub fn is_linux_sandbox(&self) -> bool {
        !matches!(self, Self::Host)
    }
}

#[derive(Debug, Clone)]
pub struct HarnessExecutionPlan {
    pub runtime: HarnessRuntimeKind,
    pub env_overrides: HashMap<String, String>,
}

impl HarnessExecutionPlan {
    pub fn is_linux_sandbox(&self) -> bool {
        self.runtime.is_linux_sandbox()
            || self
                .env_overrides
                .get(CTX_HARNESS_LINUX_SANDBOX_ENV)
                .is_some_and(|value| value == "1")
    }

    pub fn runtime_data_root(&self) -> Option<&Path> {
        self.env_overrides.get("CTX_DATA_ROOT").map(Path::new)
    }
}

#[derive(Debug, Clone)]
struct HarnessContainer {
    name: String,
    mount_mode: ContainerMountMode,
    network_mode: ContainerNetworkMode,
    allowlist: Vec<String>,
    external_mounts: HashSet<String>,
    egress_guard: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CachedContainerAction {
    Reuse,
    Reconfigure,
    Recreate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContainerReadinessState {
    MachineReady,
    RuntimeReady,
}

struct EnsureContainerRequest<'a> {
    workspace: &'a Workspace,
    worktree: Option<&'a Worktree>,
    settings: &'a ContainerExecutionSettings,
    daemon_host: &'a str,
    daemon_port: u16,
    observer: Option<&'a dyn HarnessSetupObserver>,
    readiness: ContainerReadinessState,
}

fn cached_container_action(
    cached: &HarnessContainer,
    settings: &ContainerExecutionSettings,
    external_mounts: &HashSet<String>,
) -> CachedContainerAction {
    if cached.mount_mode != settings.mount_mode || cached.external_mounts != *external_mounts {
        return CachedContainerAction::Recreate;
    }
    if cached.network_mode != settings.network_mode || cached.allowlist != settings.allowlist {
        return CachedContainerAction::Reconfigure;
    }
    CachedContainerAction::Reuse
}

#[derive(Debug, Clone, Serialize)]
pub struct HarnessContainerStatus {
    pub name: String,
    pub running: bool,
    pub known: bool,
    pub mount_mode: Option<ContainerMountMode>,
    pub network_mode: Option<ContainerNetworkMode>,
    pub allowlist: Vec<String>,
    pub egress_guard: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
struct TransparentProxyConfig {
    listen: String,
    mode: ContainerNetworkMode,
    allowlist: Vec<String>,
    max_peek_bytes: usize,
}

pub struct HarnessRuntimeManager {
    data_root: PathBuf,
    containers: Mutex<HashMap<WorkspaceId, HarnessContainer>>,
    last_activity: StdMutex<Instant>,
    active_runtime_operations: AtomicUsize,
    active_prewarm_artifact_operations: AtomicUsize,
    ops_events: crate::ops_events::OpsEvents,
}

#[cfg(test)]
// EXCEPTION: these tests intentionally serialize env-var mutations with a sync lock
// that spans async calls so process-global state cannot interleave across test cases.
#[allow(clippy::await_holding_lock)]
mod tests;
