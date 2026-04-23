use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use ctx_core::ids::WorkspaceId;
use ctx_core::models::{Workspace, Worktree};
use ctx_harness_setup::{observe_log, observe_phase, HarnessSetupObserver, HarnessSetupPhase};
use ctx_sandbox_container_runtime::{
    command_output_message, command_output_with_timeout, container_exists, container_image_present,
    container_running, ensure_container_image_available, ensure_workspace_volume,
    force_reload_default_container_image, is_default_container_image, resolve_container_image,
    sandbox_container_command, SandboxCommandMode,
};
use ctx_sandbox_contract::{
    ContainerExecutionSettings, ContainerMountMode, ContainerNetworkMode, ContainerRuntimeKind,
};
use serde::Serialize;
use tokio::process::Command;
use tokio::sync::Mutex;

mod allowlist;
mod container;
mod network_policy_transition;

pub use container::{
    bind_mount, build_mounts, container_data_root, container_user, daemon_port_from_url,
    rewrite_daemon_url_for_avf_guest, rewrite_daemon_url_for_container, sandbox_machine_required,
    should_mount_bundle_dir_in_container, should_use_keep_id_userns, workspace_container_hostname,
    MountPlan, AVF_GUEST_HOST_GATEWAY, CONTAINER_TERMINAL_HOME, CONTAINER_TERMINAL_USER,
};
pub use network_policy_transition::{
    apply_container_network_policy, transparent_proxy_policy, AppliedContainerNetworkPolicy,
};

const SANDBOX_OP_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
pub struct WorkspaceContainer {
    pub name: String,
    pub mount_mode: ContainerMountMode,
    pub network_mode: ContainerNetworkMode,
    pub allowlist: Vec<String>,
    pub external_mounts: HashSet<String>,
    pub egress_guard: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CachedContainerAction {
    Reuse,
    Reconfigure,
    Recreate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceContainerReadiness {
    MachineReady,
    RuntimeReady,
}

pub struct EnsureWorkspaceContainerRequest<'a> {
    pub workspace: &'a Workspace,
    pub worktree: Option<&'a Worktree>,
    pub settings: &'a ContainerExecutionSettings,
    pub daemon_host: &'a str,
    pub daemon_port: u16,
    pub observer: Option<&'a dyn HarnessSetupObserver>,
    pub readiness: WorkspaceContainerReadiness,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceContainerStatus {
    pub name: String,
    pub running: bool,
    pub known: bool,
    pub mount_mode: Option<ContainerMountMode>,
    pub network_mode: Option<ContainerNetworkMode>,
    pub allowlist: Vec<String>,
    pub egress_guard: Option<bool>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct WorkspaceContainerStats {
    pub container_count: usize,
    pub container_allowlist_entries: usize,
    pub container_external_mounts: usize,
    pub container_egress_guards: usize,
}

pub struct WorkspaceContainerOwner {
    data_root: PathBuf,
    containers: Mutex<HashMap<WorkspaceId, WorkspaceContainer>>,
}

pub fn workspace_container_name(workspace_id: WorkspaceId) -> String {
    format!("ctx-harness-{}", workspace_id.0)
}

pub async fn list_running_workspace_container_names(
    data_root: &std::path::Path,
    mode: &SandboxCommandMode,
) -> Result<Vec<String>> {
    let mut cmd = sandbox_container_command(data_root, mode)?;
    cmd.arg("container")
        .arg("ls")
        .arg("--format")
        .arg("{{.Names}}");
    let output = command_output_with_timeout(cmd, SANDBOX_OP_TIMEOUT).await?;
    if !output.status.success() {
        let combined = command_output_message(&output);
        if combined.is_empty() {
            anyhow::bail!(
                "container list failed while probing running workspace containers (status: {})",
                output.status
            );
        }
        anyhow::bail!(
            "container list failed while probing running workspace containers: {combined}"
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|name| !name.is_empty() && name.starts_with("ctx-harness-"))
        .map(ToOwned::to_owned)
        .collect())
}

pub fn cached_container_action(
    cached: &WorkspaceContainer,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SandboxContainerLaunchNetworking {
    network: Option<&'static str>,
    add_host: &'static str,
}

fn sandbox_container_launch_networking(
    settings: &ContainerExecutionSettings,
) -> SandboxContainerLaunchNetworking {
    SandboxContainerLaunchNetworking {
        network: if matches!(settings.runtime, ContainerRuntimeKind::SharedVmContainer) {
            None
        } else {
            Some("slirp4netns:allow_host_loopback=true")
        },
        add_host: "host.containers.internal:host-gateway",
    }
}

fn append_sandbox_container_launch_network_args(
    cmd: &mut Command,
    settings: &ContainerExecutionSettings,
) {
    let networking = sandbox_container_launch_networking(settings);
    if let Some(network) = networking.network {
        cmd.arg("--network").arg(network);
    }
    cmd.arg("--cap-add").arg("NET_ADMIN");
    cmd.arg("--add-host").arg(networking.add_host);
}

impl WorkspaceContainerOwner {
    pub fn new(data_root: PathBuf) -> Self {
        Self {
            data_root,
            containers: Mutex::new(HashMap::new()),
        }
    }

    pub async fn stats(&self) -> WorkspaceContainerStats {
        let containers = self.containers.lock().await;
        let mut stats = WorkspaceContainerStats {
            container_count: containers.len(),
            ..WorkspaceContainerStats::default()
        };
        for container in containers.values() {
            stats.container_allowlist_entries += container.allowlist.len();
            stats.container_external_mounts += container.external_mounts.len();
            if container.egress_guard {
                stats.container_egress_guards += 1;
            }
        }
        stats
    }

    pub async fn put_cached_container_for_test(
        &self,
        workspace_id: WorkspaceId,
        container: WorkspaceContainer,
    ) {
        self.containers.lock().await.insert(workspace_id, container);
    }

    pub async fn workspace_container_exists(
        &self,
        mode: &SandboxCommandMode,
        workspace_id: WorkspaceId,
    ) -> Result<bool> {
        container_exists(
            &self.data_root,
            mode,
            &workspace_container_name(workspace_id),
        )
        .await
    }

    pub async fn container_status(
        &self,
        mode: &SandboxCommandMode,
        workspace_id: WorkspaceId,
    ) -> Result<Option<WorkspaceContainerStatus>> {
        let name = workspace_container_name(workspace_id);
        let present = container_exists(&self.data_root, mode, &name).await?;
        if !present {
            return Ok(None);
        }
        let running = container_running(&self.data_root, mode, &name)
            .await?
            .unwrap_or(false);
        let cached = {
            let containers = self.containers.lock().await;
            containers.get(&workspace_id).cloned()
        };
        let (known, mount_mode, network_mode, allowlist, egress_guard) =
            if let Some(container) = cached {
                (
                    true,
                    Some(container.mount_mode),
                    Some(container.network_mode),
                    container.allowlist,
                    Some(container.egress_guard),
                )
            } else {
                (false, None, None, Vec::new(), None)
            };
        Ok(Some(WorkspaceContainerStatus {
            name,
            running,
            known,
            mount_mode,
            network_mode,
            allowlist,
            egress_guard,
        }))
    }

    pub async fn running_workspace_container_names(
        &self,
        mode: &SandboxCommandMode,
    ) -> Result<Vec<String>> {
        list_running_workspace_container_names(&self.data_root, mode).await
    }

    pub async fn stop_container(
        &self,
        mode: &SandboxCommandMode,
        workspace_id: WorkspaceId,
    ) -> Result<bool> {
        let name = workspace_container_name(workspace_id);
        let present = container_exists(&self.data_root, mode, &name).await?;
        if !present {
            self.containers.lock().await.remove(&workspace_id);
            return Ok(false);
        }

        self.containers.lock().await.remove(&workspace_id);
        let mut cmd = sandbox_container_command(&self.data_root, mode)?;
        cmd.arg("rm").arg("-f").arg(&name);
        let output = command_output_with_timeout(cmd, SANDBOX_OP_TIMEOUT).await?;
        if output.status.success() {
            return Ok(true);
        }
        let combined = command_output_message(&output);
        if combined.is_empty() {
            anyhow::bail!("container rm failed for {name} (status: {})", output.status);
        }
        anyhow::bail!("container rm failed for {name}: {combined}");
    }

    pub async fn remove_workspace_volume(
        &self,
        mode: &SandboxCommandMode,
        workspace_id: WorkspaceId,
    ) -> Result<bool> {
        let name = format!("ctx-ws-{}", workspace_id.0);
        let mut inspect = sandbox_container_command(&self.data_root, mode)?;
        inspect.arg("volume").arg("inspect").arg(&name);
        let out = command_output_with_timeout(inspect, SANDBOX_OP_TIMEOUT).await?;
        if !out.status.success() {
            return Ok(false);
        }

        let mut cmd = sandbox_container_command(&self.data_root, mode)?;
        cmd.arg("volume").arg("rm").arg("-f").arg(&name);
        let out = command_output_with_timeout(cmd, SANDBOX_OP_TIMEOUT).await?;
        if out.status.success() {
            return Ok(true);
        }
        let combined = command_output_message(&out);
        if combined.is_empty() {
            anyhow::bail!(
                "container volume rm failed for {name} (status: {})",
                out.status
            );
        }
        anyhow::bail!("container volume rm failed for {name}: {combined}");
    }

    pub async fn ensure_after_machine_ready(
        &self,
        mode: &SandboxCommandMode,
        request: EnsureWorkspaceContainerRequest<'_>,
    ) -> Result<WorkspaceContainer> {
        let EnsureWorkspaceContainerRequest {
            workspace,
            worktree,
            settings,
            daemon_host,
            daemon_port,
            observer,
            readiness,
        } = request;

        let name = workspace_container_name(workspace.id);
        let image = resolve_container_image(settings.image.as_deref());
        if matches!(settings.mount_mode, ContainerMountMode::DiskIsolated) {
            observe_log(
                observer,
                HarnessSetupPhase::ContainerCheck,
                ctx_sandbox_container_runtime::HarnessSetupLogLevel::Info,
                "ensuring workspace volume for disk-isolated mode",
            );
            let _ = ensure_workspace_volume(&self.data_root, mode, workspace.id).await?;
        }
        let mount_plan = container::build_mounts(&self.data_root, workspace, worktree, settings);
        let mut recreate = false;
        observe_phase(
            observer,
            HarnessSetupPhase::ContainerCheck,
            "checking existing workspace container",
        );
        let cached_container = {
            let containers = self.containers.lock().await;
            containers.get(&workspace.id).cloned()
        };
        if let Some(container) = cached_container {
            match cached_container_action(&container, settings, &mount_plan.external_mounts) {
                CachedContainerAction::Reuse => {
                    let exists = container_exists(&self.data_root, mode, &name).await?;
                    let running = if exists {
                        container_running(&self.data_root, mode, &name)
                            .await?
                            .unwrap_or(false)
                    } else {
                        false
                    };
                    if exists && running {
                        observe_log(
                            observer,
                            HarnessSetupPhase::ContainerCheck,
                            ctx_sandbox_container_runtime::HarnessSetupLogLevel::Info,
                            "container already ready in runtime cache",
                        );
                        return Ok(container);
                    }
                    observe_log(
                        observer,
                        HarnessSetupPhase::ContainerCheck,
                        ctx_sandbox_container_runtime::HarnessSetupLogLevel::Info,
                        if exists {
                            "runtime cache entry stale; workspace container is stopped and will be restarted"
                        } else {
                            "runtime cache entry stale; workspace container is missing and will be recreated"
                        },
                    );
                    self.containers.lock().await.remove(&workspace.id);
                }
                CachedContainerAction::Reconfigure => {
                    observe_log(
                        observer,
                        HarnessSetupPhase::ContainerCheck,
                        ctx_sandbox_container_runtime::HarnessSetupLogLevel::Info,
                        "container network policy changed; reconfiguring",
                    );
                }
                CachedContainerAction::Recreate => {
                    observe_log(
                        observer,
                        HarnessSetupPhase::ContainerCheck,
                        ctx_sandbox_container_runtime::HarnessSetupLogLevel::Info,
                        "container configuration changed; recreating",
                    );
                    self.containers.lock().await.remove(&workspace.id);
                    recreate = true;
                }
            }
        }

        if recreate {
            observe_phase(
                observer,
                HarnessSetupPhase::ContainerStartOrCreate,
                "recreating workspace container",
            );
            let mut cmd = sandbox_container_command(&self.data_root, mode)?;
            cmd.arg("rm").arg("-f").arg(&name);
            let _ = command_output_with_timeout(cmd, SANDBOX_OP_TIMEOUT).await;
        }

        let mut recreate_for_terminal_contract = false;
        loop {
            let exists = if recreate || recreate_for_terminal_contract {
                false
            } else {
                container_exists(&self.data_root, mode, &name).await?
            };

            if exists {
                let running = container_running(&self.data_root, mode, &name)
                    .await?
                    .unwrap_or(false);
                if !running {
                    observe_phase(
                        observer,
                        HarnessSetupPhase::ContainerStartOrCreate,
                        "starting existing workspace container",
                    );
                    let mut cmd = sandbox_container_command(&self.data_root, mode)?;
                    cmd.arg("start").arg(&name);
                    let output = command_output_with_timeout(cmd, SANDBOX_OP_TIMEOUT).await?;
                    if !output.status.success() {
                        let combined = command_output_message(&output);
                        if combined.is_empty() {
                            anyhow::bail!(
                                "container start failed for {name} (status: {})",
                                output.status
                            );
                        }
                        anyhow::bail!("container start failed for {name}: {combined}");
                    }
                } else {
                    observe_log(
                        observer,
                        HarnessSetupPhase::ContainerCheck,
                        ctx_sandbox_container_runtime::HarnessSetupLogLevel::Info,
                        "workspace container already running",
                    );
                }
            } else {
                let requires_front_loaded_image_readiness = !(settings.runtime
                    == ContainerRuntimeKind::NativeContainer
                    && readiness == WorkspaceContainerReadiness::RuntimeReady);
                if requires_front_loaded_image_readiness {
                    self.ensure_container_image_ready(mode, settings, observer)
                        .await?;
                }
                observe_phase(
                    observer,
                    HarnessSetupPhase::ContainerStartOrCreate,
                    "creating workspace container",
                );
                let mut cmd = sandbox_container_command(&self.data_root, mode)?;
                cmd.arg("run").arg("-d").arg("--name").arg(&name);
                cmd.arg("--hostname")
                    .arg(container::workspace_container_hostname(workspace));
                if container::should_use_keep_id_userns() {
                    cmd.arg("--userns=keep-id");
                }
                if let Some(user) = container::container_user() {
                    cmd.arg("--user").arg(user);
                }
                append_sandbox_container_launch_network_args(&mut cmd, settings);
                for mount in &mount_plan.mounts {
                    cmd.arg("--mount").arg(mount);
                }
                cmd.arg(&image);
                cmd.arg("/bin/sh")
                    .arg("-c")
                    .arg("while true; do sleep 100000; done");
                let output = command_output_with_timeout(cmd, SANDBOX_OP_TIMEOUT).await?;
                if !output.status.success() {
                    let combined = command_output_message(&output);
                    let combined_lower = combined.to_ascii_lowercase();
                    let can_adopt_existing = combined_lower.contains("name-store error")
                        || combined_lower.contains("already used by id");
                    if can_adopt_existing && container_exists(&self.data_root, mode, &name).await? {
                        observe_log(
                            observer,
                            HarnessSetupPhase::ContainerStartOrCreate,
                            ctx_sandbox_container_runtime::HarnessSetupLogLevel::Warn,
                            "container create reported an existing name; adopting the existing workspace container",
                        );
                        let running = container_running(&self.data_root, mode, &name)
                            .await?
                            .unwrap_or(false);
                        if !running {
                            observe_log(
                                observer,
                                HarnessSetupPhase::ContainerStartOrCreate,
                                ctx_sandbox_container_runtime::HarnessSetupLogLevel::Info,
                                "adopted workspace container is stopped; starting it",
                            );
                            let mut start = sandbox_container_command(&self.data_root, mode)?;
                            start.arg("start").arg(&name);
                            let output =
                                command_output_with_timeout(start, SANDBOX_OP_TIMEOUT).await?;
                            if !output.status.success() {
                                let combined = command_output_message(&output);
                                if combined.is_empty() {
                                    anyhow::bail!(
                                        "container start failed for {name} (status: {})",
                                        output.status
                                    );
                                }
                                anyhow::bail!("container start failed for {name}: {combined}");
                            }
                        }
                    } else if combined.is_empty() {
                        anyhow::bail!(
                            "container run failed for {name} (status: {})",
                            output.status
                        );
                    } else {
                        anyhow::bail!("container run failed for {name}: {combined}");
                    }
                }
            }

            match container::sync_container_terminal_identity(&self.data_root, mode, &name).await {
                Ok(()) => break,
                Err(err) if container::container_terminal_identity_missing_sudo(&err) => {
                    if recreate_for_terminal_contract {
                        return Err(err.context(
                            "workspace container still lacks terminal sudo support after recreation",
                        ));
                    }
                    if is_default_container_image(&image) {
                        observe_log(
                            observer,
                            HarnessSetupPhase::ImageLoad,
                            ctx_sandbox_container_runtime::HarnessSetupLogLevel::Info,
                            "reloading the default harness image to apply the terminal identity contract",
                        );
                        force_reload_default_container_image(&self.data_root, mode, observer)
                            .await?;
                    }
                    observe_log(
                        observer,
                        HarnessSetupPhase::ContainerStartOrCreate,
                        ctx_sandbox_container_runtime::HarnessSetupLogLevel::Info,
                        "workspace container predates the terminal identity contract; recreating",
                    );
                    let mut cmd = sandbox_container_command(&self.data_root, mode)?;
                    cmd.arg("rm").arg("-f").arg(&name);
                    let output = command_output_with_timeout(cmd, SANDBOX_OP_TIMEOUT).await?;
                    if !output.status.success() {
                        let combined = command_output_message(&output);
                        if combined.is_empty() {
                            anyhow::bail!(
                                "container rm failed for {name} while refreshing terminal identity contract (status: {})",
                                output.status
                            );
                        }
                        anyhow::bail!(
                            "container rm failed for {name} while refreshing terminal identity contract: {combined}"
                        );
                    }
                    recreate_for_terminal_contract = true;
                }
                Err(err) => return Err(err),
            }
        }

        if matches!(settings.mount_mode, ContainerMountMode::DiskIsolated) {
            container::verify_disk_isolated_container_mounts(
                &self.data_root,
                mode,
                workspace,
                &name,
            )
            .await?;
        }

        observe_phase(
            observer,
            HarnessSetupPhase::RuntimeNetworkSetup,
            "configuring container network policy",
        );
        let egress_guard = network_policy_transition::apply_container_network_policy(
            &self.data_root,
            mode,
            workspace.id,
            &name,
            settings,
            daemon_host,
            daemon_port,
        )
        .await?
        .egress_guard;
        observe_log(
            observer,
            HarnessSetupPhase::RuntimeNetworkSetup,
            ctx_sandbox_container_runtime::HarnessSetupLogLevel::Info,
            "container network policy configured",
        );

        let container = WorkspaceContainer {
            name: name.clone(),
            mount_mode: settings.mount_mode.clone(),
            network_mode: settings.network_mode.clone(),
            allowlist: settings.allowlist.clone(),
            external_mounts: mount_plan.external_mounts,
            egress_guard,
        };
        self.containers
            .lock()
            .await
            .insert(workspace.id, container.clone());
        Ok(container)
    }

    async fn ensure_container_image_ready(
        &self,
        mode: &SandboxCommandMode,
        settings: &ContainerExecutionSettings,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<()> {
        let image = resolve_container_image(settings.image.as_deref());
        observe_phase(
            observer,
            HarnessSetupPhase::ImageCheck,
            "checking harness image availability",
        );
        if container_image_present(&self.data_root, mode, &image).await? {
            observe_log(
                observer,
                HarnessSetupPhase::ImageCheck,
                ctx_sandbox_container_runtime::HarnessSetupLogLevel::Info,
                "harness image already present",
            );
            return Ok(());
        }
        observe_phase(
            observer,
            HarnessSetupPhase::ImageLoad,
            "loading harness image into local sandbox runtime",
        );
        ensure_container_image_available(&self.data_root, mode, &image, observer).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    #[cfg(unix)]
    struct EnvVarGuard {
        key: &'static str,
        prev: Option<String>,
    }

    #[cfg(unix)]
    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let prev = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, prev }
        }
    }

    #[cfg(unix)]
    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(prev) = self.prev.take() {
                std::env::set_var(self.key, prev);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    #[cfg(unix)]
    fn env_var_test_lock() -> &'static tokio::sync::Mutex<()> {
        static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
    }

    fn sample_container_settings() -> ContainerExecutionSettings {
        ContainerExecutionSettings::default()
    }

    fn sample_cached_container() -> WorkspaceContainer {
        WorkspaceContainer {
            name: "ctx-harness-test".to_string(),
            mount_mode: ContainerMountMode::DiskIsolated,
            network_mode: ContainerNetworkMode::LlmOnly,
            allowlist: Vec::new(),
            external_mounts: HashSet::from(["/tmp/hooks".to_string()]),
            egress_guard: true,
        }
    }

    #[test]
    fn cached_container_action_reuses_when_mounts_and_network_match() {
        let cached = sample_cached_container();
        let settings = sample_container_settings();
        let action = cached_container_action(&cached, &settings, &cached.external_mounts);
        assert_eq!(action, CachedContainerAction::Reuse);
    }

    #[test]
    fn cached_container_action_recreates_when_mount_mode_changes() {
        let cached = sample_cached_container();
        let mut settings = sample_container_settings();
        settings.mount_mode = ContainerMountMode::Legacy;
        let action = cached_container_action(&cached, &settings, &cached.external_mounts);
        assert_eq!(action, CachedContainerAction::Recreate);
    }

    #[test]
    fn cached_container_action_reconfigures_when_allowlist_changes() {
        let cached = sample_cached_container();
        let mut settings = sample_container_settings();
        settings.allowlist = vec!["example.com".to_string()];
        let action = cached_container_action(&cached, &settings, &cached.external_mounts);
        assert_eq!(action, CachedContainerAction::Reconfigure);
    }

    #[test]
    fn bundle_dir_mount_policy_matches_platform_expectations() {
        if cfg!(target_os = "linux") {
            assert!(container::should_mount_bundle_dir_in_container(Path::new(
                "/Applications/ctx.app/Contents/Resources/bundles"
            )));
            return;
        }
        if cfg!(target_os = "macos") || cfg!(target_os = "windows") {
            assert!(!container::should_mount_bundle_dir_in_container(Path::new(
                "/Applications/ctx.app/Contents/Resources/bundles"
            )));
            let home_var = if cfg!(target_os = "windows") {
                "USERPROFILE"
            } else {
                "HOME"
            };
            if let Some(home) = std::env::var_os(home_var).map(PathBuf::from) {
                assert!(!container::should_mount_bundle_dir_in_container(
                    &home.join("ctx-bundles")
                ));
            }
        }
    }

    #[test]
    fn shared_vm_container_launch_networking_uses_default_bridge() {
        let settings = ContainerExecutionSettings {
            runtime: ContainerRuntimeKind::SharedVmContainer,
            ..ContainerExecutionSettings::default()
        };
        let networking = sandbox_container_launch_networking(&settings);
        assert_eq!(networking.network, None);
        assert_eq!(networking.add_host, "host.containers.internal:host-gateway");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn running_workspace_container_names_filters_ctx_workspace_containers() {
        let _serial = env_var_test_lock().lock().await;
        let temp = tempfile::tempdir().expect("tempdir");
        let cli_path = temp.path().join("sandbox-cli.sh");
        let log_path = temp.path().join("sandbox-cli.log");
        std::fs::write(
            &cli_path,
            format!(
                "#!/bin/sh\nLOG=\"{log}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"container\" ] && [ \"$2\" = \"ls\" ] && [ \"$3\" = \"--format\" ] && [ \"$4\" = \"{{{{.Names}}}}\" ]; then\n  printf 'ctx-harness-one\\nctx-harness-two\\npostgres\\n\\n'\n  exit 0\nfi\necho \"unexpected invocation: $*\" >&2\nexit 1\n",
                log = log_path.display(),
            ),
        )
        .expect("write sandbox cli shim");
        std::fs::set_permissions(&cli_path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod sandbox cli shim");
        let _guard = EnvVarGuard::set(
            ctx_sandbox_container_runtime::CTX_HARNESS_SANDBOX_CLI_PATH_ENV,
            &cli_path.to_string_lossy(),
        );

        let names = list_running_workspace_container_names(
            temp.path(),
            &SandboxCommandMode::NativeContainer,
        )
        .await
        .expect("list running workspace containers");
        assert_eq!(names, vec!["ctx-harness-one", "ctx-harness-two"]);
        let log = std::fs::read_to_string(&log_path).expect("read sandbox cli log");
        assert!(
            log.contains("container ls --format {{.Names}}"),
            "expected running-container probe in log:\n{log}"
        );
    }
}
