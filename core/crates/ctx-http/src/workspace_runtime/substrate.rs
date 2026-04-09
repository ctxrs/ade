use serde::{Deserialize, Serialize};
pub(crate) use ctx_sandbox_contract::UbuntuSandboxSubstrate;

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
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SubstrateStartupReason {
    RestoreFailed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SubstrateShutdownOutcome {
    Saved,
    ColdStop,
    ColdStopAfterSaveFailure,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SubstrateShutdownReason {
    SaveUnsupported,
    SaveFailed,
}
