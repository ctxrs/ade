use std::path::Path;

use anyhow::{anyhow, bail, Result};
use ctx_core::ids::SandboxInstanceId;
use ctx_core::models::SandboxSubstrate;

use super::avf_linux_vm::{
    AvfLinuxSharedVmLifecycleState, AvfLinuxSharedVmStartOutcome, AvfLinuxSharedVmState,
    AvfLinuxSharedVmStopOutcome,
};
use super::{
    ContainerExecutionSettings, HarnessSetupObserver, SharedVmLifecycleOrchestrator,
    SubstrateShutdownOutcome, SubstrateStartupOutcome, UbuntuSandboxSubstrate,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SubstrateLifecycleRecord {
    pub(crate) substrate: SandboxSubstrate,
    pub(crate) startup_outcome: Option<SubstrateStartupOutcome>,
    pub(crate) shutdown_outcome: Option<SubstrateShutdownOutcome>,
    pub(crate) simulated: bool,
}

pub(crate) struct SharedSubstrateLifecycleManager<'a> {
    data_root: &'a Path,
}

impl<'a> SharedSubstrateLifecycleManager<'a> {
    pub(crate) fn new(data_root: &'a Path) -> Self {
        Self { data_root }
    }

    pub(crate) async fn ensure_shared_runtime_ready(
        &self,
        settings: &ContainerExecutionSettings,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<SubstrateLifecycleRecord> {
        let substrate = UbuntuSandboxSubstrate::from_runtime_kind(settings.runtime);
        substrate.ensure_enabled()?;
        if !substrate.is_shared_vm_backed() {
            bail!(
                "shared substrate lifecycle manager only supports the shared VM container runtime"
            );
        }

        let sandbox_instance_id = SandboxInstanceId(uuid::Uuid::nil());
        let orchestrator = SharedVmLifecycleOrchestrator::new(self.data_root);
        let state = orchestrator.workspace_runtime_state(sandbox_instance_id)?;
        if matches!(state.state, AvfLinuxSharedVmLifecycleState::Running) {
            return Ok(SubstrateLifecycleRecord {
                substrate: substrate.substrate,
                startup_outcome: Some(SubstrateStartupOutcome::Reuse),
                shutdown_outcome: map_shutdown_outcome(state.last_stop_outcome),
                simulated: state.simulated,
            });
        }

        let started = orchestrator
            .ensure_shared_runtime_ready(settings, observer)
            .await?;
        Ok(SubstrateLifecycleRecord {
            substrate: substrate.substrate,
            startup_outcome: Some(startup_outcome_from_state(&started)?),
            shutdown_outcome: map_shutdown_outcome(started.last_stop_outcome),
            simulated: started.simulated,
        })
    }

    pub(crate) async fn ensure_workspace_runtime_ready(
        &self,
        sandbox_instance_id: SandboxInstanceId,
        settings: &ContainerExecutionSettings,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<SubstrateLifecycleRecord> {
        self.ensure_shared_vm_runtime_ready(sandbox_instance_id, settings, observer)
            .await
    }

    pub(crate) async fn save_or_stop_shared_runtime(
        &self,
        settings: &ContainerExecutionSettings,
    ) -> Result<SubstrateLifecycleRecord> {
        let substrate = UbuntuSandboxSubstrate::from_runtime_kind(settings.runtime);
        substrate.ensure_enabled()?;
        if !substrate.is_shared_vm_backed() {
            bail!(
                "shared substrate lifecycle manager only supports the shared VM container runtime"
            );
        }

        let sandbox_instance_id = SandboxInstanceId(uuid::Uuid::nil());
        let orchestrator = SharedVmLifecycleOrchestrator::new(self.data_root);
        let state = orchestrator.workspace_runtime_state(sandbox_instance_id)?;
        if matches!(
            state.state,
            AvfLinuxSharedVmLifecycleState::Missing | AvfLinuxSharedVmLifecycleState::Stopped
        ) {
            return Ok(SubstrateLifecycleRecord {
                substrate: substrate.substrate,
                startup_outcome: None,
                shutdown_outcome: map_shutdown_outcome(state.last_stop_outcome),
                simulated: state.simulated,
            });
        }

        let stopped = orchestrator.save_or_stop_shared_runtime()?;
        Ok(SubstrateLifecycleRecord {
            substrate: substrate.substrate,
            startup_outcome: None,
            shutdown_outcome: map_shutdown_outcome(stopped.last_stop_outcome),
            simulated: stopped.simulated,
        })
    }

    async fn ensure_shared_vm_runtime_ready(
        &self,
        sandbox_instance_id: SandboxInstanceId,
        settings: &ContainerExecutionSettings,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<SubstrateLifecycleRecord> {
        let substrate = UbuntuSandboxSubstrate::from_runtime_kind(settings.runtime);
        substrate.ensure_enabled()?;
        if !substrate.is_shared_vm_backed() {
            bail!(
                "shared substrate lifecycle manager only supports the shared VM container runtime"
            );
        }

        let orchestrator = SharedVmLifecycleOrchestrator::new(self.data_root);
        let state = orchestrator.workspace_runtime_state(sandbox_instance_id)?;
        if matches!(state.state, AvfLinuxSharedVmLifecycleState::Running) {
            return Ok(SubstrateLifecycleRecord {
                substrate: substrate.substrate,
                startup_outcome: Some(SubstrateStartupOutcome::Reuse),
                shutdown_outcome: map_shutdown_outcome(state.last_stop_outcome),
                simulated: state.simulated,
            });
        }

        let started = orchestrator
            .ensure_workspace_runtime_ready(sandbox_instance_id, settings, observer)
            .await?;
        Ok(SubstrateLifecycleRecord {
            substrate: substrate.substrate,
            startup_outcome: Some(startup_outcome_from_state(&started)?),
            shutdown_outcome: map_shutdown_outcome(started.last_stop_outcome),
            simulated: started.simulated,
        })
    }
}

fn startup_outcome_from_state(state: &AvfLinuxSharedVmState) -> Result<SubstrateStartupOutcome> {
    map_startup_outcome(state.last_start_outcome).ok_or_else(|| {
        anyhow!(
            "shared VM substrate reached {:?} without reporting a startup outcome",
            state.state
        )
    })
}

fn map_startup_outcome(
    outcome: Option<AvfLinuxSharedVmStartOutcome>,
) -> Option<SubstrateStartupOutcome> {
    match outcome? {
        AvfLinuxSharedVmStartOutcome::AlreadyRunning => Some(SubstrateStartupOutcome::Reuse),
        AvfLinuxSharedVmStartOutcome::Restored => Some(SubstrateStartupOutcome::Restore),
        AvfLinuxSharedVmStartOutcome::ColdBoot => Some(SubstrateStartupOutcome::ColdBoot),
        AvfLinuxSharedVmStartOutcome::ColdBootAfterRestoreFailure => {
            Some(SubstrateStartupOutcome::ColdBootAfterRestoreFailure)
        }
    }
}

fn map_shutdown_outcome(
    outcome: Option<AvfLinuxSharedVmStopOutcome>,
) -> Option<SubstrateShutdownOutcome> {
    match outcome? {
        AvfLinuxSharedVmStopOutcome::SavedStateWritten => Some(SubstrateShutdownOutcome::Saved),
        AvfLinuxSharedVmStopOutcome::ColdStop
        | AvfLinuxSharedVmStopOutcome::ColdStopSaveUnsupported => {
            Some(SubstrateShutdownOutcome::ColdStop)
        }
        AvfLinuxSharedVmStopOutcome::ColdStopAfterSaveFailure => {
            Some(SubstrateShutdownOutcome::ColdStopAfterSaveFailure)
        }
    }
}
