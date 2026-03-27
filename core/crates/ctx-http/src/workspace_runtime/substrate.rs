use anyhow::{anyhow, Result};
use ctx_core::models::{SandboxBinding, SandboxGuestIdentity, SandboxSubstrate};
use serde::{Deserialize, Serialize};

use crate::settings::ContainerRuntimeKind;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SubstrateStartupSelection {
    Reuse,
    Restore,
    ColdBoot,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SubstrateStartupOutcome {
    Reuse,
    Restore,
    ColdBoot,
    ColdBootAfterRestoreFailure,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SubstrateShutdownOutcome {
    Saved,
    ColdStop,
    ColdStopAfterSaveFailure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UbuntuSandboxSubstrate {
    pub(crate) substrate: SandboxSubstrate,
    pub(crate) guest_identity: SandboxGuestIdentity,
}

impl UbuntuSandboxSubstrate {
    pub(crate) fn from_runtime_kind(runtime: ContainerRuntimeKind) -> Self {
        let substrate = match runtime {
            ContainerRuntimeKind::NativeContainer => SandboxSubstrate::NativeContainer,
            ContainerRuntimeKind::SharedVmContainer => SandboxSubstrate::SharedVmContainer,
        };
        Self {
            substrate,
            guest_identity: SandboxGuestIdentity::linux_container_ubuntu(),
        }
    }

    pub(crate) fn from_binding(binding: &SandboxBinding) -> Result<Self> {
        let substrate = Self {
            substrate: binding.substrate,
            guest_identity: binding.guest_identity,
        };
        substrate.ensure_enabled()?;
        Ok(substrate)
    }

    pub(crate) fn runtime_kind(self) -> ContainerRuntimeKind {
        match self.substrate {
            SandboxSubstrate::NativeContainer => ContainerRuntimeKind::NativeContainer,
            SandboxSubstrate::SharedVmContainer => ContainerRuntimeKind::SharedVmContainer,
        }
    }

    pub(crate) fn runtime_kind_env_value(self) -> &'static str {
        match self.substrate {
            SandboxSubstrate::NativeContainer => "native_container",
            SandboxSubstrate::SharedVmContainer => "shared_vm_container",
        }
    }

    pub(crate) fn is_shared_vm_backed(self) -> bool {
        matches!(self.substrate, SandboxSubstrate::SharedVmContainer)
    }

    pub(crate) fn ensure_enabled(self) -> Result<()> {
        if self.guest_identity != SandboxGuestIdentity::linux_container_ubuntu() {
            return Err(anyhow!(
                "sandbox guest identity {} is not enabled; only linux + container + ubuntu is currently supported",
                guest_identity_label(self.guest_identity)
            ));
        }
        Ok(())
    }

    pub(crate) fn launch_ready_gap_message(
        self,
        runtime_target: &str,
        substrate_ready: bool,
        image_ready: bool,
    ) -> String {
        match self.substrate {
            SandboxSubstrate::NativeContainer => {
                if !substrate_ready {
                    format!(
                        "runtime prewarm completed but local sandbox runtime is not launch-ready for '{runtime_target}'"
                    )
                } else if !image_ready {
                    format!(
                        "runtime prewarm completed but launch image for '{runtime_target}' is not present in the local sandbox runtime"
                    )
                } else {
                    format!(
                        "runtime prewarm completed but runtime target '{runtime_target}' is not launch-ready"
                    )
                }
            }
            SandboxSubstrate::SharedVmContainer => {
                if !substrate_ready {
                    format!(
                        "runtime prewarm completed but shared VM substrate for '{runtime_target}' is not launch-ready"
                    )
                } else if !image_ready {
                    format!(
                        "runtime prewarm completed but launch image for '{runtime_target}' is not present in the shared VM runtime"
                    )
                } else {
                    format!(
                        "runtime prewarm completed but runtime target '{runtime_target}' is not launch-ready"
                    )
                }
            }
        }
    }

    pub(crate) fn launch_ready_detail_message(self) -> &'static str {
        match self.substrate {
            SandboxSubstrate::NativeContainer => {
                "local sandbox runtime and launch image are ready"
            }
            SandboxSubstrate::SharedVmContainer => {
                "shared VM substrate and launch image are ready"
            }
        }
    }

    pub(crate) fn runtime_prewarm_ready_message(self, launch_ready: bool) -> &'static str {
        if launch_ready {
            return self.launch_ready_detail_message();
        }

        match self.substrate {
            SandboxSubstrate::NativeContainer => {
                "local sandbox runtime and launch image are ready"
            }
            SandboxSubstrate::SharedVmContainer => {
                "shared VM runtime artifacts are ready; launch image loads when the shared VM starts"
            }
        }
    }

    pub(crate) fn workspace_launch_ready_message(self) -> &'static str {
        match self.substrate {
            SandboxSubstrate::NativeContainer => {
                "workspace sandbox is ready in the local sandbox runtime"
            }
            SandboxSubstrate::SharedVmContainer => {
                "workspace sandbox is ready on the shared VM substrate"
            }
        }
    }
}

pub(crate) fn guest_identity_label(identity: SandboxGuestIdentity) -> String {
    format!(
        "{:?} + {:?} + {:?}",
        identity.platform, identity.isolation_kind, identity.runtime
    )
    .to_ascii_lowercase()
}
