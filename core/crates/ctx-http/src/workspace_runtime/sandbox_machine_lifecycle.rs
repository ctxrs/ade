use super::*;
use crate::settings::ContainerMountMode;
use ctx_harness_setup::{HarnessSetupLogLevel, HarnessSetupObserver, HarnessSetupPhase};
#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::collections::HashSet;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::sync::Arc;
use std::time::{Duration, Instant};
#[cfg(any(target_os = "macos", target_os = "windows"))]
use tokio::sync::Mutex;

mod inspection;
mod reclaim;

const SANDBOX_OP_TIMEOUT: Duration = Duration::from_secs(60);

#[async_trait::async_trait]
pub(crate) trait SandboxMachineLifecycleExt {
    async fn ensure_sandbox_machine_download(&self) -> Result<()>;
    async fn inspect_sandbox_machine_memory_mb(&self, machine_name: &str) -> Result<Option<u32>>;
    async fn inspect_sandbox_machine_state(&self, machine_name: &str) -> Result<Option<String>>;
    async fn init_sandbox_machine_locked(
        &self,
        machine_name: &str,
        desired_memory_mb: u32,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<()>;
    async fn stop_sandbox_machine_locked(
        &self,
        machine_name: &str,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<bool>;
    async fn remove_sandbox_machine_locked(
        &self,
        machine_name: &str,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<()>;
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    async fn ensure_sandbox_machine_materialized(
        &self,
        settings: &ContainerExecutionSettings,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<()>;
    #[allow(dead_code)]
    async fn reconcile_running_sandbox_machine_memory(
        &self,
        settings: &ContainerExecutionSettings,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<()>;
    async fn has_running_workspace_containers(&self) -> Result<bool>;
    async fn should_defer_disk_isolated_machine_reconfiguration(
        &self,
        settings: &ContainerExecutionSettings,
        machine_name: &str,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<bool>;
    async fn disk_isolated_workspace_volumes_exist(
        &self,
        machine_name: &str,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<bool>;
    async fn ensure_engine_ready_for_disk_state_inspection(
        &self,
        machine_name: &str,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<bool>;
    async fn has_running_workspace_containers_for_stopped_machine_reconfiguration(
        &self,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<bool>;
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    async fn maybe_reclaim_sandbox_machine(
        &self,
        settings: &ContainerExecutionSettings,
        system: &SystemSnapshot,
        observer: Option<&dyn HarnessSetupObserver>,
        stores: &StoreManager,
        running_sessions: &Arc<Mutex<HashSet<SessionId>>>,
        terminals: &TerminalManager,
    ) -> Result<bool>;
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    async fn should_defer_reclaim_for_active_container_runtime(
        &self,
        stores: &StoreManager,
        running_sessions: &Arc<Mutex<HashSet<SessionId>>>,
        terminals: &TerminalManager,
    ) -> bool;
}

#[async_trait::async_trait]
impl SandboxMachineLifecycleExt for HarnessRuntimeManager {
    async fn ensure_sandbox_machine_download(&self) -> Result<()> {
        if !sandbox_machine_required() {
            return Ok(());
        }
        ensure_managed_sandbox_cli_runtime(self.data_root(), None, None).await?;
        let machine_image = if cfg!(target_os = "macos") {
            Some(ensure_managed_sandbox_machine_cache(self.data_root(), None, None).await?)
        } else {
            None
        };
        let machine_name = sandbox_machine_name(self.data_root());
        let machine_lock = sandbox_machine_singleflight_lock(&machine_name);
        let _machine_guard = match machine_lock.try_lock() {
            Ok(guard) => guard,
            Err(_) => return Ok(()),
        };
        seed_shared_sandbox_machine_cache_best_effort(self.data_root(), None).await;
        if sandbox_machine_present(self.data_root(), &machine_name).await? {
            persist_sandbox_machine_cache_to_shared_best_effort(self.data_root(), None).await;
            return Ok(());
        }
        let init_outcome = run_sandbox_machine_init(
            self.data_root(),
            &machine_name,
            machine_image.as_deref(),
            Some(container_machine_memory_mb(
                &ContainerExecutionSettings::default(),
            )),
            None,
        )
        .await?;
        let output = init_outcome.output;
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let combined = format!("{stderr}\n{stdout}").trim().to_string();
        if init_outcome.continued_after_machine_present
            || output.status.success()
            || combined.to_ascii_lowercase().contains("already exists")
        {
            persist_sandbox_machine_cache_to_shared_best_effort(self.data_root(), None).await;
            return Ok(());
        }
        anyhow::bail!("sandbox machine init failed: {combined}");
    }

    async fn inspect_sandbox_machine_memory_mb(&self, machine_name: &str) -> Result<Option<u32>> {
        inspection::inspect_sandbox_machine_memory_mb(self.data_root(), machine_name).await
    }

    async fn inspect_sandbox_machine_state(&self, machine_name: &str) -> Result<Option<String>> {
        inspection::inspect_sandbox_machine_state(self.data_root(), machine_name).await
    }

    async fn init_sandbox_machine_locked(
        &self,
        machine_name: &str,
        desired_memory_mb: u32,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<()> {
        let machine_image = if cfg!(target_os = "macos") {
            Some(ensure_managed_sandbox_machine_cache(self.data_root(), observer, None).await?)
        } else {
            None
        };
        let init_outcome = run_sandbox_machine_init(
            self.data_root(),
            machine_name,
            machine_image.as_deref(),
            Some(desired_memory_mb),
            observer,
        )
        .await?;
        let output = init_outcome.output;
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let combined = format!("{stderr}\n{stdout}").trim().to_string();
        if init_outcome.continued_after_machine_present
            || output.status.success()
            || combined.to_ascii_lowercase().contains("already exists")
        {
            persist_sandbox_machine_cache_to_shared_best_effort(self.data_root(), observer).await;
            return Ok(());
        }
        anyhow::bail!("sandbox machine init failed: {combined}");
    }

    async fn stop_sandbox_machine_locked(
        &self,
        machine_name: &str,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<bool> {
        let mut cmd = sandbox_container_command(self.data_root())?;
        cmd.arg("machine").arg("stop").arg(machine_name);
        let output = command_output_with_timeout(cmd, SANDBOX_OP_TIMEOUT).await?;
        if output.status.success() {
            observe_log(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                HarnessSetupLogLevel::Info,
                "stopped local sandbox runtime",
            );
            return Ok(true);
        }
        let combined = command_output_message(&output);
        let combined_lc = combined.to_ascii_lowercase();
        if combined_lc.contains("already stopped")
            || combined_lc.contains("not running")
            || combined_lc.contains("no machine")
            || combined_lc.contains("does not exist")
        {
            return Ok(false);
        }
        anyhow::bail!("sandbox machine stop failed: {combined}");
    }

    async fn remove_sandbox_machine_locked(
        &self,
        machine_name: &str,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<()> {
        let _ = self
            .stop_sandbox_machine_locked(machine_name, observer)
            .await;
        let mut cmd = sandbox_container_command(self.data_root())?;
        cmd.arg("machine").arg("rm").arg("-f").arg(machine_name);
        let output = command_output_with_timeout(cmd, SANDBOX_OP_TIMEOUT).await?;
        if output.status.success() {
            observe_log(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                HarnessSetupLogLevel::Info,
                "removed local sandbox runtime for reconfiguration",
            );
            return Ok(());
        }
        let combined = command_output_message(&output);
        let combined_lc = combined.to_ascii_lowercase();
        if combined_lc.contains("does not exist") || combined_lc.contains("no machine") {
            return Ok(());
        }
        anyhow::bail!("sandbox machine rm -f failed: {combined}");
    }

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    async fn ensure_sandbox_machine_materialized(
        &self,
        settings: &ContainerExecutionSettings,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<()> {
        if !sandbox_machine_required() {
            return Ok(());
        }
        let desired_memory_mb = container_machine_memory_mb(settings);
        let machine_name = sandbox_machine_name(self.data_root());
        let machine_lock = sandbox_machine_singleflight_lock(&machine_name);
        let _machine_guard = machine_lock.lock().await;
        seed_shared_sandbox_machine_cache_best_effort(self.data_root(), observer).await;

        let present = sandbox_machine_present(self.data_root(), &machine_name).await?;
        if present {
            let actual_memory_mb = self
                .inspect_sandbox_machine_memory_mb(&machine_name)
                .await?;
            if actual_memory_mb == Some(desired_memory_mb) {
                return Ok(());
            }
            if self
                .should_defer_disk_isolated_machine_reconfiguration(
                    settings,
                    &machine_name,
                    observer,
                )
                .await?
            {
                observe_log(
                    observer,
                    HarnessSetupPhase::MachineStartOrInit,
                    HarnessSetupLogLevel::Warn,
                    "deferring local sandbox runtime memory reconfiguration because disk-isolated workspace volumes would be destroyed by machine recreation",
                );
                return Ok(());
            }
            if self
                .has_running_workspace_containers_for_stopped_machine_reconfiguration(observer)
                .await?
            {
                observe_log(
                    observer,
                    HarnessSetupPhase::MachineStartOrInit,
                    HarnessSetupLogLevel::Warn,
                    "deferring local sandbox runtime memory reconfiguration until active workspace containers stop",
                );
                return Ok(());
            }
            let detail = actual_memory_mb
                .map(|value| format!("{value} MiB"))
                .unwrap_or_else(|| "unknown".to_string());
            observe_log(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                HarnessSetupLogLevel::Info,
                &format!(
                    "reconfiguring local sandbox runtime memory from {detail} to {desired_memory_mb} MiB"
                ),
            );
            self.remove_sandbox_machine_locked(&machine_name, observer)
                .await?;
        }

        self.init_sandbox_machine_locked(&machine_name, desired_memory_mb, observer)
            .await
    }

    #[allow(dead_code)]
    async fn reconcile_running_sandbox_machine_memory(
        &self,
        settings: &ContainerExecutionSettings,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<()> {
        if !sandbox_machine_required() {
            return Ok(());
        }
        let desired_memory_mb = container_machine_memory_mb(settings);
        let machine_name = sandbox_machine_name(self.data_root());
        let machine_lock = sandbox_machine_singleflight_lock(&machine_name);
        let _machine_guard = machine_lock.lock().await;

        if !sandbox_machine_present(self.data_root(), &machine_name).await? {
            observe_log(
                observer,
                HarnessSetupPhase::MachineCheck,
                HarnessSetupLogLevel::Warn,
                "local sandbox runtime is reachable but machine state could not be inspected; leaving memory profile unchanged",
            );
            return Ok(());
        }

        let actual_memory_mb = self
            .inspect_sandbox_machine_memory_mb(&machine_name)
            .await?;
        if actual_memory_mb == Some(desired_memory_mb) {
            return Ok(());
        }
        if self
            .should_defer_disk_isolated_machine_reconfiguration(settings, &machine_name, observer)
            .await?
        {
            observe_log(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                HarnessSetupLogLevel::Warn,
                "deferring local sandbox runtime memory reconfiguration because disk-isolated workspace volumes would be destroyed by machine recreation",
            );
            return Ok(());
        }
        if self
            .has_running_workspace_containers_for_stopped_machine_reconfiguration(observer)
            .await?
        {
            observe_log(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                HarnessSetupLogLevel::Warn,
                "deferring local sandbox runtime memory reconfiguration until active workspace containers stop",
            );
            return Ok(());
        }
        let detail = actual_memory_mb
            .map(|value| format!("{value} MiB"))
            .unwrap_or_else(|| "unknown".to_string());
        observe_log(
            observer,
            HarnessSetupPhase::MachineStartOrInit,
            HarnessSetupLogLevel::Info,
            &format!(
                "reconfiguring local sandbox runtime memory from {detail} to {desired_memory_mb} MiB"
            ),
        );
        self.remove_sandbox_machine_locked(&machine_name, observer)
            .await?;
        self.init_sandbox_machine_locked(&machine_name, desired_memory_mb, observer)
            .await
    }

    async fn has_running_workspace_containers(&self) -> Result<bool> {
        let mut cmd = sandbox_container_command(self.data_root())?;
        cmd.arg("ps").arg("--format").arg("{{.Names}}");
        let output = command_output_with_timeout(cmd, SANDBOX_OP_TIMEOUT).await?;
        if !output.status.success() {
            anyhow::bail!("sandbox CLI ps failed: {}", command_output_message(&output));
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(stdout
            .lines()
            .map(str::trim)
            .any(|name| name.starts_with("ctx-harness-")))
    }

    async fn should_defer_disk_isolated_machine_reconfiguration(
        &self,
        settings: &ContainerExecutionSettings,
        machine_name: &str,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<bool> {
        if !matches!(settings.mount_mode, ContainerMountMode::DiskIsolated) {
            return Ok(false);
        }
        self.disk_isolated_workspace_volumes_exist(machine_name, observer)
            .await
    }

    async fn disk_isolated_workspace_volumes_exist(
        &self,
        machine_name: &str,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<bool> {
        if !self
            .ensure_engine_ready_for_disk_state_inspection(machine_name, observer)
            .await?
        {
            observe_log(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                HarnessSetupLogLevel::Warn,
                "unable to verify disk-isolated workspace volumes before memory reconfiguration; leaving local sandbox runtime unchanged",
            );
            return Ok(true);
        }

        let mut cmd = sandbox_container_command(self.data_root())?;
        cmd.arg("volume").arg("ls").arg("--format").arg("{{.Name}}");
        let output = match command_output_with_timeout(cmd, SANDBOX_OP_TIMEOUT).await {
            Ok(output) => output,
            Err(err) => {
                observe_log(
                    observer,
                    HarnessSetupPhase::MachineStartOrInit,
                    HarnessSetupLogLevel::Warn,
                    &format!(
                        "failed to inspect disk-isolated workspace volumes before memory reconfiguration: {err}"
                    ),
                );
                return Ok(true);
            }
        };
        if !output.status.success() {
            let detail = command_output_message(&output);
            let suffix = if detail.is_empty() {
                String::new()
            } else {
                format!(": {detail}")
            };
            observe_log(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                HarnessSetupLogLevel::Warn,
                &format!(
                    "unable to inspect disk-isolated workspace volumes before memory reconfiguration{suffix}"
                ),
            );
            return Ok(true);
        }

        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::trim)
            .any(|name| name.starts_with("ctx-ws-")))
    }

    async fn ensure_engine_ready_for_disk_state_inspection(
        &self,
        machine_name: &str,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<bool> {
        if sandbox_engine_ready(self.data_root()).await? {
            return Ok(true);
        }

        observe_log(
            observer,
            HarnessSetupPhase::MachineStartOrInit,
            HarnessSetupLogLevel::Info,
            "starting local sandbox runtime to inspect disk-isolated workspace volumes before memory reconfiguration",
        );
        let mut start = sandbox_container_command(self.data_root())?;
        start.arg("machine").arg("start").arg(machine_name);
        let output = command_output_with_timeout(start, SANDBOX_MACHINE_START_TIMEOUT).await?;
        if !output.status.success() {
            return Ok(false);
        }

        let deadline = Instant::now() + sandbox_machine_ready_timeout();
        loop {
            if sandbox_engine_ready(self.data_root()).await? {
                return Ok(true);
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
            tokio::time::sleep(sandbox_machine_ready_poll_interval()).await;
        }
    }

    async fn has_running_workspace_containers_for_stopped_machine_reconfiguration(
        &self,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<bool> {
        match self.has_running_workspace_containers().await {
            Ok(has_running) => Ok(has_running),
            Err(err) => {
                if sandbox_engine_ready(self.data_root())
                    .await
                    .unwrap_or(false)
                {
                    return Err(err);
                }
                let machine_name = sandbox_machine_name(self.data_root());
                match self.inspect_sandbox_machine_state(&machine_name).await? {
                    Some(state) if state.contains("running") || state.contains("starting") => {
                        observe_log(
                            observer,
                            HarnessSetupPhase::MachineStartOrInit,
                            HarnessSetupLogLevel::Warn,
                            "local sandbox runtime appears to be running but unreachable; deferring memory reconfiguration until workload probes recover",
                        );
                        tracing::debug!(
                            "treating workspace container probe failure as busy because the local sandbox runtime still reports a running state: {err:#}"
                        );
                        return Ok(true);
                    }
                    Some(_) => {}
                    None => {
                        observe_log(
                            observer,
                            HarnessSetupPhase::MachineStartOrInit,
                            HarnessSetupLogLevel::Warn,
                            "local sandbox runtime state is unknown while workload probes are unreachable; deferring memory reconfiguration until the runtime can be inspected safely",
                        );
                        tracing::debug!(
                            "treating workspace container probe failure as busy because the local sandbox runtime state is unknown: {err:#}"
                        );
                        return Ok(true);
                    }
                }
                observe_log(
                    observer,
                    HarnessSetupPhase::MachineStartOrInit,
                    HarnessSetupLogLevel::Info,
                    "local sandbox runtime is not reachable; continuing memory reconfiguration without workload probe",
                );
                tracing::debug!(
                    "treating workspace container probe failure as idle because the sandbox runtime is not reachable: {err:#}"
                );
                Ok(false)
            }
        }
    }

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    async fn maybe_reclaim_sandbox_machine(
        &self,
        settings: &ContainerExecutionSettings,
        system: &SystemSnapshot,
        observer: Option<&dyn HarnessSetupObserver>,
        stores: &StoreManager,
        running_sessions: &Arc<Mutex<HashSet<SessionId>>>,
        terminals: &TerminalManager,
    ) -> Result<bool> {
        reclaim::maybe_reclaim_sandbox_machine(
            self,
            settings,
            system,
            observer,
            stores,
            running_sessions,
            terminals,
        )
        .await
    }

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    async fn should_defer_reclaim_for_active_container_runtime(
        &self,
        stores: &StoreManager,
        running_sessions: &Arc<Mutex<HashSet<SessionId>>>,
        terminals: &TerminalManager,
    ) -> bool {
        reclaim::should_defer_reclaim_for_active_container_runtime(
            stores,
            running_sessions,
            terminals,
        )
        .await
    }
}
